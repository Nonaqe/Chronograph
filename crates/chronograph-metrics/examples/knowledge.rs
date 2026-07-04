//! Демо: knowledge / bus factor — риск концентрации знаний по файлам.
//!
//! Запуск: `cargo run -p chronograph-metrics --example knowledge -- <repo> [top]`
//!
//! Подаётся как РИСК (принцип 2.4): файлы с bus_factor = 1 наверху. Имена авторов
//! показаны для глазной валидации на dev-репо; в реальном HTML-отчёте (Этап 3/4)
//! применяется анонимизация (Author #N).

use std::collections::HashMap;
use std::path::Path;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{compute_knowledge, KnowledgeConfig};
use chronograph_store::DuckStore;

fn main() {
    let repo = std::env::args()
        .nth(1)
        .expect("usage: knowledge <repo> [top]");
    let top: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    // Полный self-contained прогон: analyze (инкрементально) → knowledge.
    let cfg = Config::new(&repo);
    let src = GitSource::open(&cfg).expect("открытие репо");
    let db = Path::new(&repo).join(".chronograph").join("cache.duckdb");
    std::fs::create_dir_all(db.parent().unwrap()).ok();
    {
        let mut store = DuckStore::open(&db).expect("открытие кэша");
        run_analysis(&src, &mut store, &cfg, now_unix()).expect("analyze");
    }
    let store = DuckStore::open(&db).expect("переоткрытие кэша");

    let names = author_names(&store);
    let report = compute_knowledge(&store, &src, &KnowledgeConfig::default()).expect("knowledge");
    let rows = report.files;

    // Риск-вид: сначала bus_factor возр. (1 = риск), затем top_owner_ratio убыв.
    let mut ranked = rows.clone();
    ranked.sort_by(|a, b| {
        a.bus_factor
            .cmp(&b.bus_factor)
            .then(b.top_owner_ratio.partial_cmp(&a.top_owner_ratio).unwrap())
            .then(a.path.cmp(&b.path))
    });

    let bus1 = rows.iter().filter(|r| r.bus_factor == 1).count();
    println!(
        "файлов: {}; с bus_factor = 1 (риск концентрации): {}; пропущено blame: {}",
        rows.len(),
        bus1,
        report.blame_skipped
    );
    println!("{:>4} {:>6}  {:<24} file", "bf", "top%", "top owner");
    for fk in ranked.iter().take(top) {
        let owner = names
            .get(&fk.owners[0].author_id)
            .cloned()
            .unwrap_or_else(|| "?".into());
        println!(
            "{:>4} {:>6.0}  {:<24} {}",
            fk.bus_factor,
            fk.top_owner_ratio * 100.0,
            trunc(&owner, 24),
            fk.path
        );
    }
}

fn author_names(store: &DuckStore) -> HashMap<i64, String> {
    let mut stmt = store
        .conn()
        .prepare("SELECT author_id, canonical_name FROM authors")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let tail: String = s.chars().skip(s.chars().count() - (max - 1)).collect();
        format!("…{tail}")
    }
}
