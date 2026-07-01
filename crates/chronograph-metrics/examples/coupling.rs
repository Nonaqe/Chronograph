//! Демо: топ change-coupling пар из готового кэша.
//!
//! Запуск: `cargo run -p chronograph-metrics --example coupling -- <repo> [min_support]`

use std::path::Path;

use chronograph_metrics::{compute_coupling, CouplingConfig};
use chronograph_store::DuckStore;

fn main() {
    let repo = std::env::args()
        .nth(1)
        .expect("usage: coupling <repo> [min_support]");
    let min_support: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    let db = Path::new(&repo).join(".chronograph").join("cache.duckdb");
    let store = DuckStore::open(&db).expect("открытие кэша");
    let cfg = CouplingConfig {
        min_support,
        exclude_mechanical: true,
    };
    let rows = compute_coupling(&store, &cfg).expect("coupling");

    println!("min_support = {min_support}; всего пар: {}", rows.len());
    println!(
        "{:>5} {:>6}  {:<32} {:<32}",
        "supp", "ratio", "file_a", "file_b"
    );
    for c in rows.iter().take(25) {
        println!(
            "{:>5} {:>6.2}  {:<32} {:<32}",
            c.support,
            c.coupling_ratio,
            trunc(&c.path_a, 32),
            trunc(&c.path_b, 32),
        );
    }
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let tail: String = s.chars().skip(s.chars().count() - (max - 1)).collect();
        format!("…{tail}")
    }
}
