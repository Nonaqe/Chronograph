//! Инкрементальный blame-кэш + largest-first планирование очереди blame.
//!
//! Blame — самая дорогая операция движка. Два ускорения БЕЗ потери точности:
//!
//! 1. **Кэш** (`blame_cache`, служебная таблица — как `complexity_cache`, НЕ §7:
//!    кэш — деталь реализации, а не аналитический продукт для report/export).
//!    Ключ — `(канонический путь, sha последнего изменения, ЧИСЛО РЕВИЗИЙ)`: если
//!    в новых коммитах файл не менялся, его blame на новом HEAD идентичен прошлому.
//!    Число ревизий в ключе обязательно: при равных committed_at «sha последнего
//!    изменения» выбирается tie-break'ом и может НЕ измениться при реально
//!    изменившемся файле; ревизии же строго растут при любом изменении (модель
//!    append-only) — инвалидация монотонна. Повторные прогоны переблеймливают
//!    только изменившиеся файлы. Результат `Failed` (паника gix-blame) тоже
//!    кэшируется — паника детерминирована, нет смысла заново жевать падающий файл.
//!
//! 2. **Largest-first**: непокрытые кэшем файлы уходят в параллельный blame в
//!    порядке убывания стоимости (прокси — число ревизий): гиганты стартуют
//!    первыми и параллельно, мелочь равномерно добивает хвост. Иначе гигант,
//!    доставшийся потоку последним, сериализует хвост при 15 простаивающих ядрах.
//!    На результат не влияет вообще — только на расписание; соответствие
//!    файл→результат восстанавливается по индексам.

use std::collections::HashMap;

use chronograph_core::{BlameHunk, BlameSource, FileBlame, Result};
use chronograph_store::DuckStore;
use duckdb::params;

use crate::paths::LivingFile;
use crate::store_err as se;

/// Причина, по которой файл выпал из knowledge/age (не молча — показывается
/// поимённо в отчёте и CLI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// blame упал (паника внутри gix-blame — upstream-баг).
    Failed,
    /// Файл дороже blame-бюджета: `cost = revisions × total_added`.
    OverBudget {
        /// Прокси стоимости файла (строко-ревизии).
        cost: u64,
        /// Действовавший бюджет.
        budget: u64,
    },
}

/// Пропущенный из blame файл + причина.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameSkip {
    /// Канонический путь.
    pub path: String,
    /// Почему пропущен.
    pub reason: SkipReason,
}

/// Blame файлов с кэшем, largest-first планированием и бюджетом.
///
/// Возвращает `(результаты в порядке files, пропуски с причинами)`. Файлы дороже
/// `budget` (0 = безлимит) НЕ блеймятся: в результатах — `Failed` (выпадают из
/// метрик, как при сбое), в пропусках — `OverBudget` с числами. Решение по бюджету
/// НЕ кэшируется — пересматривается каждый прогон (поднял бюджет → посчитается).
/// Кэш читается по `(path, last_sha, revisions)`; промахи блеймятся (гиганты
/// первыми) и записываются в кэш. Точность непропущенных идентична прямому blame.
pub(crate) fn cached_blame_many(
    store: &DuckStore,
    blamer: &impl BlameSource,
    files: &[LivingFile],
    at_commit: &str,
    budget: u64,
) -> Result<(Vec<FileBlame>, Vec<BlameSkip>)> {
    let conn = store.conn();
    ensure_cache_table(conn)?;

    // 1. Чтение кэша: какие (path, last_sha, revisions) уже посчитаны.
    let cached = load_cache(store, files)?;

    // 2. Промахи. Файлы дороже бюджета отсеиваются ДО blame (с причиной), остальные
    //    уходят в параллельный blame ГИГАНТАМИ ПЕРВЫМИ (largest-first).
    let mut skips: Vec<BlameSkip> = Vec::new();
    let mut over_budget: Vec<usize> = Vec::new();
    let mut misses: Vec<usize> = Vec::new();
    for (i, f) in files.iter().enumerate() {
        if cached.contains_key(f.path.as_str()) {
            continue;
        }
        let cost = f.blame_cost();
        if budget > 0 && cost > budget {
            over_budget.push(i);
            skips.push(BlameSkip {
                path: f.path.clone(),
                reason: SkipReason::OverBudget { cost, budget },
            });
        } else {
            misses.push(i);
        }
    }
    misses.sort_by(|&a, &b| {
        files[b]
            .blame_cost()
            .cmp(&files[a].blame_cost())
            .then_with(|| files[a].path.cmp(&files[b].path))
    });
    let miss_paths: Vec<String> = misses.iter().map(|&i| files[i].path.clone()).collect();
    let blamed = blamer.blame_many(&miss_paths, at_commit)?;

    // 3. Свежие результаты — в кэш (over-budget НЕ кэшируется); упавшие — в причины.
    store_cache(store, &misses, files, &blamed)?;
    for (&i, fb) in misses.iter().zip(&blamed) {
        if matches!(fb, FileBlame::Failed) {
            skips.push(BlameSkip {
                path: files[i].path.clone(),
                reason: SkipReason::Failed,
            });
        }
    }
    // Упавшие ИЗ КЭША тоже причины (закэшированный Failed прошлого прогона).
    for f in files {
        if matches!(cached.get(f.path.as_str()), Some(FileBlame::Failed)) {
            skips.push(BlameSkip {
                path: f.path.clone(),
                reason: SkipReason::Failed,
            });
        }
    }
    skips.sort_by(|a, b| a.path.cmp(&b.path));

    // 4. Сборка результата в исходном порядке `files`.
    let mut by_index: HashMap<usize, FileBlame> = misses.into_iter().zip(blamed).collect();
    for i in over_budget {
        by_index.insert(i, FileBlame::Failed);
    }
    let mut out = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        match by_index.remove(&i) {
            Some(fb) => out.push(fb),
            None => out.push(cached.get(f.path.as_str()).cloned().unwrap_or_else(|| {
                // Недостижимо: файл либо в cached, либо в misses/over_budget. Пустой
                // blame — безопасный фолбэк на случай нарушения инварианта.
                FileBlame::Blamed(Vec::new())
            })),
        }
    }
    Ok((out, skips))
}

