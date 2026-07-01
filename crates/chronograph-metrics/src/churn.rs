//! Churn — изменчивость файлов.
//!
//! **Что считаем:** для каждого файла число коммитов, затронувших его, в
//! скользящих временных окнах (total / recent / mid / long), а также суммарные
//! добавленные/удалённые строки.
//!
//! **Как:** агрегация над сырыми таблицами `file_changes` + `commits` в DuckDB.
//! Переименования склеиваются: путь резолвится до «канонического» (последнего)
//! имени по цепочке rename'ов (`change_type='R'`, поле `old_path`), чтобы история
//! переименованного файла не фрагментировалась (ТЗ принцип 2.5).
//!
//! **Зачем:** часто меняющийся файл — потенциальная точка нестабильности. Сам по
//! себе слабый сигнал; ценен в связке с complexity (hotspot).
//!
//! **Нюансы (ТЗ 3.1):** окна отсчитываются от максимального `committed_at`
//! (детерминизм); «механические» коммиты опционально исключаются; «мёртвые»
//! (удалённые и не воскрешённые) файлы помечаются `is_alive=false`, чтобы не
//! висеть в hotspots.

use chronograph_core::Result;
use chronograph_store::DuckStore;

use crate::config::ChurnConfig;
use crate::store_err as se;

const SECONDS_PER_DAY: i64 = 86_400;

/// Churn одного (канонического) файла.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChurn {
    /// Канонический путь (после резолва переименований).
    pub path: String,
    /// Коммитов за всю историю.
    pub churn_total: u64,
    /// Коммитов за короткое окно (`window_recent_days`).
    pub churn_recent: u64,
    /// Коммитов за среднее окно (`window_mid_days`).
    pub churn_mid: u64,
    /// Коммитов за длинное окно (`window_long_days`).
    pub churn_long: u64,
    /// Суммарно добавлено строк за всю историю.
    pub lines_added: u64,
    /// Суммарно удалено строк за всю историю.
    pub lines_deleted: u64,
    /// Жив ли файл (последнее изменение — не удаление).
    pub is_alive: bool,
}

/// Посчитать churn по всем файлам репозитория из кэша.
///
/// Возвращает по строке на каждый канонический путь, затронутый хотя бы одним
/// учитываемым (не исключённым) коммитом. Порядок детерминирован (по пути).
pub fn compute_churn(store: &DuckStore, cfg: &ChurnConfig) -> Result<Vec<FileChurn>> {
    let conn = store.conn();

    // Якорь окон — последняя активность репо (детерминированно).
    let anchor: Option<i64> = conn
        .query_row(
            "SELECT CAST(epoch(max(committed_at)) AS BIGINT) FROM commits",
            [],
            |r| r.get(0),
        )
        .map_err(se)?;
    let Some(anchor) = anchor else {
        return Ok(Vec::new()); // пустой репозиторий
    };

    // Резолв переименований → таблица path → canonical.
    let canonical = crate::paths::build_canonical_map(conn)?;
    crate::paths::materialize_path_map(conn, &canonical)?;

    let cut_recent = anchor - cfg.window_recent_days as i64 * SECONDS_PER_DAY;
    let cut_mid = anchor - cfg.window_mid_days as i64 * SECONDS_PER_DAY;
    let cut_long = anchor - cfg.window_long_days as i64 * SECONDS_PER_DAY;
    let mech = if cfg.exclude_mechanical {
        "NOT c.is_mechanical"
    } else {
        "TRUE"
    };

    // Окна как count(DISTINCT CASE WHEN ...) — переносимый SQL без INTERVAL.
    // is_alive — тип последнего по времени изменения канонического пути.
    let sql = format!(
        "WITH mapped AS (
             SELECT pm.canonical AS path,
                    fc.sha AS sha,
                    fc.added AS added,
                    fc.deleted AS deleted,
                    fc.change_type AS change_type,
                    CAST(epoch(c.committed_at) AS BIGINT) AS ts
             FROM file_changes fc
             JOIN commits c ON fc.sha = c.sha
             JOIN path_map pm ON fc.path = pm.path
             WHERE {mech}
         ),
         agg AS (
             SELECT path,
                    count(DISTINCT sha) AS churn_total,
                    count(DISTINCT CASE WHEN ts >= {cut_recent} THEN sha END) AS churn_recent,
                    count(DISTINCT CASE WHEN ts >= {cut_mid} THEN sha END) AS churn_mid,
                    count(DISTINCT CASE WHEN ts >= {cut_long} THEN sha END) AS churn_long,
                    CAST(sum(added) AS BIGINT) AS lines_added,
                    CAST(sum(deleted) AS BIGINT) AS lines_deleted
             FROM mapped GROUP BY path
         ),
         last_change AS (
             SELECT path, change_type,
                    row_number() OVER (PARTITION BY path ORDER BY ts DESC, sha DESC) AS rn
             FROM mapped
         )
         SELECT a.path, a.churn_total, a.churn_recent, a.churn_mid, a.churn_long,
                a.lines_added, a.lines_deleted, (lc.change_type <> 'D') AS is_alive
         FROM agg a
         JOIN last_change lc ON a.path = lc.path AND lc.rn = 1
         ORDER BY a.path"
    );

    let mut stmt = conn.prepare(&sql).map_err(se)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(FileChurn {
                path: row.get(0)?,
                churn_total: row.get::<_, i64>(1)? as u64,
                churn_recent: row.get::<_, i64>(2)? as u64,
                churn_mid: row.get::<_, i64>(3)? as u64,
                churn_long: row.get::<_, i64>(4)? as u64,
                lines_added: row.get::<_, i64>(5)? as u64,
                lines_deleted: row.get::<_, i64>(6)? as u64,
                is_alive: row.get(7)?,
            })
        })
        .map_err(se)?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(se)?);
    }
    Ok(out)
}
