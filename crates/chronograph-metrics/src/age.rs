//! Code age / stability — возраст строк кода (§3.6 ТЗ).
//!
//! **Что считаем:** для каждого живого файла — распределение ВОЗРАСТА строк, где
//! возраст строки = `anchor − время последнего изменившего её коммита`. Распределение
//! описываем перцентилями: newest (min) / median (p50) / p90 / oldest (max) в днях.
//!
//! **Как:** построчный blame HEAD-версии файла (тот же трейт [`BlameSource`], что и
//! knowledge — gix здесь не фигурирует). Каждый участок blame даёт коммит; его время
//! берём из таблицы `commits`. `anchor = max(committed_at)` истории (детерминированно,
//! как окна churn — не wall-clock).
//!
//! **Зачем (§3.6):** стабильные старые куски vs зоны постоянного переписывания. Файл с
//! большим median возрастом — устойчивый; с маленьким — активно переписывается.
//!
//! **Детерминизм:** возраст — целые дни из UTC-таймстемпов; перцентиль — nearest-rank
//! по строкам (параметр-свободно); порядок результата по пути. Два прогона идентичны.
//!
//! **§3.6 не задаёт конкретный агрегат** («распределение возраста строк») — поэтому
//! храним перцентили (не выдумываем порог вида «% строк старше X дней»). Набор
//! перцентилей (p90) — согласованная точка, см. CONTEXT.md.
//!
//! **v1:** blame без rename-following (как knowledge); файлы, на которых blame упал
//! (паника gix-blame), считаются в [`AgeReport::blame_skipped`], не теряются молча.

use std::collections::HashMap;

use chronograph_core::{BlameSource, FileBlame, Result};
use chronograph_store::DuckStore;

use crate::store_err as se;

const SECONDS_PER_DAY: i64 = 86_400;

/// Возрастной профиль одного файла (дни).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAge {
    /// Канонический путь файла на HEAD.
    pub path: String,
    /// Сколько строк сблеймлено (вес распределения).
    pub lines: u32,
    /// Возраст самой свежей строки (min).
    pub newest_age_days: i64,
    /// Медианный возраст строки (p50, взвешенно по строкам).
    pub median_age_days: i64,
    /// Возраст «старого хвоста» (p90).
    pub p90_age_days: i64,
    /// Возраст самой древней строки (max).
    pub oldest_age_days: i64,
}

/// Итог code age: профили по файлам + счётчик пропущенных из-за сбоя blame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgeReport {
    /// По строке на файл (детерминированный порядок по пути).
    pub files: Vec<FileAge>,
    /// Живых файлов, пропущенных из-за паники blame (как в knowledge) — не молча.
    pub blame_skipped: usize,
}

/// Посчитать code age по всем живым файлам репозитория.
///
/// `blame_budget` — бюджет blame на файл (см. [`crate::config::DEFAULT_BLAME_BUDGET`]),
/// `0` — безлимит. Файлы дороже бюджета выпадают из age (учтены в `blame_skipped`).
pub fn compute_age(
    store: &DuckStore,
    blamer: &impl BlameSource,
    blame_budget: u64,
) -> Result<AgeReport> {
    let conn = store.conn();

    // HEAD последнего прогона — коммит, на который считаем blame.
    let head: Option<String> = conn
        .query_row("SELECT head_sha FROM analysis_meta LIMIT 1", [], |r| {
            r.get(0)
        })
        .ok();
    let Some(head) = head else {
        return Ok(AgeReport::default()); // анализ ещё не прогонялся
    };
    // Якорь = последняя активность истории (детерминизм, как окна churn — не wall-clock).
    let Some(anchor) = load_anchor(store)? else {
        return Ok(AgeReport::default()); // пустой репозиторий
    };

    // Карта коммит → время коммита (unix-секунды UTC).
    let commit_time = load_commit_times(store)?;

    let files = crate::paths::living_files_meta(conn)?;
    // Blame через инкрементальный кэш + largest-first + бюджет (см. blame_cache).
    let (blamed, _skips) =
        crate::blame_cache::cached_blame_many(store, blamer, &files, &head, blame_budget)?;
    let paths: Vec<String> = files.into_iter().map(|f| f.path).collect();

    Ok(from_blame(&commit_time, anchor, &paths, &blamed))
}

