//! Интеграционные тесты code age на синтетическом репозитории с КОНТРОЛИРУЕМЫМИ
//! датами коммитов → заранее известные возрасты строк. Настоящий gix blame.

use std::path::Path;
use std::process::Command;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{compute_age, FileAge};
use chronograph_store::DuckStore;
use tempfile::TempDir;

const FIXED_NOW: i64 = 1_700_000_000;

/// git-команда с явной датой коммита (и автор, и коммиттер — движок берёт время
/// коммиттера в `committed_at`).
fn git_at(dir: &Path, date: &str, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fix@example.com")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fix@example.com")
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git доступен");
    assert!(status.success(), "git {args:?} провалилась");
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

/// Репозиторий с известными возрастами. Коммиты:
/// c1 (2020-01-01): f.rs (4 строки), g.rs (2 строки);
/// c2 (2020-01-11, +10 дней): меняет строку 2 в f.rs.
///
/// anchor = max(committed_at) = c2 = 2020-01-11. blame f.rs на HEAD: строки 1/3/4 →
/// c1 (возраст 10д), строка 2 → c2 (возраст 0д). g.rs не менялся → обе строки 10д.
fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git_at(
        p,
        "2020-01-01T00:00:00 +0000",
        &["init", "-q", "-b", "main"],
    );

    write(p, "f.rs", "l1\nl2\nl3\nl4\n");
    write(p, "g.rs", "a1\na2\n");
    git_at(p, "2020-01-01T00:00:00 +0000", &["add", "."]);
    git_at(
        p,
        "2020-01-01T00:00:00 +0000",
        &["commit", "-q", "-m", "c1"],
    );

    write(p, "f.rs", "l1\nCHANGED\nl3\nl4\n");
    git_at(
        p,
        "2020-01-11T00:00:00 +0000",
        &["commit", "-q", "-am", "c2"],
    );

    dir
}

fn setup(repo: &Path, db: &Path) -> (DuckStore, GitSource) {
    let cfg = Config::new(repo);
    let src = GitSource::open(&cfg).unwrap();
    {
        let mut store = DuckStore::open(db).unwrap();
        run_analysis(&src, &mut store, &cfg, FIXED_NOW).unwrap();
    }
    (DuckStore::open(db).unwrap(), src)
}

fn find<'a>(rows: &'a [FileAge], path: &str) -> &'a FileAge {
    rows.iter().find(|r| r.path == path).expect("файл есть")
}

#[test]
fn age_matches_known_commit_dates() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, blamer) = setup(repo.path(), &tmp.path().join("a.duckdb"));

    let report = compute_age(&store, &blamer, 0).unwrap();
    assert_eq!(report.blame_skipped, 0);

    // f.rs: возрасты [0,10,10,10] → newest 0, median 10, p90 10, oldest 10.
    let f = find(&report.files, "f.rs");
    assert_eq!(f.lines, 4);
    assert_eq!(f.newest_age_days, 0, "строка от c2 — свежая");
    assert_eq!(f.oldest_age_days, 10, "строки от c1 — 10 дней");
    assert_eq!(f.median_age_days, 10);
    assert_eq!(f.p90_age_days, 10);

    // g.rs: обе строки от c1 → всё 10 дней.
    let g = find(&report.files, "g.rs");
    assert_eq!(g.lines, 2);
    assert_eq!(g.newest_age_days, 10);
    assert_eq!(g.oldest_age_days, 10);
    assert_eq!(g.median_age_days, 10);
}

#[test]
fn age_percentiles_are_ordered() {
    // Property: newest ≤ median ≤ p90 ≤ oldest, возраст ≥ 0.
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, blamer) = setup(repo.path(), &tmp.path().join("a.duckdb"));

    let report = compute_age(&store, &blamer, 0).unwrap();
    assert!(!report.files.is_empty());
    for fa in &report.files {
        assert!(fa.newest_age_days >= 0, "{}: возраст ≥ 0", fa.path);
        assert!(
            fa.newest_age_days <= fa.median_age_days,
            "{}: newest ≤ median",
            fa.path
        );
        assert!(
            fa.median_age_days <= fa.p90_age_days,
            "{}: median ≤ p90",
            fa.path
        );
        assert!(
            fa.p90_age_days <= fa.oldest_age_days,
            "{}: p90 ≤ oldest",
            fa.path
        );
    }
}

#[test]
fn age_is_deterministic() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, blamer) = setup(repo.path(), &tmp.path().join("a.duckdb"));

    let a = compute_age(&store, &blamer, 0).unwrap();
    let b = compute_age(&store, &blamer, 0).unwrap();
    assert_eq!(a, b);
}
