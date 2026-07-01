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
