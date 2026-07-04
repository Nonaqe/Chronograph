//! JSON-экспорт (§4.1/§6.1 ТЗ) — детерминированный артефакт для Web UI (`web/`).
//!
//! Один файл `chronograph.json`: meta + материализованные метрики (file_metrics,
//! coupling, knowledge, file_age, blame_skips) + ПОЛНЫЙ поток событий per-commit
//! (питает timeline scrubber и Gource-анимацию, решение 5c в CONTEXT.md).
//!
//! Детерминизм (правило 4): порядок полей фиксирован структурами, порядок строк —
//! явными ORDER BY, float сериализуется serde_json (ryu, shortest roundtrip —
//! платформонезависимо). Два прогона на одном репо → байт-в-байт одинаковый JSON
//! (e2e-тест + insta-снапшот схемы).
//!
//! Анонимизация (принцип 2.4): авторы по умолчанию — «Author #N», где N — плотный
//! 1-based индекс по возрастанию `author_id` среди авторов `commits` (события
//! покрывают всех авторов). Нумерация консистентна ВНУТРИ экспорта (events и
//! knowledge используют одну карту), но может отличаться от HTML-отчёта (там карта
//! строится только по knowledge-авторам). Реальные имена — только явный opt-in
//! (`ExportOptions::show_names`).

use std::collections::HashMap;

use chronograph_core::error::BoxError;
use chronograph_core::{Error, Result};
use chronograph_store::DuckStore;
use serde::Serialize;

fn se<E: Into<BoxError>>(e: E) -> Error {
    Error::store(e)
}

/// Версия схемы экспорта. Ломающее изменение структуры → инкремент (потребители
/// проверяют совместимость по этому полю, не по engine_version).
pub const EXPORT_SCHEMA_VERSION: u32 = 1;

/// Опции экспорта.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Реальные имена авторов вместо «Author #N». По умолчанию ВЫКЛ (принцип 2.4:
    /// knowledge/события — риск концентрации, не заслуга/вина).
    pub show_names: bool,
}

/// Детерминированная мета экспорта (без wall-clock `analyzed_at` — правило 4).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportMeta {
    pub schema_version: u32,
    pub engine_version: String,
    pub config_hash: String,
    /// Полный SHA HEAD-коммита прогона (в отличие от HTML, где усечён для показа).
    pub head_sha: String,
    /// Якорь времени (unix-секунды UTC) = max(committed_at). От него отсчитаны
    /// age-дни в `file_age`; UI по нему форматирует даты.
    pub anchor_ts: i64,
    pub total_commits: u64,
    pub total_authors: u64,
    /// true — авторы обезличены (Author #N); false — явный opt-in show_names.
    pub anonymized: bool,
}

/// Метрики одного файла (строка `file_metrics`; NULL там, где сигнала нет —
/// например complexity у файла без поддержанного языка).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileMetricsEntry {
    pub path: String,
    pub churn_total: Option<i64>,
    pub churn_30d: Option<i64>,
    pub churn_90d: Option<i64>,
    pub churn_365d: Option<i64>,
    pub complexity: Option<f64>,
    pub complexity_per_loc: Option<f64>,
    /// Ранг hotspot (1 = горячее всего); NULL — файл не участвует в ранжировании
    /// (нет cyclomatic complexity, см. решение Этапа 1).
    pub hotspot_rank: Option<i64>,
    pub is_alive: Option<bool>,
}

/// Пара change coupling (канонична: a < b; симметрия — по построению).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CouplingPairEntry {
    pub a: String,
    pub b: String,
    pub support: i64,
    pub ratio: f64,
}

/// Риск концентрации знаний по файлу (module = файл, решение Этапа 4).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnowledgeExportEntry {
    pub path: String,
    pub bus_factor: i64,
    pub top_owner_ratio: f64,
    /// Ярлык крупнейшего владельца — «Author #N» или имя при show_names.
    pub top_owner: String,
}

/// Распределение возраста строк файла перцентилями (§3.6), дни от anchor_ts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileAgeEntry {
    pub path: String,
    pub lines: i64,
    pub newest_age_days: i64,
    pub median_age_days: i64,
    pub p90_age_days: i64,
    pub oldest_age_days: i64,
}

/// Файл, выпавший из blame (knowledge/age), с машинно-читаемой причиной.
///
/// `reason` — код («over_budget» / «failed»), не человекочитаемый текст: экспорт —
/// данные, локализация/формулировка — забота потребителя (HTML-отчёт делает это сам).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlameSkipExportEntry {
    pub path: String,
    pub reason: String,
    pub cost: Option<i64>,
    pub budget: Option<i64>,
}

/// Одно изменение файла внутри коммита-события.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangeEvent {
    pub path: String,
    /// Код типа изменения как в git: A/M/D/R/C.
    #[serde(rename = "type")]
    pub change_type: String,
    /// Прежний путь для rename/copy; иначе null.
    pub old_path: Option<String>,
    pub added: i64,
    pub deleted: i64,
}

