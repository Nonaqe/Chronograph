//! Демо-утилита: вывести топ файлов по churn из готового кэша.
//!
//! Запуск: `cargo run -p chronograph-metrics --example churn -- <cache.duckdb>`

use chronograph_metrics::{compute_churn, ChurnConfig};
use chronograph_store::DuckStore;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("использование: churn <путь к cache.duckdb>");
    let store = DuckStore::open(&path).expect("открытие кэша");
    let cfg = ChurnConfig::default();
    let mut rows = compute_churn(&store, &cfg).expect("подсчёт churn");

    // Топ по общему churn; стабильный tie-break по пути.
    rows.sort_by(|a, b| b.churn_total.cmp(&a.churn_total).then(a.path.cmp(&b.path)));

    println!(
        "{:<55} {:>6} {:>6} {:>6} {:>6} {:>5}",
        "path",
        "total",
        format!("{}d", cfg.window_long_days),
        format!("{}d", cfg.window_mid_days),
        format!("{}d", cfg.window_recent_days),
        "alive",
    );
    for r in rows.iter().take(30) {
        println!(
            "{:<55} {:>6} {:>6} {:>6} {:>6} {:>5}",
            truncate(&r.path, 55),
            r.churn_total,
            r.churn_long,
            r.churn_mid,
            r.churn_recent,
            if r.is_alive { "yes" } else { "no" },
        );
    }
    println!("\nвсего файлов: {}", rows.len());
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("…{}", &s[s.len() - (max - 1)..])
    }
}
