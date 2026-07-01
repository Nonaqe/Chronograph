//! Команда `chronograph analyze` — строит инкрементальный кэш истории.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_store::DuckStore;
use clap::Args;

/// Относительный путь кэша внутри анализируемого репозитория.
const CACHE_REL_PATH: &str = ".chronograph/cache.duckdb";

#[derive(Args)]
pub struct AnalyzeArgs {
    /// Путь к git-репозиторию (по умолчанию — текущая директория).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Путь к файлу кэша DuckDB (по умолчанию `<repo>/.chronograph/cache.duckdb`).
    #[arg(long = "db", value_name = "FILE")]
    pub db: Option<PathBuf>,

    /// Glob-паттерны исключаемых путей (vendored/generated). Можно повторять.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Форсировать полный пересчёт вместо инкрементального.
    #[arg(long = "no-incremental")]
    pub no_incremental: bool,

    /// Порог «механического» коммита по числу файлов (по умолчанию выключено).
    #[arg(long = "mechanical-max-files", value_name = "N")]
    pub mechanical_max_files: Option<u32>,
}

impl AnalyzeArgs {
    /// Собрать конфиг ядра из аргументов CLI.
    pub fn to_config(&self) -> Config {
        Config {
            repo_path: self.path.clone(),
            since: None,
            exclude: self.exclude.clone(),
            incremental: !self.no_incremental,
            mechanical_commit_max_files: self.mechanical_max_files,
        }
    }

    /// Итоговый путь к файлу кэша.
    fn db_path(&self) -> PathBuf {
        self.db
            .clone()
            .unwrap_or_else(|| self.path.join(CACHE_REL_PATH))
    }
}

pub fn run(args: AnalyzeArgs) -> anyhow::Result<()> {
    let cfg = args.to_config();
    let db_path = args.db_path();

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

    let outcome = run_analysis(&source, &mut store, &cfg, now).context("анализ истории")?;

    match outcome.head_sha {
        None => println!("Репозиторий пуст — нечего анализировать."),
        Some(head) if outcome.up_to_date => {
            println!("Кэш актуален (HEAD {head}); новых коммитов нет.");
        }
        Some(head) => {
            println!(
                "Обработано новых коммитов: {}. HEAD: {head}",
                outcome.new_commits
            );
            println!("Кэш: {}", db_path.display());
        }
    }
    Ok(())
}