/// Событие-коммит потока timeline/анимации (только коммиты с изменениями файлов).
///
/// Порядок потока — (ts, sha): хронологичен, но при РАВНЫХ ts тай-брейк по sha
/// не топологичен (потомок может встать раньше родителя — то же ограничение, что
/// у is_alive Этапа 1; точное решение — топологический индекс коммита, backlog).
/// Для реальных репо с различающимися датами не проявляется.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommitEvent {
    pub sha: String,
    /// unix-секунды UTC.
    pub ts: i64,
    /// Ярлык автора («Author #N» или имя при show_names).
    pub author: String,
    /// Флаг механического коммита — фильтрация на стороне UI, данные не режем.
    pub mechanical: bool,
    pub changes: Vec<ChangeEvent>,
}

/// Полный документ экспорта.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Export {
    pub meta: ExportMeta,
    /// Все файлы с метриками (сортировка по пути). Hotspot-подмножество — по
    /// `hotspot_rank != null`; treemap строится по файлам с complexity.
    pub files: Vec<FileMetricsEntry>,
    /// Пары coupling — по ratio↓, support↓, путям.
    pub coupling: Vec<CouplingPairEntry>,
    /// Knowledge по риску: bus_factor возр., top_owner_ratio убыв., путь.
    pub knowledge: Vec<KnowledgeExportEntry>,
    /// Возраст файлов — по пути.
    pub file_age: Vec<FileAgeEntry>,
    /// Пропуски blame — по пути.
    pub blame_skips: Vec<BlameSkipExportEntry>,
    /// Поток событий — по (ts, sha), изменения внутри — по пути.
    pub events: Vec<CommitEvent>,
}

/// Собрать документ экспорта из материализованных таблиц стора.
pub fn build_export(store: &DuckStore, opts: &ExportOptions) -> Result<Export> {
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

    let (total_commits, anchor_ts): (i64, i64) = conn
        .query_row(
            "SELECT count(*), COALESCE(CAST(epoch(max(committed_at)) AS BIGINT), 0) \
             FROM commits",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(se)?;
    let total_authors: i64 = conn
        .query_row("SELECT count(*) FROM authors", [], |r| r.get(0))
        .map_err(se)?;

    let meta = ExportMeta {
        schema_version: EXPORT_SCHEMA_VERSION,
        engine_version,
        config_hash,
        head_sha,
        anchor_ts,
        total_commits: total_commits as u64,
        total_authors: total_authors as u64,
        anonymized: !opts.show_names,
    };

    let authors = author_labels(store, opts)?;
    let label = |id: i64| -> String {
        authors
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "Author #?".into())
    };

    // Файлы с метриками — по пути (полный детерминированный порядок: path уникален
    // в file_metrics по построению материализации).
    let files = {
        let mut stmt = conn
            .prepare(
                "SELECT path, churn_total, churn_30d, churn_90d, churn_365d, \
                        complexity, complexity_per_loc, hotspot_rank, is_alive \
                 FROM file_metrics ORDER BY path",
            )
            .map_err(se)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(FileMetricsEntry {
                    path: r.get(0)?,
                    churn_total: r.get(1)?,
                    churn_30d: r.get(2)?,
                    churn_90d: r.get(3)?,
                    churn_365d: r.get(4)?,
                    complexity: r.get(5)?,
                    complexity_per_loc: r.get(6)?,
                    hotspot_rank: r.get(7)?,
                    is_alive: r.get(8)?,
                })
            })
            .map_err(se)?;
        collect_rows(rows)?
    };

    // Coupling — тот же порядок, что в HTML-отчёте.
    let coupling = {
        let mut stmt = conn
            .prepare(
                "SELECT path_a, path_b, support, coupling_ratio FROM coupling \
                 ORDER BY coupling_ratio DESC, support DESC, path_a, path_b",
            )
            .map_err(se)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CouplingPairEntry {
                    a: r.get(0)?,
                    b: r.get(1)?,
                    support: r.get(2)?,
                    ratio: r.get(3)?,
                })
            })
            .map_err(se)?;
        collect_rows(rows)?
    };

    // Knowledge по риску, топ-владелец — детерминированный tie-break (как в data.rs).
    let knowledge = {
        let mut stmt = conn
            .prepare(
                "SELECT module, bus_factor, top_owner_ratio, top_author FROM (\
                     SELECT m.module AS module, m.bus_factor AS bus_factor, \
                            m.top_owner_ratio AS top_owner_ratio, \
                            k.author_id AS top_author, \
                            row_number() OVER (PARTITION BY m.module \
                                ORDER BY k.ownership_ratio DESC, k.author_id ASC) AS rn \
                     FROM module_bus_factor m \
                     JOIN knowledge k ON k.path = m.module\
                 ) WHERE rn = 1 \
                 ORDER BY bus_factor ASC, top_owner_ratio DESC, module ASC",
            )
            .map_err(se)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(se)?;
        let mut v = Vec::new();
        for row in rows {
            let (path, bus_factor, top_owner_ratio, top_author) = row.map_err(se)?;
            v.push(KnowledgeExportEntry {
                path,
                bus_factor,
                top_owner_ratio,
                top_owner: label(top_author),
            });
        }
        v
    };

    let file_age = {
        let mut stmt = conn
            .prepare(
                "SELECT path, lines, newest_age_days, median_age_days, p90_age_days, \
                        oldest_age_days \
                 FROM file_age ORDER BY path",
            )
            .map_err(se)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(FileAgeEntry {
                    path: r.get(0)?,
                    lines: r.get(1)?,
                    newest_age_days: r.get(2)?,
                    median_age_days: r.get(3)?,
                    p90_age_days: r.get(4)?,
                    oldest_age_days: r.get(5)?,
                })
            })
            .map_err(se)?;
        collect_rows(rows)?
    };

    // Служебная таблица может отсутствовать на старом кэше — идемпотентно создаём.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS blame_skips \
         (path TEXT, reason TEXT, cost BIGINT, budget BIGINT);",
    )
    .map_err(se)?;
    let blame_skips = {
        let mut stmt = conn
            .prepare("SELECT path, reason, cost, budget FROM blame_skips ORDER BY path")
            .map_err(se)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(BlameSkipExportEntry {
                    path: r.get(0)?,
                    reason: r.get(1)?,
                    cost: r.get(2)?,
                    budget: r.get(3)?,
                })
            })
            .map_err(se)?;
        collect_rows(rows)?
    };

    // Поток событий: коммиты с изменениями файлов, в хронологическом порядке.
    // Полный тай-брейк в ORDER BY: один коммит может содержать НЕСКОЛЬКО строк с
    // одинаковым path (D + R при переиспользовании имени — см. баг детерминизма
    // Этапа 1 в CONTEXT.md), поэтому сортируем и по change_type/old_path.
    let events = {
        let mut stmt = conn
            .prepare(
                "SELECT c.sha, CAST(epoch(c.committed_at) AS BIGINT), c.author_id, \
                        c.is_mechanical, f.path, f.change_type, f.old_path, \
                        f.added, f.deleted \
                 FROM commits c JOIN file_changes f ON f.sha = c.sha \
                 ORDER BY c.committed_at, c.sha, f.path, f.change_type, f.old_path",
            )
            .map_err(se)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, bool>(3)?,
                    ChangeEvent {
                        path: r.get(4)?,
                        change_type: r.get(5)?,
                        old_path: r.get(6)?,
                        added: r.get(7)?,
                        deleted: r.get(8)?,
                    },
                ))
            })
            .map_err(se)?;
        let mut v: Vec<CommitEvent> = Vec::new();
        for row in rows {
            let (sha, ts, author_id, mechanical, change) = row.map_err(se)?;
            match v.last_mut() {
                Some(last) if last.sha == sha => last.changes.push(change),
                _ => v.push(CommitEvent {
                    sha,
                    ts,
                    author: label(author_id),
                    mechanical,
                    changes: vec![change],
                }),
            }
        }
        v
    };

    Ok(Export {
        meta,
        files,
        coupling,
        knowledge,
        file_age,
        blame_skips,
        events,
    })
}

