//! Профайлер: грубый time-breakdown по метрикам на одном репозитории.
//!
//! Запуск: `cargo run -p chronograph-metrics --example profile -- <repo>`
//!
//! Открывает репо, гонит analyze, затем тайминит КАЖДУЮ метрику отдельно на уже
//! построенном сторе. Цель — показать, куда реально уходит время (не угадывать).

use std::path::Path;
use std::time::Instant;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{
    compute_churn, compute_complexity, compute_coupling, compute_knowledge, ChurnConfig,
    CouplingConfig, KnowledgeConfig,
};
use chronograph_store::DuckStore;

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn main() {
    let repo = std::env::args().nth(1).expect("usage: profile <repo>");
    let cfg = Config::new(&repo);
    let src = GitSource::open(&cfg).expect("открытие репо");
    let db = Path::new(&repo).join(".chronograph").join("profile.duckdb");
    std::fs::create_dir_all(db.parent().unwrap()).ok();
    // Свежий кэш для чистоты замера (complexity-кэш и т.п. не «подыгрывают»).
    let _ = std::fs::remove_file(&db);

    let t = Instant::now();
    {
        let mut store = DuckStore::open(&db).expect("кэш");
        run_analysis(&src, &mut store, &cfg, now_unix()).expect("analyze");
    }
    let t_analyze = t.elapsed();
    let store = DuckStore::open(&db).expect("кэш");

    let commits: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM commits", [], |r| r.get(0))
        .unwrap();
    let files: i64 = store
        .conn()
        .query_row("SELECT count(DISTINCT path) FROM file_changes", [], |r| {
            r.get(0)
        })
        .unwrap();
    println!("репо: {commits} коммитов, {files} путей (file_changes)");
    println!("{:<14} {:>10}  строк", "стадия", "секунды");

    macro_rules! timeit {
        ($name:expr, $body:expr) => {{
            let t = Instant::now();
            let n = $body;
            println!("{:<14} {:>10.2}  {}", $name, t.elapsed().as_secs_f64(), n);
        }};
    }

    println!("{:<14} {:>10.2}", "analyze", t_analyze.as_secs_f64());
    timeit!(
        "churn",
        compute_churn(&store, &ChurnConfig::default())
            .unwrap()
            .len()
    );
    timeit!(
        "complexity",
        compute_complexity(&store, &src).unwrap().len()
    );
    timeit!(
        "coupling",
        compute_coupling(&store, &CouplingConfig::default())
            .unwrap()
            .len()
    );
    timeit!(
        "knowledge",
        compute_knowledge(&store, &src, &KnowledgeConfig::default())
            .unwrap()
            .files
            .len()
    );
}
