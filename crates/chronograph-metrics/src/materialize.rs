//! Материализация аналитических метрик в §7-таблицы DuckDB.
//!
//! Считает churn/complexity/hotspot/coupling один раз и персистит в `file_metrics`
//! и `coupling`. Потребитель — `chronograph-report` (репорт читает готовые таблицы,
//! ТЗ §4.3), а также будущий export. Интерактивные CLI `hotspots`/`coupling`
//! остаются on-the-fly и материализацию не используют.
//!
//! Идемпотентно: полный DELETE + INSERT в одной транзакции.

use std::collections::HashMap;

use chronograph_core::{BlameSource, BlobReader, Result};
use chronograph_store::DuckStore;
use duckdb::{params, types::Value};

use crate::churn::compute_churn;
use crate::complexity::compute_complexity;
use crate::config::{ChurnConfig, KnowledgeConfig};
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
    /// Конфиг knowledge / bus factor.
    pub knowledge: KnowledgeConfig,
}

/// Итог материализации.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeSummary {
    /// Строк в `file_metrics`.
    pub file_metrics_rows: usize,
    /// Строк в `coupling`.
    pub coupling_rows: usize,
    /// Строк в `knowledge` (пар файл×автор).
    pub knowledge_rows: usize,
    /// Строк в `module_bus_factor` (по файлу).
    pub module_bus_factor_rows: usize,
    /// Строк в `file_age` (по файлу).
    pub file_age_rows: usize,
    /// Живых файлов, пропущенных из-за сбоя blame (паника gix-blame) — не молча.
    pub blame_skipped: usize,
}

