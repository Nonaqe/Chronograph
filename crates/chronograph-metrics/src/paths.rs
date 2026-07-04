//! Резолв переименований: отображение путь → канонический (последний) путь.
//!
//! Используется и churn, и complexity — чтобы история/состояние файла не
//! фрагментировались при rename'ах (принцип 2.5 ТЗ). Материализуется во временную
//! таблицу `path_map` для JOIN'ов в аналитическом SQL.

use std::collections::{HashMap, HashSet};

use chronograph_core::Result;
use duckdb::{params, Connection};

use crate::store_err as se;

/// Построить отображение «любой встречавшийся путь → его канонический путь».
pub(crate) fn build_canonical_map(conn: &Connection) -> Result<Vec<(String, String)>> {
    // Рёбра rename: old_path → path. При повторном использовании имени берём
    // самое позднее переименование (по committed_at) — детерминированно.
    let mut next: HashMap<String, (String, i64)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT fc.old_path, fc.path, CAST(epoch(c.committed_at) AS BIGINT) AS ts
                 FROM file_changes fc
                 JOIN commits c ON fc.sha = c.sha
                 WHERE fc.change_type = 'R' AND fc.old_path IS NOT NULL",
            )
            .map_err(se)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(se)?;
        for row in rows {
            let (old, new, ts) = row.map_err(se)?;
            match next.get(&old) {
                Some((_, prev_ts)) if *prev_ts >= ts => {}
                _ => {
                    next.insert(old, (new, ts));
                }
            }
        }
    }

    let mut paths: HashSet<String> = HashSet::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT path FROM file_changes
                 UNION
                 SELECT old_path FROM file_changes WHERE old_path IS NOT NULL",
            )
            .map_err(se)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(se)?;
        for row in rows {
            paths.insert(row.map_err(se)?);
        }
    }

    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let canonical = resolve_canonical(&p, &next);
        out.push((p, canonical));
    }
    out.sort();
    Ok(out)
}

/// Пройти цепочку переименований вперёд до конечного имени, со страховкой от цикла.
fn resolve_canonical(start: &str, next: &HashMap<String, (String, i64)>) -> String {
    let mut cur = start.to_string();
    let mut seen = HashSet::new();
    seen.insert(cur.clone());
    while let Some((n, _)) = next.get(&cur) {
        if seen.contains(n) {
            break;
        }
        seen.insert(n.clone());
        cur = n.clone();
    }
    cur
}

/// Мета живого файла на HEAD — для blame-кэша и планирования очереди blame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LivingFile {
    /// Канонический путь.
    pub path: String,
    /// SHA коммита, ПОСЛЕДНИМ изменившего файл — ключ инвалидации blame-кэша:
    /// файл не менялся → его blame на новом HEAD идентичен прошлому.
    pub last_sha: String,
    /// Число ревизий файла в истории — прокси стоимости blame (стоимость ≈
    /// размер × глубина истории) для largest-first планирования.
    pub revisions: u64,
    /// Суммарно добавлено строк за историю — вместе с ревизиями даёт прокси
    /// стоимости blame (`cost = revisions × total_added`) для blame-бюджета.
    pub total_added: u64,
}

impl LivingFile {
    /// Прокси стоимости blame файла: `revisions × total_added`.
    ///
    /// Blame ≈ O(размер × глубина истории); для append-хвостатых файлов
    /// total_added ≈ размер. Данные-обоснование дефолта бюджета — см.
    /// [`crate::config::DEFAULT_BLAME_BUDGET`].
    pub fn blame_cost(&self) -> u64 {
        self.revisions.saturating_mul(self.total_added)
    }
}

/// Живые канонические файлы на HEAD (последнее по времени изменение ≠ удаление),
/// с метой: sha последнего изменения + число ревизий.
///
/// Материализует `path_map`; порядок — по пути. Механический фильтр НЕ применяется:
/// текущее состояние файла определяется фактически последним изменением. Общий для
/// knowledge и code age (и blame-кэша).
pub(crate) fn living_files_meta(conn: &Connection) -> Result<Vec<LivingFile>> {
    let canonical = build_canonical_map(conn)?;
    materialize_path_map(conn, &canonical)?;

    // Тай-брейк ПОЛНЫЙ (как в churn/complexity): при склейке имён несколько строк
    // одного коммита мапятся на один canonical — (ts, sha) равны; не-'D' выигрывает,
    // далее change_type и сырой путь (иначе живость/last_sha недетерминированы).
    let sql = "WITH mapped AS (
                   SELECT pm.canonical AS path,
                          fc.path AS raw_path,
                          fc.change_type AS change_type,
                          CAST(epoch(c.committed_at) AS BIGINT) AS ts,
                          fc.sha AS sha,
                          fc.added AS added
                   FROM file_changes fc
                   JOIN commits c ON fc.sha = c.sha
                   JOIN path_map pm ON fc.path = pm.path
               ),
               ranked AS (
                   SELECT path, change_type, sha,
                          row_number() OVER (PARTITION BY path
                              ORDER BY ts DESC, sha DESC,
                                       CASE WHEN change_type = 'D' THEN 1 ELSE 0 END,
                                       change_type, raw_path) AS rn,
                          count(*) OVER (PARTITION BY path) AS revisions,
                          sum(added) OVER (PARTITION BY path) AS total_added
                   FROM mapped
               )
               SELECT path, sha, CAST(revisions AS BIGINT),
                      CAST(COALESCE(total_added, 0) AS BIGINT)
               FROM ranked
               WHERE rn = 1 AND change_type <> 'D'
               ORDER BY path";
    let mut stmt = conn.prepare(sql).map_err(se)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LivingFile {
                path: r.get(0)?,
                last_sha: r.get(1)?,
                revisions: r.get::<_, i64>(2)? as u64,
                total_added: r.get::<_, i64>(3)? as u64,
            })
        })
        .map_err(se)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(se)?);
    }
    Ok(out)
}

/// Залить отображение path→canonical во временную таблицу `path_map`.
pub(crate) fn materialize_path_map(
    conn: &Connection,
    canonical: &[(String, String)],
) -> Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS path_map;
         CREATE TEMPORARY TABLE path_map (path TEXT, canonical TEXT);",
    )
    .map_err(se)?;
    let mut stmt = conn
        .prepare("INSERT INTO path_map (path, canonical) VALUES (?, ?)")
        .map_err(se)?;
    for (path, canon) in canonical {
        stmt.execute(params![path, canon]).map_err(se)?;
    }
    Ok(())
}
