//! Демо: посчитать complexity для переданных файлов.
//!
//! Запуск: `cargo run -p chronograph-lang --example complexity -- <file>...`

use chronograph_lang::{file_complexity, ComplexityMethod};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    println!(
        "{:<45} {:<11} {:>6} {:>5} {:>8}",
        "path", "method", "value", "loc", "per_loc"
    );
    for path in paths {
        let Ok(src) = std::fs::read(&path) else {
            eprintln!("skip (не прочитан): {path}");
            continue;
        };
        let fc = file_complexity(&path, &src);
        let method = match fc.method {
            ComplexityMethod::Cyclomatic => "cyclomatic",
            ComplexityMethod::Indentation => "indent",
        };
        println!(
            "{:<45} {:<11} {:>6} {:>5} {:>8.3}",
            path, method, fc.value, fc.loc, fc.per_loc
        );
    }
}