/// Сериализовать экспорт в компактный JSON (детерминированно).
pub fn export_json(store: &DuckStore, opts: &ExportOptions) -> Result<String> {
    let doc = build_export(store, opts)?;
    serde_json::to_string(&doc).map_err(se)
}

/// Карта `author_id → ярлык`.
///
/// Анонимный режим: плотный 1-based индекс по возрастанию `author_id` среди авторов
/// `commits` (порядок ingestion — детерминирован, личности не раскрывает).
/// `show_names`: канонические имена из `authors` (после mailmap), пустое имя →
/// канонический email.
fn author_labels(store: &DuckStore, opts: &ExportOptions) -> Result<HashMap<i64, String>> {
    let conn = store.conn();
    let mut map = HashMap::new();
    if opts.show_names {
        let mut stmt = conn
            .prepare(
                "SELECT author_id, COALESCE(NULLIF(canonical_name, ''), canonical_email) \
                 FROM authors ORDER BY author_id",
            )
            .map_err(se)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(se)?;
        for row in rows {
            let (id, name) = row.map_err(se)?;
            map.insert(id, name);
        }
    } else {
        let mut stmt = conn
            .prepare("SELECT DISTINCT author_id FROM commits ORDER BY author_id")
            .map_err(se)?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0)).map_err(se)?;
        for (i, row) in rows.enumerate() {
            map.insert(row.map_err(se)?, format!("Author #{}", i + 1));
        }
    }
    Ok(map)
}

fn collect_rows<T>(
    rows: impl Iterator<Item = std::result::Result<T, duckdb::Error>>,
) -> Result<Vec<T>> {
    let mut v = Vec::new();
    for row in rows {
        v.push(row.map_err(se)?);
    }
    Ok(v)
}
