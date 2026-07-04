//! Команда `chronograph export` — детерминированный JSON-экспорт (§4.1/§6.1).
//!
//! Полный путь как у `report`: инкрементальный analyze → материализация → экспорт
//! одного `chronograph.json` (метрики + поток событий per-commit). Потребитель —
//! Web UI (`web/`), но файл самодостаточен для любого пайплайна.
//!
//! Авторы анонимизированы по умолчанию (Author #N, принцип 2.4); реальные имена —
//! только по явному `--show-names`. Формат пока только `json`; `parquet` — follow-up
//! (флаг заложен, чтобы CLI не менять).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{materialize, KnowledgeConfig, MaterializeConfig, DEFAULT_BLAME_BUDGET};
use chronograph_report::ExportOptions;
use chronograph_store::DuckStore;
use clap::{Args, ValueEnum};

const CACHE_REL_PATH: &str = ".chronograph/cache.duckdb";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExportFormat {
    /// Один self-contained chronograph.json.
    Json,
}

#[derive(Args)]
pub struct ExportArgs {
    /// Путь к git-репозиторию (по умолчанию — текущая директория).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Куда записать экспорт (по умолчанию `chronograph.json` в текущей директории).
    #[arg(long = "out", value_name = "FILE", default_value = "chronograph.json")]
    pub out: PathBuf,

    /// Формат экспорта. Пока только json; parquet — в бэклоге.
    #[arg(long = "format", value_enum, default_value_t = ExportFormat::Json)]
    pub format: ExportFormat,

    /// Путь к файлу кэша DuckDB (по умолчанию `<repo>/.chronograph/cache.duckdb`).
    #[arg(long = "db", value_name = "FILE")]
    pub db: Option<PathBuf>,

    /// Показать реальные имена авторов вместо анонимных Author #N.
    ///
    /// По умолчанию ВЫКЛ (принцип 2.4: экспорт — публичный артефакт).
    #[arg(long = "show-names", default_value_t = false)]
    pub show_names: bool,

    /// Glob-паттерны исключаемых путей (vendored/generated). Можно повторять.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Бюджет blame на файл (строко-ревизии: ревизии × добавленные строки).
    /// Файлы дороже — пропускаются; причины видны в blame_skips. 0 — безлимит.
    #[arg(long = "blame-budget", value_name = "N", default_value_t = DEFAULT_BLAME_BUDGET)]
    pub blame_budget: u64,
}

pub fn run(args: ExportArgs) -> anyhow::Result<()> {
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
    let mat_cfg = MaterializeConfig {
        knowledge: KnowledgeConfig {
            blame_budget: args.blame_budget,
            ..Default::default()
        },
        ..Default::default()
    };
    materialize(&store, &source, &mat_cfg).context("материализация метрик")?;

    let opts = ExportOptions {
        show_names: args.show_names,
    };
    chronograph_report::generate_json(&store, &args.out, &opts)
        .with_context(|| format!("экспорт {}", args.out.display()))?;

    println!("Экспорт записан: {}", args.out.display());
    Ok(())
}
