//! Интеграционные тесты материализации: `file_metrics` и `coupling` заполняются
//! корректно и детерминированно.

use std::path::Path;
use std::process::Command;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{materialize, MaterializeConfig};
use chronograph_store::DuckStore;
use duckdb::types::Value;
use tempfile::TempDir;

const FIXED_NOW: i64 = 1_700_000_000;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_AUTHOR_DATE", "2021-06-01T00:00:00 +0000")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_DATE", "2021-06-01T00:00:00 +0000")
        .status()
        .expect("git доступен");
    assert!(status.success(), "git {args:?} провалилась");
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

// a.rs — два if → cyclomatic 3; b.rs — один if → cyclomatic 2.
fn a_src(n: i32) -> String {
    format!("fn f(x: i32) -> i32 {{ if x > 0 {{ if x > {n} {{ return {n}; }} }} 0 }}")
}
fn b_src(n: i32) -> String {
    format!("fn g(x: i32) -> i32 {{ if x > {n} {{ return 1; }} 0 }}")
}

/// a.rs и b.rs меняются вместе 5 раз → support(a,b)=5 (== дефолтный min_support).
fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    for i in 1..=5 {
        write(p, "a.rs", &a_src(i));
        write(p, "b.rs", &b_src(i));
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", &format!("c{i}")]);
    }
    dir
}

fn setup(repo: &Path, db: &Path) -> DuckStore {
    let cfg = Config::new(repo);
    let src = GitSource::open(&cfg).unwrap();
    {
        let mut store = DuckStore::open(db).unwrap();
        run_analysis(&src, &mut store, &cfg, FIXED_NOW).unwrap();
    }
    let store = DuckStore::open(db).unwrap();
    materialize(&store, &src, &MaterializeConfig::default()).unwrap();
    store
}

fn scalar(store: &DuckStore, sql: &str) -> i64 {
    store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn file_metrics_populated() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let store = setup(repo.path(), &tmp.path().join("c.duckdb"));

    // Строки на a.rs и b.rs.
    assert_eq!(scalar(&store, "SELECT count(*) FROM file_metrics"), 2);

    // a.rs: churn_total=5, complexity=3, живой, ранг проставлен.
    let (churn, cx, alive, rank_is_null): (i64, f64, bool, bool) = store
        .conn()
        .query_row(
            "SELECT churn_total, complexity, is_alive, hotspot_rank IS NULL \
             FROM file_metrics WHERE path='a.rs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(churn, 5);
    assert_eq!(cx, 3.0);
    assert!(alive);
    assert!(!rank_is_null, "hotspot_rank должен быть проставлен");

    // b.rs: complexity=2.
    assert_eq!(
        scalar(
            &store,
            "SELECT count(*) FROM file_metrics WHERE path='b.rs' AND complexity=2.0"
        ),
        1
    );
}

#[test]
fn coupling_populated() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let store = setup(repo.path(), &tmp.path().join("c.duckdb"));

    // Пара (a.rs, b.rs): support 5, ratio 1.0, explained_by_imports NULL.
    let (support, ratio, imports_is_null): (i64, f64, bool) = store
        .conn()
        .query_row(
            "SELECT support, coupling_ratio, explained_by_imports IS NULL \
             FROM coupling WHERE path_a='a.rs' AND path_b='b.rs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(support, 5);
    assert!((ratio - 1.0).abs() < 1e-9);
    assert!(imports_is_null, "explained_by_imports пока NULL (backlog)");
}

#[test]
fn knowledge_populated() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let store = setup(repo.path(), &tmp.path().join("c.duckdb"));

    // Один автор фикстуры → каждый файл принадлежит ему целиком.
    // module_bus_factor: 2 файла, все bus_factor=1, top_owner_ratio=1.0.
    assert_eq!(scalar(&store, "SELECT count(*) FROM module_bus_factor"), 2);
    assert_eq!(
        scalar(
            &store,
            "SELECT count(*) FROM module_bus_factor WHERE bus_factor=1 AND top_owner_ratio=1.0"
        ),
        2
    );
    // knowledge: по одной паре (файл, автор) на файл.
    assert_eq!(scalar(&store, "SELECT count(*) FROM knowledge"), 2);
    assert_eq!(
        scalar(&store, "SELECT count(DISTINCT author_id) FROM knowledge"),
        1
    );
    // knowledge_meta: счётчик пропущенных blame (фикстура ничего не роняет → 0).
    assert_eq!(
        scalar(&store, "SELECT blame_skipped FROM knowledge_meta"),
        0
    );
}

#[test]
fn file_age_populated_from_shared_blame() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let store = setup(repo.path(), &tmp.path().join("c.duckdb"));

    // 2 файла. Все коммиты фикстуры на одну дату → anchor == время коммитов →
    // возраст всех строк 0 (median/newest/oldest = 0).
    assert_eq!(scalar(&store, "SELECT count(*) FROM file_age"), 2);
    assert_eq!(
        scalar(
            &store,
            "SELECT count(*) FROM file_age \
             WHERE newest_age_days=0 AND median_age_days=0 AND oldest_age_days=0"
        ),
        2
    );
    // lines проставлены (a.rs — одна строка кода).
    assert!(scalar(&store, "SELECT sum(lines) FROM file_age") >= 2);
}

#[test]
fn materialize_is_idempotent() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("c.duckdb");
    let store = setup(repo.path(), &db);
    // Повторная материализация не задваивает строки.
    let src = GitSource::open(&Config::new(repo.path())).unwrap();
    materialize(&store, &src, &MaterializeConfig::default()).unwrap();
    assert_eq!(scalar(&store, "SELECT count(*) FROM file_metrics"), 2);
    assert_eq!(scalar(&store, "SELECT count(*) FROM coupling"), 1);
    assert_eq!(scalar(&store, "SELECT count(*) FROM knowledge"), 2);
    assert_eq!(scalar(&store, "SELECT count(*) FROM module_bus_factor"), 2);
    assert_eq!(scalar(&store, "SELECT count(*) FROM file_age"), 2);
}

#[test]
fn materialization_is_deterministic() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let store_a = setup(repo.path(), &tmp.path().join("a.duckdb"));
    let store_b = setup(repo.path(), &tmp.path().join("b.duckdb"));
    assert_eq!(dump(&store_a), dump(&store_b));
}

/// Детерминированный дамп аналитических таблиц.
fn dump(store: &DuckStore) -> String {
    let mut out = String::new();
    for (table, order, ncols) in [
        ("file_metrics", "path", 9usize),
        ("coupling", "path_a, path_b", 5),
        ("knowledge", "path, author_id", 3),
        ("module_bus_factor", "module", 3),
        ("knowledge_meta", "blame_skipped", 1),
        ("file_age", "path", 6),
    ] {
        out.push_str(&format!("== {table} ==\n"));
        let sql = format!("SELECT * FROM {table} ORDER BY {order}");
        let mut stmt = store.conn().prepare(&sql).unwrap();
        let rows = stmt
            .query_map([], |row| {
                let mut line = String::new();
                for i in 0..ncols {
                    let v: Value = row.get(i)?;
                    line.push_str(&format!("{v:?}|"));
                }
                Ok(line)
            })
            .unwrap();
        for r in rows {
            out.push_str(&r.unwrap());
            out.push('\n');
        }
    }
    out
}