/// Служебная таблица кэша (не §7): по строке на hunk; спец-строки — маркеры
/// «пустой blame» (`commit_sha NULL, failed=false`) и «blame упал» (`failed=true`),
/// чтобы отличать «посчитано и пусто/упало» от «нет в кэше».
fn ensure_cache_table(conn: &duckdb::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS blame_cache (
             path       TEXT,
             last_sha   TEXT,
             revisions  BIGINT,
             failed     BOOLEAN,
             commit_sha TEXT,
             lines      INTEGER
         );",
    )
    .map_err(se)?;
    Ok(())
}

/// Прочитать кэш для актуальных `(path, last_sha)` пар.
fn load_cache<'a>(
    store: &DuckStore,
    files: &'a [LivingFile],
) -> Result<HashMap<&'a str, FileBlame>> {
    let conn = store.conn();
    // Актуальные ключи — во временную таблицу, JOIN отсекает устаревшие записи.
    conn.execute_batch(
        "DROP TABLE IF EXISTS blame_keys;
         CREATE TEMPORARY TABLE blame_keys (path TEXT, last_sha TEXT, revisions BIGINT);",
    )
    .map_err(se)?;
    {
        let mut app = conn.appender("blame_keys").map_err(se)?;
        for f in files {
            app.append_row(params![f.path, f.last_sha, f.revisions as i64])
                .map_err(se)?;
        }
        app.flush().map_err(se)?;
    }

    let mut stmt = conn
        .prepare(
            "SELECT bc.path, bc.failed, bc.commit_sha, bc.lines
             FROM blame_cache bc
             JOIN blame_keys k ON bc.path = k.path AND bc.last_sha = k.last_sha                   AND bc.revisions = k.revisions",
        )
        .map_err(se)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(se)?;

    let mut acc: HashMap<String, FileBlame> = HashMap::new();
    for row in rows {
        let (path, failed, commit_sha, lines) = row.map_err(se)?;
        let entry = acc
            .entry(path)
            .or_insert_with(|| FileBlame::Blamed(Vec::new()));
        if failed {
            *entry = FileBlame::Failed;
        } else if let (FileBlame::Blamed(hunks), Some(sha)) = (entry, commit_sha) {
            hunks.push(BlameHunk {
                commit_sha: sha,
                lines: lines as u32,
            });
        }
    }

    // Ключи — &str из files (стабильные), значения — собранные FileBlame.
    let mut out = HashMap::new();
    for f in files {
        if let Some(fb) = acc.remove(f.path.as_str()) {
            out.insert(f.path.as_str(), fb);
        }
    }
    Ok(out)
}

/// Записать свежие blame-результаты в кэш, заместив устаревшие записи этих путей.
fn store_cache(
    store: &DuckStore,
    misses: &[usize],
    files: &[LivingFile],
    blamed: &[FileBlame],
) -> Result<()> {
    if misses.is_empty() {
        return Ok(());
    }
    let conn = store.conn();
    conn.execute_batch("BEGIN TRANSACTION").map_err(se)?;
    {
        let mut del = conn
            .prepare("DELETE FROM blame_cache WHERE path = ?")
            .map_err(se)?;
        for &i in misses {
            del.execute(params![files[i].path]).map_err(se)?;
        }
    }
    {
        let mut app = conn.appender("blame_cache").map_err(se)?;
        for (&i, fb) in misses.iter().zip(blamed) {
            let f = &files[i];
            match fb {
                FileBlame::Failed => {
                    app.append_row(params![
                        f.path,
                        f.last_sha,
                        f.revisions as i64,
                        true,
                        None::<String>,
                        0_i64
                    ])
                    .map_err(se)?;
                }
                FileBlame::Blamed(hunks) if hunks.is_empty() => {
                    // Маркер «посчитано, пусто» — иначе неотличимо от промаха.
                    app.append_row(params![
                        f.path,
                        f.last_sha,
                        f.revisions as i64,
                        false,
                        None::<String>,
                        0_i64
                    ])
                    .map_err(se)?;
                }
                FileBlame::Blamed(hunks) => {
                    for h in hunks {
                        app.append_row(params![
                            f.path,
                            f.last_sha,
                            f.revisions as i64,
                            false,
                            h.commit_sha,
                            h.lines as i64
                        ])
                        .map_err(se)?;
                    }
                }
            }
        }
        app.flush().map_err(se)?;
    }
    conn.execute_batch("COMMIT").map_err(se)?;
    Ok(())
}
