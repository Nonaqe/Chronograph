//! Дев-утилита: выполнить произвольный SQL над кэшем и напечатать строки.
//!
//! Запуск: `cargo run -p chronograph-metrics --example sql -- <cache.duckdb> "<SQL>"`

use chronograph_store::DuckStore;
use duckdb::types::Value;

fn main() {
    let db = std::env::args().nth(1).expect("usage: sql <db> <sql>");
    let sql = std::env::args().nth(2).expect("usage: sql <db> <sql>");
    let store = DuckStore::open(&db).expect("open");
    let conn = store.conn();
    let mut stmt = conn.prepare(&sql).expect("prepare");
    let mut rows = stmt.query([]).expect("query");
    while let Some(row) = rows.next().expect("row") {
        let mut line = String::new();
        let mut i = 0;
        while let Ok(v) = row.get::<_, Value>(i) {
            line.push_str(&format!("{v:?}\t"));
            i += 1;
        }
        println!("{line}");
    }
}