/// Посчитать и записать аналитические таблицы.
///
/// `reader` нужен complexity (blob'ы), `reader` же как [`BlameSource`] — knowledge
/// (построчный blame). `GitSource` реализует оба трейта.
pub fn materialize<R: BlobReader + BlameSource>(
    store: &DuckStore,
    reader: &R,
    cfg: &MaterializeConfig,
) -> Result<MaterializeSummary> {
    let churn = compute_churn(store, &cfg.churn)?;
    let complexity = compute_complexity(store, reader)?;
    let hotspots = compute_hotspots(&churn, &complexity, &cfg.hotspot);
    let coupling = compute_coupling(store, &cfg.coupling)?;

    // ОБЩИЙ blame-проход для knowledge + age: blame — самая дорогая операция
    // (профилирование: ×20 к остальным метрикам), блеймить дважды на одних файлах
    // нельзя. Блеймим ОДИН раз (через инкрементальный кэш + largest-first, см.
    // blame_cache) и питаем обе метрики через их `from_blame`.
    let conn = store.conn();
    let head: Option<String> = conn
        .query_row("SELECT head_sha FROM analysis_meta LIMIT 1", [], |r| {
            r.get(0)
        })
        .ok();
    let files_meta = crate::paths::living_files_meta(conn)?;
    let (blamed, skips) = match &head {
        Some(h) => crate::blame_cache::cached_blame_many(
            store,
            reader,
            &files_meta,
            h,
            cfg.knowledge.blame_budget,
        )?,
        None => (Vec::new(), Vec::new()),
    };
    let files: Vec<String> = files_meta.into_iter().map(|f| f.path).collect();
    let commit_author = crate::knowledge::load_commit_authors(store)?;
    let commit_time = crate::age::load_commit_times(store)?;
    let anchor = crate::age::load_anchor(store)?.unwrap_or(0);
    let knowledge = crate::knowledge::from_blame(&commit_author, &files, &blamed, &cfg.knowledge);
    let age = crate::age::from_blame(&commit_time, anchor, &files, &blamed);

    // `knowledge_meta`/`blame_skips` — внутренние таблицы аналитического слоя (не
    // §7-схема, как complexity_cache): счётчик и ПОИМЁННЫЕ причины пропусков blame
    // для отчёта (файл — причина: сбой gix-blame или превышение бюджета).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS knowledge_meta (blame_skipped INTEGER);
         CREATE TABLE IF NOT EXISTS blame_skips (
             path   TEXT,
             reason TEXT,     -- 'failed' | 'over_budget'
             cost   BIGINT,   -- строко-ревизии (для over_budget; NULL для failed)
             budget BIGINT    -- действовавший бюджет (для over_budget; NULL для failed)
         );",
    )
    .map_err(se)?;
    conn.execute_batch("BEGIN TRANSACTION").map_err(se)?;

    conn.execute_batch(
        "DELETE FROM file_metrics; DELETE FROM coupling; \
         DELETE FROM knowledge; DELETE FROM module_bus_factor; \
         DELETE FROM knowledge_meta; DELETE FROM file_age; DELETE FROM blame_skips;",
    )
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

    // knowledge (файл × автор) + module_bus_factor (по файлу; module = путь, v1).
    // Порядок записи детерминирован: knowledge отсортирован по пути, owners — внутри.
    let mut kn_stmt = conn
        .prepare("INSERT INTO knowledge (path, author_id, ownership_ratio) VALUES (?, ?, ?)")
        .map_err(se)?;
    let mut bf_stmt = conn
        .prepare(
            "INSERT INTO module_bus_factor (module, bus_factor, top_owner_ratio) \
             VALUES (?, ?, ?)",
        )
        .map_err(se)?;
    let mut knowledge_rows = 0usize;
    for fk in &knowledge.files {
        for o in &fk.owners {
            kn_stmt
                .execute(params![fk.path, o.author_id, o.ownership_ratio])
                .map_err(se)?;
            knowledge_rows += 1;
        }
        bf_stmt
            .execute(params![fk.path, fk.bus_factor as i64, fk.top_owner_ratio])
            .map_err(se)?;
    }
    drop(kn_stmt);
    drop(bf_stmt);

    // file_age (по файлу): распределение возраста строк перцентилями. Из ТОГО ЖЕ
    // blame-прохода, что и knowledge (общий `blamed`). Порядок детерминирован (по пути).
    let mut age_stmt = conn
        .prepare(
            "INSERT INTO file_age \
             (path, lines, newest_age_days, median_age_days, p90_age_days, oldest_age_days) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .map_err(se)?;
    for fa in &age.files {
        age_stmt
            .execute(params![
                fa.path,
                fa.lines as i64,
                fa.newest_age_days,
                fa.median_age_days,
                fa.p90_age_days,
                fa.oldest_age_days
            ])
            .map_err(se)?;
    }
    drop(age_stmt);

    // Счётчик и поимённые причины пропусков blame — для явного показа в отчёте.
    conn.execute(
        "INSERT INTO knowledge_meta (blame_skipped) VALUES (?)",
        params![skips.len() as i64],
    )
    .map_err(se)?;
    {
        let mut sk_stmt = conn
            .prepare("INSERT INTO blame_skips (path, reason, cost, budget) VALUES (?, ?, ?, ?)")
            .map_err(se)?;
        for s in &skips {
            match &s.reason {
                crate::blame_cache::SkipReason::Failed => {
                    sk_stmt
                        .execute(params![s.path, "failed", Value::Null, Value::Null])
                        .map_err(se)?;
                }
                crate::blame_cache::SkipReason::OverBudget { cost, budget } => {
                    sk_stmt
                        .execute(params![s.path, "over_budget", *cost as i64, *budget as i64])
                        .map_err(se)?;
                }
            }
        }
    }

    conn.execute_batch("COMMIT").map_err(se)?;

    Ok(MaterializeSummary {
        file_metrics_rows,
        coupling_rows: coupling.len(),
        knowledge_rows,
        module_bus_factor_rows: knowledge.files.len(),
        file_age_rows: age.files.len(),
        blame_skipped: knowledge.blame_skipped,
    })
}
