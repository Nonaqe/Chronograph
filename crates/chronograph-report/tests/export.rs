//! End-to-end тесты JSON-экспорта: analyze → materialize → export_json.
//!
//! Главное: (1) БАЙТ-идентичность двух прогонов (правило 4 — экспорт это артефакт,
//! в отличие от живого рендера web/); (2) insta-снапшот — схема не ломается
//! незаметно (ТЗ §10); (3) анонимизация по умолчанию (принцип 2.4); (4) поток
//! событий несёт rename с old_path (нужно Gource-анимации для перемещения узлов).

use std::path::Path;
use std::process::Command;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{materialize, MaterializeConfig};
use chronograph_report::{export_json, ExportOptions};
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

/// Фикстура как в report.rs (6 коммитов a.rs+b.rs → coupling support 6) плюс
/// седьмой коммит — чистый rename b.rs→c.rs (событие 'R' с old_path).
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
    git(p, &["mv", "b.rs", "c.rs"]);
    git(p, &["commit", "-q", "-m", "rename b to c"]);
    dir
}

/// Полный путь: analyze → materialize → export_json.
fn build_export(repo: &Path, workdir: &Path, opts: &ExportOptions) -> String {
    let db = workdir.join("cache.duckdb");
    let cfg = Config::new(repo);
    let src = GitSource::open(&cfg).unwrap();
    {
        let mut store = DuckStore::open(&db).unwrap();
        run_analysis(&src, &mut store, &cfg, FIXED_NOW).unwrap();
    }
    let store = DuckStore::open(&db).unwrap();
    materialize(&store, &src, &MaterializeConfig::default()).unwrap();
    export_json(&store, opts).unwrap()
}

#[test]
fn export_is_byte_identical_across_runs() {
    let repo = build_fixture();
    let w1 = TempDir::new().unwrap();
    let w2 = TempDir::new().unwrap();
    let a = build_export(repo.path(), w1.path(), &ExportOptions::default());
    let b = build_export(repo.path(), w2.path(), &ExportOptions::default());
    assert_eq!(a, b, "JSON-экспорт должен быть байт-в-байт идентичен");
}

#[test]
fn export_schema_snapshot() {
    let repo = build_fixture();
    let w = TempDir::new().unwrap();
    let json = build_export(repo.path(), w.path(), &ExportOptions::default());
    // Снапшот — pretty-печать того же документа: читаемый диф при изменении схемы.
    // Стабилен: фиксированные даты/автор фикстуры → детерминированные sha;
    // config_hash не включает repo_path (проверено в core).
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    insta::assert_snapshot!(serde_json::to_string_pretty(&value).unwrap());
}

#[test]
fn export_is_anonymous_by_default_and_shows_names_on_optin() {
    let repo = build_fixture();
    let w1 = TempDir::new().unwrap();
    let w2 = TempDir::new().unwrap();

    let anon = build_export(repo.path(), w1.path(), &ExportOptions::default());
    assert!(anon.contains("\"Author #1\""));
    assert!(
        !anon.contains("Fixture Author"),
        "имя автора не должно попадать в анонимный экспорт"
    );
    assert!(anon.contains("\"anonymized\":true"));

    let named = build_export(repo.path(), w2.path(), &ExportOptions { show_names: true });
    assert!(named.contains("Fixture Author"));
    assert!(named.contains("\"anonymized\":false"));
}

#[test]
fn export_events_carry_rename_and_are_chronological() {
    let repo = build_fixture();
    let w = TempDir::new().unwrap();
    let json = build_export(repo.path(), w.path(), &ExportOptions::default());
    let doc: serde_json::Value = serde_json::from_str(&json).unwrap();

    let events = doc["events"].as_array().unwrap();
    assert_eq!(events.len(), 7, "6 коммитов контента + 1 rename");

    // Хронологический порядок (неубывающие ts; в фикстуре все даты равны).
    let ts: Vec<i64> = events.iter().map(|e| e["ts"].as_i64().unwrap()).collect();
    assert!(ts.windows(2).all(|w| w[0] <= w[1]));

    // Rename-событие: type R, old_path заполнен (нужно анимации для переноса узла).
    let rename = events
        .iter()
        .flat_map(|e| e["changes"].as_array().unwrap())
        .find(|c| c["type"] == "R")
        .expect("событие rename присутствует");
    assert_eq!(rename["path"], "c.rs");
    assert_eq!(rename["old_path"], "b.rs");

    // Метрики на месте: coupling a.rs↔c.rs (канонические пути), files не пуст.
    assert!(!doc["files"].as_array().unwrap().is_empty());
    let coupling = doc["coupling"].as_array().unwrap();
    assert!(coupling
        .iter()
        .any(|p| p["a"] == "a.rs" && p["b"] == "c.rs" && p["support"].as_i64().unwrap() >= 5));

    // Мета: полный head_sha (40 hex), версия схемы.
    assert_eq!(doc["meta"]["schema_version"], 1);
    assert_eq!(doc["meta"]["head_sha"].as_str().unwrap().len(), 40);
    assert_eq!(doc["meta"]["total_commits"], 7);
}
