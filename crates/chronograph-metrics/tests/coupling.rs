//! Интеграционные тесты change coupling на синтетическом репо с известными
//! со-изменениями. Включает обязательный property-тест симметрии (CLAUDE.md).

use std::path::Path;
use std::process::Command;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{compute_coupling, Coupling, CouplingConfig};
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

fn touch(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

fn commit(dir: &Path, msg: &str) {
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", msg]);
}

/// История со-изменений:
/// - (a,b) вместе в c1,c2,c3 → support 3
/// - a один в c4; b один в c5
/// - (a,c) вместе в c6 → support 1
/// - c7: «механический» коммит (a + g0..g5 = 7 файлов)
///
/// commits(a)=5, commits(b)=4, commits(c)=1 (без механических).
fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);

    for i in 1..=3 {
        touch(p, "a.rs", &format!("a{i}"));
        touch(p, "b.rs", &format!("b{i}"));
        commit(p, &format!("c{i}"));
    }
    touch(p, "a.rs", "a4");
    commit(p, "c4-a-alone");
    touch(p, "b.rs", "b5");
    commit(p, "c5-b-alone");
    touch(p, "a.rs", "a6");
    touch(p, "c.rs", "c6");
    commit(p, "c6-a-c");

    // Механический коммит: a + шесть g-файлов (7 > порога 5).
    touch(p, "a.rs", "a7");
    for i in 0..6 {
        touch(p, &format!("g{i}.rs"), "g");
    }
    commit(p, "c7-mechanical");

    dir
}

fn analyze_and_couple(repo: &Path, mech_max: Option<u32>, cfg: &CouplingConfig) -> Vec<Coupling> {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("cache.duckdb");
    let mut acfg = Config::new(repo);
    acfg.mechanical_commit_max_files = mech_max;
    let src = GitSource::open(&acfg).unwrap();
    {
        let mut store = DuckStore::open(&db).unwrap();
        run_analysis(&src, &mut store, &acfg, FIXED_NOW).unwrap();
    }
    let store = DuckStore::open(&db).unwrap();
    // db temp живёт до конца функции.
    let out = compute_coupling(&store, cfg).unwrap();
    drop(store);
    out
}

/// support пары независимо от порядка аргументов.
fn support_of(rows: &[Coupling], x: &str, y: &str) -> Option<u64> {
    let (a, b) = if x < y { (x, y) } else { (y, x) };
    rows.iter()
        .find(|c| c.path_a == a && c.path_b == b)
        .map(|c| c.support)
}

fn ratio_of(rows: &[Coupling], x: &str, y: &str) -> Option<f64> {
    let (a, b) = if x < y { (x, y) } else { (y, x) };
    rows.iter()
        .find(|c| c.path_a == a && c.path_b == b)
        .map(|c| c.coupling_ratio)
}

fn cfg(min_support: u32, exclude_mechanical: bool) -> CouplingConfig {
    CouplingConfig {
        min_support,
        exclude_mechanical,
    }
}

#[test]
fn support_and_ratio_are_correct() {
    let dir = build_fixture();
    let rows = analyze_and_couple(dir.path(), Some(5), &cfg(1, true));

    // (a,b): support 3, ratio 3/min(5,4) = 0.75.
    assert_eq!(support_of(&rows, "a.rs", "b.rs"), Some(3));
    assert!((ratio_of(&rows, "a.rs", "b.rs").unwrap() - 0.75).abs() < 1e-9);

    // (a,c): support 1, ratio 1/min(5,1) = 1.0 (c меняется только с a).
    assert_eq!(support_of(&rows, "a.rs", "c.rs"), Some(1));
    assert!((ratio_of(&rows, "a.rs", "c.rs").unwrap() - 1.0).abs() < 1e-9);

    // (b,c) никогда вместе → пары нет.
    assert_eq!(support_of(&rows, "b.rs", "c.rs"), None);
}

#[test]
fn coupling_is_symmetric() {
    // Обязательный property-тест CLAUDE.md: coupling(A,B) == coupling(B,A).
    let dir = build_fixture();
    let rows = analyze_and_couple(dir.path(), Some(5), &cfg(1, true));

    assert_eq!(
        support_of(&rows, "a.rs", "b.rs"),
        support_of(&rows, "b.rs", "a.rs")
    );
    assert_eq!(
        ratio_of(&rows, "a.rs", "b.rs"),
        ratio_of(&rows, "b.rs", "a.rs")
    );
    // Пара хранится ровно один раз, канонически (path_a < path_b).
    let n = rows
        .iter()
        .filter(|c| {
            (c.path_a == "a.rs" && c.path_b == "b.rs") || (c.path_a == "b.rs" && c.path_b == "a.rs")
        })
        .count();
    assert_eq!(n, 1);
    assert!(rows.iter().all(|c| c.path_a < c.path_b));
}

#[test]
fn min_support_filters_weak_pairs() {
    let dir = build_fixture();
    // min_support=2 → (a,c) с support 1 отсекается, (a,b) с support 3 остаётся.
    let rows = analyze_and_couple(dir.path(), Some(5), &cfg(2, true));
    assert_eq!(support_of(&rows, "a.rs", "b.rs"), Some(3));
    assert_eq!(support_of(&rows, "a.rs", "c.rs"), None);
}

#[test]
fn mechanical_commits_are_excluded() {
    let dir = build_fixture();
    let rows = analyze_and_couple(dir.path(), Some(5), &cfg(1, true));
    // g-файлы были только в механическом коммите → никаких пар с ними.
    assert!(
        rows.iter()
            .all(|c| !c.path_a.starts_with("g") && !c.path_b.starts_with("g")),
        "механический коммит не должен порождать coupling-пары"
    );
    // И commits(a) не раздут механическим коммитом: ratio(a,b) остался 0.75.
    assert!((ratio_of(&rows, "a.rs", "b.rs").unwrap() - 0.75).abs() < 1e-9);
}

#[test]
fn mechanical_filter_can_be_disabled() {
    let dir = build_fixture();
    // exclude_mechanical=false → механический коммит учитывается, g-пары появляются.
    let rows = analyze_and_couple(dir.path(), Some(5), &cfg(1, false));
    assert!(
        rows.iter()
            .any(|c| c.path_a.starts_with("g") || c.path_b.starts_with("g")),
        "с выключенным фильтром механический коммит даёт пары"
    );
}

#[test]
fn coupling_is_deterministic() {
    let dir = build_fixture();
    let a = analyze_and_couple(dir.path(), Some(5), &cfg(1, true));
    let b = analyze_and_couple(dir.path(), Some(5), &cfg(1, true));
    assert_eq!(a, b);
}
