//! Данные репорта, читаемые из материализованных таблиц стора.
//!
//! Порядок строк детерминирован (явные ORDER BY) — основа байт-идентичности HTML.
//! Wall-clock `analyzed_at` НЕ читается: в отчёт идут только детерминированные
//! `head_sha`/`engine_version`/`config_hash`.

use std::collections::HashMap;

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
    /// Файлов с bus_factor = 1 (риск концентрации знаний).
    pub bus_factor_one: u64,
    /// Живых файлов, пропущенных из-за сбоя blame (паника gix-blame) — явно, не молча.
    pub blame_skipped: u64,
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

/// Строка knowledge / bus factor (риск концентрации знаний по файлу).
///
/// Автор всегда АНОНИМИЗИРОВАН (`top_owner` = «Author #N») — публичный отчёт не
/// раскрывает имён (принцип 2.4 CLAUDE.md: риск, не заслуга/вина).
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeEntry {
    pub module: String,
    pub bus_factor: u32,
    pub top_owner_ratio: f64,
    /// Анонимный ярлык крупнейшего владельца, напр. «Author #1».
    pub top_owner: String,
}

/// Файл, пропущенный из blame (выпал из knowledge/age), с человекочитаемой причиной.
///
/// Показывается в отчёте ПОИМЁННО — прозрачность решения (принцип 2.6: никаких
/// молчаливых дыр в метриках).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameSkipEntry {
    pub path: String,
    /// Причина, готовая к показу (детерминированная строка).
    pub reason: String,
}

/// Все данные для рендера одного отчёта.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportData {
    pub overview: Overview,
    pub hotspots: Vec<HotspotEntry>,
    pub couplings: Vec<CouplingEntry>,
    pub knowledge: Vec<KnowledgeEntry>,
    /// Median-возраст (дни) каждого файла — сырьё для гистограммы возраста (§3.6).
    /// Отсортирован возр. (детерминизм). Бакеты выбираются на слое рендера.
    pub age_medians: Vec<i64>,
    /// Пропущенные из blame файлы с причинами (порядок — по пути).
    pub blame_skips: Vec<BlameSkipEntry>,
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
        let bus_factor_one: i64 = conn
            .query_row(
                "SELECT count(*) FROM module_bus_factor WHERE bus_factor = 1",
                [],
                |r| r.get(0),
            )
            .map_err(se)?;
        // knowledge_meta — внутренняя таблица metrics; на старом кэше её может не
        // быть, поэтому идемпотентно создаём (default 0) перед чтением.
        conn.execute_batch("CREATE TABLE IF NOT EXISTS knowledge_meta (blame_skipped INTEGER);")
            .map_err(se)?;
        let blame_skipped: i64 = conn
            .query_row(
                "SELECT COALESCE(max(blame_skipped), 0) FROM knowledge_meta",
                [],
                |r| r.get(0),
            )
            .map_err(se)?;

        let overview = Overview {
            head_sha: head_sha.chars().take(12).collect(),
            engine_version,
            config_hash,
            total_commits: total_commits as u64,
            total_files: total_files as u64,
            hotspot_files: hotspot_files as u64,
            coupling_pairs: coupling_pairs as u64,
            bus_factor_one: bus_factor_one as u64,
            blame_skipped: blame_skipped as u64,
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

        // Анонимизация: author_id → «Author #N», где N — плотный индекс по
        // возрастанию author_id среди авторов в knowledge. Детерминировано, имён не
        // раскрывает (author_id — порядок ingestion, не значащий сигнал).
        let anon = build_anon_map(store)?;

        // Knowledge по риску: bus_factor возр. (1 = риск наверху), затем
        // top_owner_ratio убыв., затем путь. Топ-владелец на модуль — детерминированный
        // tie-break (доля убыв., author_id возр.).
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
                        r.get::<_, i64>(1)? as u32,
                        r.get::<_, f64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .map_err(se)?;
            let mut v = Vec::new();
            for row in rows {
                let (module, bus_factor, top_owner_ratio, top_author) = row.map_err(se)?;
                v.push(KnowledgeEntry {
                    module,
                    bus_factor,
                    top_owner_ratio,
                    top_owner: anon
                        .get(&top_author)
                        .cloned()
                        .unwrap_or_else(|| "Author #?".into()),
                });
            }
            v
        };

        // Median-возраст по файлам для гистограммы (§3.6). Порядок — возр. (детерминизм).
        let age_medians = {
            let mut stmt = conn
                .prepare("SELECT median_age_days FROM file_age ORDER BY median_age_days")
                .map_err(se)?;
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0)).map_err(se)?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row.map_err(se)?);
            }
            v
        };

        // Пропуски blame с причинами (служебная таблица; на старом кэше может
        // отсутствовать — идемпотентно создаём пустую).
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
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                })
                .map_err(se)?;
            let mut v = Vec::new();
            for row in rows {
                let (path, reason, cost, budget) = row.map_err(se)?;
                let reason = match (reason.as_str(), cost, budget) {
                    ("over_budget", Some(c), Some(b)) => format!(
                        "слишком дорог для blame: стоимость {c} строко-ревизий > бюджета {b} \
                         (файл с очень большой историей изменений; поднять: --blame-budget)"
                    ),
                    ("failed", _, _) => {
                        "сбой blame (известный баг gix-blame на этом файле)".to_string()
                    }
                    _ => reason,
                };
                v.push(BlameSkipEntry { path, reason });
            }
            v
        };

        Ok(ReportData {
            overview,
            hotspots,
            couplings,
            knowledge,
            age_medians,
            blame_skips,
        })
    }
}

/// Построить анонимную карту `author_id → "Author #N"`.
///
/// N — плотный 1-based индекс по возрастанию `author_id` среди авторов, реально
/// присутствующих в `knowledge`. Детерминировано и не раскрывает личности.
fn build_anon_map(store: &DuckStore) -> Result<HashMap<i64, String>> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare("SELECT DISTINCT author_id FROM knowledge ORDER BY author_id")
        .map_err(se)?;
    let rows = stmt.query_map([], |r| r.get::<_, i64>(0)).map_err(se)?;
    let mut map = HashMap::new();
    for (i, row) in rows.enumerate() {
        map.insert(row.map_err(se)?, format!("Author #{}", i + 1));
    }
    Ok(map)
}
