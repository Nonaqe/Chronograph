//! Команда `chronograph hotspots` — таблица топ hotspots в терминал (без графики).
//!
//! Приводит кэш в актуальное состояние (инкрементальный analyze), затем считает
//! churn + complexity (из git-объектов) и hotspot-ранг (`churn_pct × cx_pct`).
//! Ранжируются только живые файлы с cyclomatic complexity (см. CONTEXT.md).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{
    compute_churn, compute_complexity, compute_hotspots, ChurnConfig, HotspotConfig,
};
use chronograph_store::DuckStore;
use clap::Args;

const CACHE_REL_PATH: &str = ".chronograph/cache.duckdb";

#[derive(Args)]
pub struct HotspotsArgs {
    /// Путь к git-репозиторию (по умолчанию — текущая директория).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Путь к файлу кэша DuckDB (по умолчанию `<repo>/.chronograph/cache.duckdb`).
    #[arg(long = "db", value_name = "FILE")]
    pub db: Option<PathBuf>,

    /// Сколько строк показать.
    #[arg(long = "top", value_name = "N", default_value_t = 20)]
    pub top: usize,

    /// Glob-паттерны исключаемых путей (vendored/generated). Можно повторять.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,
}

pub fn run(args: HotspotsArgs) -> anyhow::Result<()> {
    let cfg = Config {
        repo_path: args.path.clone(),
        since: None,
        exclude: args.exclude.clone(),
        incremental: true,
        mechanical_commit_max_files: None,
    };
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| args.path.join(CACHE_REL_PATH));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("не удалось создать каталог кэша {}", parent.display()))?;
    }

    let source = GitSource::open(&cfg)
        .with_context(|| format!("открытие репозитория {}", cfg.repo_path.display()))?;
    let mut store = DuckStore::open(&db_path)
        .with_context(|| format!("открытие кэша {}", db_path.display()))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Инкрементально доводим кэш до HEAD.
    run_analysis(&source, &mut store, &cfg, now).context("анализ истории")?;

    let churn = compute_churn(&store, &ChurnConfig::default()).context("подсчёт churn")?;
    let complexity =
        compute_complexity(&store, &source).context("подсчёт complexity из git-объектов")?;
    let hotspots = compute_hotspots(&churn, &complexity, &HotspotConfig::default());

    if hotspots.is_empty() {
        println!(
            "Нет файлов с поддержанной complexity (Rust/Python/Go/JS/TS) — hotspot-таблица пуста."
        );
        return Ok(());
    }

    println!(
        "{:>3}  {:<44} {:>6} {:>5} {:>7} {:>7} {:>6}",
        "#", "path", "churn", "cx", "churn%", "cx%", "score"
    );
    for h in hotspots.iter().take(args.top) {
        println!(
            "{:>3}  {:<44} {:>6} {:>5} {:>7.2} {:>7.2} {:>6.3}",
            h.rank,
            truncate(&h.path, 44),
            h.churn,
            h.complexity as u64,
            h.churn_pct,
            h.complexity_pct,
            h.score,
        );
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let tail: String = s
            .chars()
            .rev()
            .take(max - 1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    }
}