/// Агрегировать code age из ГОТОВОГО blame (без БД/gix).
///
/// Выделено, чтобы материализация блеймила ОДИН раз и питала knowledge+age из одного
/// прохода. `files[i]` соответствует `blamed[i]`. `anchor` — max(committed_at).
pub(crate) fn from_blame(
    commit_time: &HashMap<String, i64>,
    anchor: i64,
    files: &[String],
    blamed: &[FileBlame],
) -> AgeReport {
    let mut out = Vec::with_capacity(files.len());
    let mut blame_skipped = 0usize;
    for (path, fb) in files.iter().zip(blamed) {
        let hunks = match fb {
            FileBlame::Blamed(hunks) => hunks,
            FileBlame::Failed => {
                blame_skipped += 1;
                continue;
            }
        };

        // Пары (возраст_дни, строк) по участкам; коммит без времени в сторе — пропуск.
        let mut aged: Vec<(i64, u32)> = Vec::with_capacity(hunks.len());
        let mut total: u64 = 0;
        for h in hunks {
            let Some(&ct) = commit_time.get(&h.commit_sha) else {
                continue;
            };
            let age_days = (anchor - ct).max(0) / SECONDS_PER_DAY;
            aged.push((age_days, h.lines));
            total += h.lines as u64;
        }
        if total == 0 {
            continue; // пустой файл / все коммиты неизвестны
        }

        // Сортировка по возрасту возр. — для nearest-rank перцентилей.
        aged.sort_by_key(|a| a.0);

        out.push(FileAge {
            path: path.clone(),
            lines: total as u32,
            newest_age_days: aged.first().map(|p| p.0).unwrap_or(0),
            median_age_days: weighted_percentile(&aged, total, 0.5),
            p90_age_days: weighted_percentile(&aged, total, 0.9),
            oldest_age_days: aged.last().map(|p| p.0).unwrap_or(0),
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    AgeReport {
        files: out,
        blame_skipped,
    }
}

/// Якорь возраста = последняя активность истории (`max(committed_at)`, unix-сек UTC).
/// `None` — пустой репозиторий.
pub(crate) fn load_anchor(store: &DuckStore) -> Result<Option<i64>> {
    store
        .conn()
        .query_row(
            "SELECT CAST(epoch(max(committed_at)) AS BIGINT) FROM commits",
            [],
            |r| r.get(0),
        )
        .map_err(se)
}

/// Взвешенный перцентиль методом nearest-rank: наименьший возраст, чья накопленная
/// доля строк ≥ `p`. Параметр-свободно и детерминированно. `aged` отсортирован по
/// возрасту возр.
fn weighted_percentile(aged: &[(i64, u32)], total: u64, p: f64) -> i64 {
    // Ранг ≥ 1: даже при p=0 берём первый элемент.
    let target = ((p * total as f64).ceil() as u64).max(1);
    let mut cumulative: u64 = 0;
    for (age, lines) in aged {
        cumulative += *lines as u64;
        if cumulative >= target {
            return *age;
        }
    }
    aged.last().map(|p| p.0).unwrap_or(0)
}

/// Загрузить `commit.sha → committed_at` (unix-секунды UTC) из стора.
pub(crate) fn load_commit_times(store: &DuckStore) -> Result<HashMap<String, i64>> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare("SELECT sha, CAST(epoch(committed_at) AS BIGINT) FROM commits")
        .map_err(se)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(se)?;
    let mut map = HashMap::new();
    for r in rows {
        let (sha, ts) = r.map_err(se)?;
        map.insert(sha, ts);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_percentile_nearest_rank() {
        // Возрасты [0,10,10,10] (после сортировки), веса по 1: total 4.
        let aged = vec![(0, 1u32), (10, 1), (10, 1), (10, 1)];
        // p50: target=ceil(2)=2 → cum 1(<2),2(>=2)@age10 → 10.
        assert_eq!(weighted_percentile(&aged, 4, 0.5), 10);
        // p90: target=ceil(3.6)=4 → age10.
        assert_eq!(weighted_percentile(&aged, 4, 0.9), 10);
        // p0: target=1 → первый (age0).
        assert_eq!(weighted_percentile(&aged, 4, 0.0), 0);
    }

    #[test]
    fn weighted_percentile_respects_line_weight() {
        // Один свежий участок в 1 строку и один древний в 9 строк: медиана — древний.
        let aged = vec![(0, 1u32), (100, 9)];
        assert_eq!(weighted_percentile(&aged, 10, 0.5), 100);
        // Но newest (min) остаётся 0 — это отдельное поле, не перцентиль.
        assert_eq!(aged.first().unwrap().0, 0);
    }
}
