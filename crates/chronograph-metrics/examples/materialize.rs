//! Демо: материализовать аналитические таблицы и прочитать их обратно.
//!
//! Запуск: `cargo run -p chronograph-metrics --example materialize -- <repo>`

use std::path::Path;

use chronograph_core::Config;
use chronograph_git::GitSource;
use chronograph_metrics::{materialize, MaterializeConfig};
use chronograph_store::DuckStore;

fn main() {
    let repo = std::env::args().nth(1).expect("usage: materialize <repo>");
    let cfg = Config::new(&repo);
    let reader = GitSource::open(&cfg).expect("открытие репо");
    let db = Path::new(&repo).join(".chronograph").join("cache.duckdb");
    let store = DuckStore::open(&db).expect("открытие кэша");

    let summary = materialize(&store, &reader, &MaterializeConfig::default()).expect("materialize");
    println!(
        "материализовано: file_metrics={}, coupling={}",
        summary.file_metrics_rows, summary.coupling_rows
    );

    println!("\n== file_metrics (топ по hotspot_rank) ==");
    println!(
        "{:<4} {:<28} {:>6} {:>4} {:>5}",
        "rank", "path", "churn", "cx", "alive"
    );
    let mut stmt = store
        .conn()
        .prepare(
            "SELECT hotspot_rank, path, churn_total, complexity, is_alive \
             FROM file_metrics WHERE hotspot_rank IS NOT NULL \
             ORDER BY hotspot_rank LIMIT 8",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (rank, path, churn, cx, alive) = row.unwrap();
        println!(
            "{rank:<4} {path:<28} {churn:>6} {:>4} {:>5}",
            cx as i64, alive
        );
    }

    println!("\n== coupling (топ по ratio) ==");
    println!(
        "{:>5} {:>6}  {:<26} {:<26}",
        "supp", "ratio", "file_a", "file_b"
    );
    let mut stmt = store
        .conn()
        .prepare(
            "SELECT support, coupling_ratio, path_a, path_b FROM coupling \
             ORDER BY coupling_ratio DESC, support DESC LIMIT 8",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (support, ratio, a, b) = row.unwrap();
        println!("{support:>5} {ratio:>6.2}  {a:<26} {b:<26}");
    }
}
