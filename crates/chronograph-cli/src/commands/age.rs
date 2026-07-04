//! Команда `chronograph age` — распределение возраста строк по файлам (§3.6).
//!
//! Как hotspots/coupling/knowledge: инкрементально доводит кэш до HEAD, считает code
//! age (blame → возраст строк по последнему коммиту), печатает таблицу. Без графики.
//!
//! Возраст в днях от `anchor = max(committed_at)` (детерминированно, не wall-clock).
//! Малый median = зона постоянного переписывания; большой = стабильный старый код.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{compute_age, FileAge, DEFAULT_BLAME_BUDGET};
use chronograph_store::DuckStore;
use clap::Args;

const CACHE_REL_PATH: &str = ".chronograph/cache.duckdb";

#[derive(Args)]
pub struct AgeArgs {
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

    /// Бюджет blame на файл (строко-ревизии: ревизии × добавленные строки).
    /// Файлы дороже — пропускаются с явной пометкой. 0 — безлимит.
    #[arg(long = "blame-budget", value_name = "N", default_value_t = DEFAULT_BLAME_BUDGET)]
    pub blame_budget: u64,
}

pub fn run(args: AgeArgs) -> anyhow::Result<()> {
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

    let report = compute_age(&store, &source, args.blame_budget).context("code age")?;
    print!("{}", render(&report.files, args.top, report.blame_skipped));
    Ok(())
}

/// Отрендерить таблицу возраста (чистая функция — тестируется без БД).
///
/// Сортировка: median возр. (сверху — самые переписываемые: малый возраст), затем
/// путь — детерминированно.
fn render(files: &[FileAge], top: usize, blame_skipped: usize) -> String {
    let mut out = String::new();
    if files.is_empty() {
        out.push_str("Нет данных о возрасте (пустой репозиторий?).\n");
        return out;
    }

    let mut ranked: Vec<&FileAge> = files.iter().collect();
    ranked.sort_by(|a, b| {
        a.median_age_days
            .cmp(&b.median_age_days)
            .then_with(|| a.path.cmp(&b.path))
    });

    // Репо-сводка: медиана медиан (грубый ориентир «в среднем» возраст).
    let mut medians: Vec<i64> = files.iter().map(|f| f.median_age_days).collect();
    medians.sort_unstable();
    let repo_median = medians[medians.len() / 2];
    out.push_str(&format!(
        "файлов: {}; медиана median-возраста: {} дн.; пропущено blame: {}\n",
        files.len(),
        repo_median,
        blame_skipped
    ));
    out.push_str(&format!(
        "{:>7} {:>7} {:>7} {:>7}  {}\n",
        "newest", "median", "p90", "oldest", "file"
    ));
    for f in ranked.iter().take(top) {
        out.push_str(&format!(
            "{:>7} {:>7} {:>7} {:>7}  {}\n",
            f.newest_age_days, f.median_age_days, f.p90_age_days, f.oldest_age_days, f.path,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fa(path: &str, newest: i64, median: i64, p90: i64, oldest: i64) -> FileAge {
        FileAge {
            path: path.to_string(),
            lines: 10,
            newest_age_days: newest,
            median_age_days: median,
            p90_age_days: p90,
            oldest_age_days: oldest,
        }
    }

    #[test]
    fn render_sorts_by_median_youngest_first() {
        let files = vec![
            fa("stable.rs", 100, 300, 400, 500),
            fa("churned.rs", 0, 5, 20, 40),
        ];
        let out = render(&files, 10, 0);
        let p_churned = out.find("churned.rs").unwrap();
        let p_stable = out.find("stable.rs").unwrap();
        assert!(p_churned < p_stable, "малый median (переписываемый) сверху");
    }

    #[test]
    fn render_shows_summary_and_skipped() {
        let files = vec![fa("a.rs", 0, 10, 20, 30)];
        let out = render(&files, 10, 4);
        assert!(out.contains("пропущено blame: 4"));
        assert!(out.contains("median-возраста"));
    }

    #[test]
    fn render_empty_is_graceful() {
        let out = render(&[], 10, 0);
        assert!(out.contains("Нет данных"));
    }
}
