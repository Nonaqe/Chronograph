//! Команда `chronograph report` — self-contained HTML-репорт.
//!
//! Полный путь: инкрементальный analyze → материализация аналитических таблиц →
//! рендер `report.html` (Overview + Hotspots treemap + Coupling). Ноль внешних
//! зависимостей в самом файле.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{materialize, MaterializeConfig};
use chronograph_store::DuckStore;
use clap::Args;

const CACHE_REL_PATH: &str = ".chronograph/cache.duckdb";

#[derive(Args)]
pub struct ReportArgs {
    /// Путь к git-репозиторию (по умолчанию — текущая директория).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Куда записать HTML (по умолчанию `report.html` в текущей директории).
    #[arg(long = "out", value_name = "FILE", default_value = "report.html")]
    pub out: PathBuf,

    /// Путь к файлу кэша DuckDB (по умолчанию `<repo>/.chronograph/cache.duckdb`).
    #[arg(long = "db", value_name = "FILE")]
    pub db: Option<PathBuf>,

    /// Glob-паттерны исключаемых путей (vendored/generated). Можно повторять.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,
}

pub fn run(args: ReportArgs) -> anyhow::Result<()> {
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
    run_analysis(&source, &mut store, &cfg, now).context("анализ истории")?;
    materialize(&store, &source, &MaterializeConfig::default()).context("материализация метрик")?;
    chronograph_report::generate(&store, &args.out)
        .with_context(|| format!("генерация отчёта {}", args.out.display()))?;

    println!("Отчёт записан: {}", args.out.display());
    Ok(())
}
