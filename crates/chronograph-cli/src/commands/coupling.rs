//! Команда `chronograph coupling` — таблица топ change-coupling пар в терминал.
//!
//! Как `hotspots`: инкрементально доводит кэш до HEAD, считает coupling
//! (co-occurrence по коммитам, исключая механические), печатает топ-N по
//! `coupling_ratio`. Без графики.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{compute_coupling, Coupling, CouplingConfig};
use chronograph_store::DuckStore;
use clap::Args;

const CACHE_REL_PATH: &str = ".chronograph/cache.duckdb";
/// Дефолт min_support — из примера ТЗ 3.4, подтверждён (см. CONTEXT.md).
const DEFAULT_MIN_SUPPORT: u32 = 5;

#[derive(Args)]
pub struct CouplingArgs {
    /// Путь к git-репозиторию (по умолчанию — текущая директория).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Путь к файлу кэша DuckDB (по умолчанию `<repo>/.chronograph/cache.duckdb`).
    #[arg(long = "db", value_name = "FILE")]
    pub db: Option<PathBuf>,

    /// Сколько строк показать.
    #[arg(long = "top", value_name = "N", default_value_t = 20)]
    pub top: usize,

    /// Минимальный support (совместных коммитов) для попадания пары в рейтинг.
    #[arg(long = "min-support", value_name = "N", default_value_t = DEFAULT_MIN_SUPPORT)]
    pub min_support: u32,

    /// Glob-паттерны исключаемых путей (vendored/generated). Можно повторять.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,
}

pub fn run(args: CouplingArgs) -> anyhow::Result<()> {
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

    let coupling_cfg = CouplingConfig {
        min_support: args.min_support,
        exclude_mechanical: true,
    };
    let pairs = compute_coupling(&store, &coupling_cfg).context("подсчёт change coupling")?;

    print!("{}", render(&pairs, args.top, args.min_support));
    Ok(())
}

/// Отрендерить таблицу топ-N пар (чистая функция — тестируется без БД).
fn render(pairs: &[Coupling], top: usize, min_support: u32) -> String {
    let mut out = String::new();
    if pairs.is_empty() {
        out.push_str(&format!(
            "Нет пар с support ≥ {min_support}. Попробуй меньший --min-support.\n"
        ));
        return out;
    }
    out.push_str(&format!(
        "{:>5} {:>6}  {:<34} {:<34}\n",
        "supp", "ratio", "file_a", "file_b"
    ));
    for c in pairs.iter().take(top) {
        out.push_str(&format!(
            "{:>5} {:>6.2}  {:<34} {:<34}\n",
            c.support,
            c.coupling_ratio,
            truncate(&c.path_a, 34),
            truncate(&c.path_b, 34),
        ));
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let tail: String = s.chars().skip(s.chars().count() - (max - 1)).collect();
        format!("…{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(a: &str, b: &str, support: u64, ratio: f64) -> Coupling {
        Coupling {
            path_a: a.to_string(),
            path_b: b.to_string(),
            support,
            coupling_ratio: ratio,
        }
    }

    #[test]
    fn render_limits_to_top_n_and_shows_rows() {
        let pairs = vec![
            pair("a.rs", "b.rs", 10, 0.9),
            pair("c.rs", "d.rs", 8, 0.7),
            pair("e.rs", "f.rs", 6, 0.5),
        ];
        let out = render(&pairs, 2, 5);
        assert!(out.contains("a.rs"));
        assert!(out.contains("b.rs"));
        assert!(out.contains("c.rs"));
        // top=2 → третья пара не показана.
        assert!(!out.contains("e.rs"));
        assert!(out.contains("0.90"));
    }

    #[test]
    fn render_empty_hints_min_support() {
        let out = render(&[], 20, 5);
        assert!(out.contains("min_support") || out.contains("support ≥ 5"));
    }

    #[test]
    fn truncate_shortens_long_paths() {
        let long = "src/very/deeply/nested/module/submodule/file_name.rs";
        let t = truncate(long, 20);
        assert_eq!(t.chars().count(), 20);
        assert!(t.starts_with('…'));
        assert!(t.ends_with("file_name.rs"));
    }
}
