//! Интеграционные тесты полного пайплайна Этапа 0:
//! `run_analysis(GitSource, DuckStore)` на реальном синтетическом git-репо.
//!
//! Покрывают критерий готовности Этапа 0 (строится таблица изменений; повторный
//! запуск инкрементален) и обязательное правило воспроизводимости из CLAUDE.md.

use std::path::Path;
use std::process::Command;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_store::DuckStore;
use duckdb::{types::Value, Connection};
use tempfile::TempDir;

/// Фиксированное «время прогона» — чтобы analysis_meta.analyzed_at не вносил
/// различий между прогонами (единственное легитимно «плавающее» поле).
const FIXED_NOW: i64 = 1_700_000_000;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_AUTHOR_DATE", "2021-01-01T00:00:00 +0000")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_DATE", "2021-01-01T00:00:00 +0000")
        .status()
        .expect("git доступен в dev-окружении");
    assert!(status.success(), "git {args:?} провалилась");
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

/// Репо из трёх коммитов с известной историей (add → modify+add → rename).
fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);

    write(p, "a.txt", "line1\nline2\n");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "c1"]);

    write(p, "a.txt", "line1\nline2\nline3\n");
    write(p, "b.txt", "bbb\n");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "c2"]);

    git(p, &["mv", "b.txt", "c.txt"]);
    git(p, &["commit", "-q", "-am", "c3"]);

    dir
}

/// Прогнать анализ репо `repo` в файл БД `db`.
fn analyze(repo: &Path, db: &Path) -> chronograph_core::AnalysisOutcome {
    let cfg = Config::new(repo);
    let src = GitSource::open(&cfg).unwrap();
    let mut store = DuckStore::open(db).unwrap();
    run_analysis(&src, &mut store, &cfg, FIXED_NOW).unwrap()
}

/// Детерминированный дамп сырых таблиц из файла БД.
fn dump(db: &Path) -> String {
    let conn = Connection::open(db).unwrap();
    let mut out = String::new();
    for (table, order, ncols) in [
        ("authors", "author_id", 3usize),
        ("commits", "sha", 5),
        ("file_changes", "sha, path, change_type", 7),
        ("analysis_meta", "head_sha", 4),
    ] {
        out.push_str(&format!("== {table} ==\n"));
        let sql = format!("SELECT * FROM {table} ORDER BY {order}");
        let mut stmt = conn.prepare(&sql).unwrap();
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

fn scalar(db: &Path, sql: &str) -> i64 {
    let conn = Connection::open(db).unwrap();
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn scalar_text(db: &Path, sql: &str) -> Option<String> {
    let conn = Connection::open(db).unwrap();
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn builds_change_table_from_any_repo() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("cache.duckdb");

    let outcome = analyze(repo.path(), &db);
    assert_eq!(outcome.new_commits, 3);
    assert!(!outcome.up_to_date);

    assert_eq!(scalar(&db, "SELECT count(*) FROM commits"), 3);
    // a.txt×3 (add+modify+rename-source… фактически: c1 a, c2 a+b, c3 rename) =
    // 1 + 2 + 1 = 4 строки изменений.
    assert_eq!(scalar(&db, "SELECT count(*) FROM file_changes"), 4);
    assert_eq!(scalar(&db, "SELECT count(*) FROM authors"), 1);
    // Переименование записано типом 'R' с сохранённым прежним путём — история
    // не фрагментируется (b.txt → c.txt).
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM file_changes WHERE change_type = 'R'"
        ),
        1
    );
    assert_eq!(
        scalar_text(
            &db,
            "SELECT old_path FROM file_changes WHERE change_type = 'R'"
        ),
        Some("b.txt".to_string())
    );
    // Для обычных изменений old_path остаётся NULL.
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM file_changes WHERE change_type <> 'R' AND old_path IS NOT NULL"
        ),
        0
    );
}

#[test]
fn second_run_is_incremental_noop() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("cache.duckdb");

    let first = analyze(repo.path(), &db);
    assert_eq!(first.new_commits, 3);

    // Повторный запуск без новых коммитов → ничего не обрабатывается.
    let second = analyze(repo.path(), &db);
    assert_eq!(second.new_commits, 0);
    assert!(second.up_to_date);
    assert_eq!(scalar(&db, "SELECT count(*) FROM commits"), 3);
}

#[test]
fn incremental_processes_only_new_commits() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("cache.duckdb");

    analyze(repo.path(), &db);
    assert_eq!(scalar(&db, "SELECT count(*) FROM commits"), 3);

    // Новый коммит в репо.
    write(repo.path(), "d.txt", "ddd\n");
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-q", "-m", "c4"]);

    let outcome = analyze(repo.path(), &db);
    // Обработан только один новый коммит.
    assert_eq!(outcome.new_commits, 1);
    assert_eq!(scalar(&db, "SELECT count(*) FROM commits"), 4);
}

#[test]
fn two_runs_produce_identical_output() {
    // Воспроизводимость (правило 4 CLAUDE.md): один репо → два независимых
    // прогона дают байт-в-байт идентичный дамп таблиц.
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let db_a = tmp.path().join("a.duckdb");
    let db_b = tmp.path().join("b.duckdb");

    analyze(repo.path(), &db_a);
    analyze(repo.path(), &db_b);

    assert_eq!(dump(&db_a), dump(&db_b));
}
