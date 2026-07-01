//! Интеграционные тесты complexity: контент берётся из git-объектов (не с диска),
//! кэшируется по blob_sha, склеивается по каноническому пути.

use std::path::Path;
use std::process::Command;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_lang::{ComplexityMethod, SupportedLanguage};
use chronograph_metrics::{compute_complexity, FileComplexityRow};
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

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

// Rust-файл с ровно тремя if → cyclomatic 4.
const CALC_RS: &str = "fn f(x: i32) -> i32 {\n\
    if x > 0 {\n\
        if x > 10 { return 10; }\n\
    }\n\
    if x < 0 { return -1; }\n\
    x\n\
}\n";

// Текстовый файл (fallback): глубины 0+1+2 = 3.
const NOTES_TXT: &str = "a\n  b\n    c\n";

fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    write(p, "calc.rs", CALC_RS);
    write(p, "notes.txt", NOTES_TXT);
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "c1"]);
    dir
}

/// Прогнать analyze в БД и вернуть (store, git-источник как BlobReader).
fn setup(repo: &Path, db: &Path) -> (DuckStore, GitSource) {
    let cfg = Config::new(repo);
    let src = GitSource::open(&cfg).unwrap();
    {
        let mut store = DuckStore::open(db).unwrap();
        run_analysis(&src, &mut store, &cfg, FIXED_NOW).unwrap();
    }
    (DuckStore::open(db).unwrap(), src)
}

fn find<'a>(rows: &'a [FileComplexityRow], path: &str) -> &'a FileComplexityRow {
    rows.iter().find(|r| r.path == path).expect("файл есть")
}

#[test]
fn computes_cyclomatic_and_fallback() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, reader) = setup(repo.path(), &tmp.path().join("c.duckdb"));

    let rows = compute_complexity(&store, &reader).unwrap();

    let calc = find(&rows, "calc.rs");
    assert_eq!(calc.method, ComplexityMethod::Cyclomatic);
    assert_eq!(calc.language, Some(SupportedLanguage::Rust));
    assert_eq!(calc.value, 4.0, "три if → cyclomatic 4");

    let notes = find(&rows, "notes.txt");
    assert_eq!(notes.method, ComplexityMethod::Indentation);
    assert_eq!(notes.language, None);
    assert_eq!(notes.value, 3.0);
}

#[test]
fn content_comes_from_git_object_not_disk() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("c.duckdb");
    let (store, reader) = setup(repo.path(), &db);

    // Портим файл на диске (без коммита) — добавляем кучу веток.
    write(
        repo.path(),
        "calc.rs",
        "fn f(x:i32)->i32{ if x>0 {} if x>1{} if x>2{} if x>3{} if x>4{} x }",
    );

    let rows = compute_complexity(&store, &reader).unwrap();
    // Должно остаться значение закоммиченного блоба (4), а не диска (6).
    assert_eq!(
        find(&rows, "calc.rs").value,
        4.0,
        "complexity считается из git-объекта на HEAD, а не с рабочего дерева"
    );
}

#[test]
fn results_are_cached_by_blob_sha() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, reader) = setup(repo.path(), &tmp.path().join("c.duckdb"));

    let first = compute_complexity(&store, &reader).unwrap();
    // После первого прогона кэш заполнен (2 файла).
    let cached: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM complexity_cache", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cached, 2);

    // Второй прогон использует кэш и даёт идентичный результат.
    let second = compute_complexity(&store, &reader).unwrap();
    assert_eq!(first, second);
}

#[test]
fn renamed_file_uses_canonical_path() {
    let repo = build_fixture();
    // Переименуем calc.rs → math.rs.
    git(repo.path(), &["mv", "calc.rs", "math.rs"]);
    git(repo.path(), &["commit", "-q", "-m", "rename"]);

    let tmp = TempDir::new().unwrap();
    let (store, reader) = setup(repo.path(), &tmp.path().join("c.duckdb"));
    let rows = compute_complexity(&store, &reader).unwrap();

    // Complexity под новым (каноническим) именем; старое имя не висит.
    assert_eq!(find(&rows, "math.rs").value, 4.0);
    assert!(rows.iter().all(|r| r.path != "calc.rs"));
}

#[test]
fn deleted_file_is_absent() {
    let repo = build_fixture();
    git(repo.path(), &["rm", "-q", "notes.txt"]);
    git(repo.path(), &["commit", "-q", "-m", "del"]);

    let tmp = TempDir::new().unwrap();
    let (store, reader) = setup(repo.path(), &tmp.path().join("c.duckdb"));
    let rows = compute_complexity(&store, &reader).unwrap();

    assert!(
        rows.iter().all(|r| r.path != "notes.txt"),
        "мёртвый файл исключён"
    );
    assert_eq!(find(&rows, "calc.rs").value, 4.0);
}
