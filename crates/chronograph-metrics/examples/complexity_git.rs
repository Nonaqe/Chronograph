//! Демо: complexity из git-объектов на HEAD (не с диска).
//!
//! Запуск: `cargo run -p chronograph-metrics --example complexity_git -- <repo>`
//! (репозиторий должен быть уже проанализирован — есть `.chronograph/cache.duckdb`).

use std::path::Path;

use chronograph_core::Config;
use chronograph_git::GitSource;
use chronograph_lang::ComplexityMethod;
use chronograph_metrics::compute_complexity;
use chronograph_store::DuckStore;

fn main() {
    let repo = std::env::args()
        .nth(1)
        .expect("usage: complexity_git <repo>");
    let cfg = Config::new(&repo);
    let reader = GitSource::open(&cfg).expect("открытие репо");
    let db = Path::new(&repo).join(".chronograph").join("cache.duckdb");
    let store = DuckStore::open(&db).expect("открытие кэша");

    let mut rows = compute_complexity(&store, &reader).expect("complexity");
    rows.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap()
            .then(a.path.cmp(&b.path))
    });

    println!(
        "{:<32} {:<11} {:>6} {:>5} {:>8}",
        "path", "method", "value", "loc", "per_loc"
    );
    for r in rows.iter().take(20) {
        let m = match r.method {
            ComplexityMethod::Cyclomatic => "cyclomatic",
            ComplexityMethod::Indentation => "indent",
        };
        println!(
            "{:<32} {:<11} {:>6} {:>5} {:>8.3}",
            r.path, m, r.value, r.loc, r.per_loc
        );
    }
    println!("\nвсего живых файлов: {}", rows.len());
}
