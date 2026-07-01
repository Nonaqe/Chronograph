//! Данные репорта, читаемые из материализованных таблиц стора.
//!
//! Порядок строк детерминирован (явные ORDER BY) — основа байт-идентичности HTML.
//! Wall-clock `analyzed_at` НЕ читается: в отчёт идут только детерминированные
//! `head_sha`/`engine_version`/`config_hash`.

use chronograph_core::error::BoxError;
use chronograph_core::{Error, Result};
use chronograph_store::DuckStore;

fn se<E: Into<BoxError>>(e: E) -> Error {
    Error::store(e)
}

/// Сводка по репозиторию (детерминированная мета + счётчики).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    pub head_sha: String,
    pub engine_version: String,
    pub config_hash: String,
    pub total_commits: u64,
    pub total_files: u64,
    pub hotspot_files: u64,
    pub coupling_pairs: u64,
}

/// Строка hotspot для treemap/таблицы.
#[derive(Debug, Clone, PartialEq)]
pub struct HotspotEntry {
    pub rank: u32,
    pub path: String,
    pub churn: u64,
    pub complexity: f64,
}

/// Строка coupling-таблицы.
#[derive(Debug, Clone, PartialEq)]
pub struct CouplingEntry {
    pub path_a: String,
    pub path_b: String,
    pub support: u64,
    pub ratio: f64,
}

/// Все данные для рендера одного отчёта.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportData {
    pub overview: Overview,
    pub hotspots: Vec<HotspotEntry>,
    pub couplings: Vec<CouplingEntry>,
}

impl ReportData {
    /// Прочитать данные отчёта из материализованных таблиц стора.
    pub fn from_store(store: &DuckStore) -> Result<Self> {
        let conn = store.conn();

        let (engine_version, config_hash, head_sha) = conn
            .query_row(
                "SELECT engine_version, config_hash, head_sha FROM analysis_meta LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(se)?;

        let total_commits: i64 = conn
            .query_row("SELECT count(*) FROM commits", [], |r| r.get(0))
            .map_err(se)?;
        let total_files: i64 = conn
            .query_row("SELECT count(*) FROM file_metrics", [], |r| r.get(0))
            .map_err(se)?;
        let hotspot_files: i64 = conn
            .query_row(
                "SELECT count(*) FROM file_metrics WHERE hotspot_rank IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(se)?;
        let coupling_pairs: i64 = conn
            .query_row("SELECT count(*) FROM coupling", [], |r| r.get(0))
            .map_err(se)?;

        let overview = Overview {
            head_sha: head_sha.chars().take(12).collect(),
            engine_version,
            config_hash,
            total_commits: total_commits as u64,
            total_files: total_files as u64,
            hotspot_files: hotspot_files as u64,
            coupling_pairs: coupling_pairs as u64,
        };

        // Hotspots — по возрастанию ранга (детерминированно, ранг уникален).
        let hotspots = {
            let mut stmt = conn
                .prepare(
                    "SELECT hotspot_rank, path, COALESCE(churn_total, 0), complexity \
                     FROM file_metrics WHERE hotspot_rank IS NOT NULL \
                     ORDER BY hotspot_rank",
                )
                .map_err(se)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(HotspotEntry {
                        rank: r.get::<_, i64>(0)? as u32,
                        path: r.get(1)?,
                        churn: r.get::<_, i64>(2)? as u64,
                        complexity: r.get(3)?,
                    })
                })
                .map_err(se)?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row.map_err(se)?);
            }
            v
        };

        // Coupling — по ratio↓, support↓, путям (полный детерминированный порядок).
        let couplings = {
            let mut stmt = conn
                .prepare(
                    "SELECT path_a, path_b, support, coupling_ratio FROM coupling \
                     ORDER BY coupling_ratio DESC, support DESC, path_a, path_b",
                )
                .map_err(se)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(CouplingEntry {
                        path_a: r.get(0)?,
                        path_b: r.get(1)?,
                        support: r.get::<_, i64>(2)? as u64,
                        ratio: r.get(3)?,
                    })
                })
                .map_err(se)?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row.map_err(se)?);
            }
            v
        };

        Ok(ReportData {
            overview,
            hotspots,
            couplings,
        })
    }
}
