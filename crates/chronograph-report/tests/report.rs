//! End-to-end тест отчёта: полный путь analyze → materialize → render.
//! Главное — байт-идентичность HTML двух прогонов (детерминизм, правило 4).

use std::path::Path;
use std::process::Command;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{materialize, MaterializeConfig};
use chronograph_report::generate;
use chronograph_store::DuckStore;
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

fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    for i in 1..=6 {
        std::fs::write(
            p.join("a.rs"),
            format!("fn f(x:i32)->i32{{ if x>0 {{ if x>{i} {{return {i};}} }} 0 }}"),
        )
        .unwrap();
        std::fs::write(
            p.join("b.rs"),
            format!("fn g(x:i32)->i32{{ if x>{i} {{return 1;}} 0 }}"),
        )
        .unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", &format!("c{i}")]);
    }
    dir
}

/// Полный путь: analyze → materialize → generate → вернуть содержимое HTML.
fn build_report(repo: &Path, workdir: &Path) -> String {
    let db = workdir.join("cache.duckdb");
    let out = workdir.join("report.html");
    let cfg = Config::new(repo);
    let src = GitSource::open(&cfg).unwrap();
    {
        let mut store = DuckStore::open(&db).unwrap();
        run_analysis(&src, &mut store, &cfg, FIXED_NOW).unwrap();
    }
    let store = DuckStore::open(&db).unwrap();
    materialize(&store, &src, &MaterializeConfig::default()).unwrap();
    generate(&store, &out).unwrap();
    std::fs::read_to_string(&out).unwrap()
}

#[test]
fn report_is_byte_identical_across_runs() {
    let repo = build_fixture();
    let w1 = TempDir::new().unwrap();
    let w2 = TempDir::new().unwrap();
    let html_a = build_report(repo.path(), w1.path());
    let html_b = build_report(repo.path(), w2.path());
    assert_eq!(html_a, html_b, "HTML должен быть байт-в-байт идентичен");
}

#[test]
fn report_is_self_contained_and_has_content() {
    let repo = build_fixture();
    let w = TempDir::new().unwrap();
    let html = build_report(repo.path(), w.path());

    // Self-contained: стиль инлайн, без внешних ресурсов/JS.
    assert!(html.contains("<style>"));
    assert!(!html.contains("<script"));
    assert!(!html.contains("src=\"http"));
    assert!(!html.contains("href=\"http"));

    // Есть содержимое: treemap SVG, файлы, секция coupling.
    assert!(html.contains("<svg"));
    assert!(html.contains("a.rs"));
    assert!(html.contains("Change coupling"));
    // Пара a.rs↔b.rs (support 6 ≥ min_support 5) в таблице.
    assert!(html.contains("b.rs"));
}
