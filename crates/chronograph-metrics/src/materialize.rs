//! Материализация аналитических метрик в §7-таблицы DuckDB.
//!
//! Считает churn/complexity/hotspot/coupling один раз и персистит в `file_metrics`
//! и `coupling`. Потребитель — `chronograph-report` (репорт читает готовые таблицы,
//! ТЗ §4.3), а также будущий export. Интерактивные CLI `hotspots`/`coupling`
//! остаются on-the-fly и материализацию не используют.
//!
//! Идемпотентно: полный DELETE + INSERT в одной транзакции.

use std::collections::HashMap;

use chronograph_core::{BlobReader, Result};
use chronograph_store::DuckStore;
use duckdb::{params, types::Value};

use crate::churn::compute_churn;
use crate::complexity::compute_complexity;
use crate::config::ChurnConfig;
use crate::coupling::{compute_coupling, CouplingConfig};
use crate::hotspot::{compute_hotspots, HotspotConfig};
use crate::store_err as se;

/// Конфигурация материализации (все под-метрики).
#[derive(Debug, Clone, Default)]
pub struct MaterializeConfig {
    /// Конфиг churn.
    pub churn: ChurnConfig,
    /// Конфиг hotspot.
    pub hotspot: HotspotConfig,
    /// Конфиг coupling.
    pub coupling: CouplingConfig,
}

/// Итог материализации.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeSummary {
    /// Строк в `file_metrics`.
    pub file_metrics_rows: usize,
    /// Строк в `coupling`.
    pub coupling_rows: usize,
}

/// Посчитать и записать аналитические таблицы. `reader` нужен complexity (blob'ы).
pub fn materialize<R: BlobReader>(
    store: &DuckStore,
    reader: &R,
    cfg: &MaterializeConfig,
) -> Result<MaterializeSummary> {
    let churn = compute_churn(store, &cfg.churn)?;
    let complexity = compute_complexity(store, reader)?;
    let hotspots = compute_hotspots(&churn, &complexity, &cfg.hotspot);
    let coupling = compute_coupling(store, &cfg.coupling)?;

    let conn = store.conn();
    conn.execute_batch("BEGIN TRANSACTION").map_err(se)?;

    conn.execute_batch("DELETE FROM file_metrics; DELETE FROM coupling;")
        .map_err(se)?;

    // hotspot rank и complexity по пути.
    let rank_by_path: HashMap<&str, u32> =
        hotspots.iter().map(|h| (h.path.as_str(), h.rank)).collect();
    let cx_by_path: HashMap<&str, &crate::FileComplexityRow> =
        complexity.iter().map(|c| (c.path.as_str(), c)).collect();
    let churn_by_path: HashMap<&str, &crate::FileChurn> =
        churn.iter().map(|c| (c.path.as_str(), c)).collect();

    // Универсум file_metrics: объединение путей churn и complexity.
    let mut paths: Vec<&str> = churn_by_path
        .keys()
        .chain(cx_by_path.keys())
        .copied()
        .collect();
    paths.sort_unstable();
    paths.dedup();

    let mut fm_stmt = conn
        .prepare(
            "INSERT INTO file_metrics \
             (path, churn_total, churn_30d, churn_90d, churn_365d, \
              complexity, complexity_per_loc, hotspot_rank, is_alive) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .map_err(se)?;

    let mut file_metrics_rows = 0usize;
    for path in &paths {
        let ch = churn_by_path.get(path);
        let cx = cx_by_path.get(path);
        let is_alive = ch
            .map(|c| c.is_alive)
            // complexity считается только для живых файлов
            .unwrap_or(cx.is_some());

        fm_stmt
            .execute(params![
                path,
                ch.map(|c| c.churn_total as i64),
                ch.map(|c| c.churn_recent as i64),
                ch.map(|c| c.churn_mid as i64),
                ch.map(|c| c.churn_long as i64),
                cx.map(|c| c.value),
                cx.map(|c| c.per_loc),
                rank_by_path.get(path).map(|r| *r as i64),
                is_alive,
            ])
            .map_err(se)?;
        file_metrics_rows += 1;
    }
    drop(fm_stmt);

    // coupling: explained_by_imports пока NULL (language-aware — backlog, ТЗ 3.4).
    let mut cp_stmt = conn
        .prepare(
            "INSERT INTO coupling \
             (path_a, path_b, support, coupling_ratio, explained_by_imports) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .map_err(se)?;
    for c in &coupling {
        cp_stmt
            .execute(params![
                c.path_a,
                c.path_b,
                c.support as i64,
                c.coupling_ratio,
                Value::Null
            ])
            .map_err(se)?;
    }
    drop(cp_stmt);

    conn.execute_batch("COMMIT").map_err(se)?;

    Ok(MaterializeSummary {
        file_metrics_rows,
        coupling_rows: coupling.len(),
    })
}
