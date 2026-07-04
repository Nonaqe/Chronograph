//! Интеграционные тесты churn на синтетических репо с заранее известными ответами.

use std::path::Path;
use std::process::Command;

use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{compute_churn, ChurnConfig, FileChurn};
use chronograph_store::DuckStore;
use tempfile::TempDir;

const FIXED_NOW: i64 = 1_700_000_000;

/// git-команда с фиксированными автором/коммиттером и заданной датой коммита.
fn git_at(dir: &Path, date: &str, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git доступен");
    assert!(status.success(), "git {args:?} провалилась");
}

/// Недавние коммиты (2021) — с монотонно растущими секундами, чтобы порядок
/// «последнего изменения» (для is_alive) был однозначен. В реальных репо при
/// равных таймстемпах порядок неоднозначен — известный нюанс, см. CONTEXT.md.
fn git(dir: &Path, args: &[&str]) {
    git_at(dir, "2021-06-01T00:00:00 +0000", args);
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

fn find<'a>(rows: &'a [FileChurn], path: &str) -> Option<&'a FileChurn> {
    rows.iter().find(|r| r.path == path)
}

/// Прогнать analyze с заданным порогом механического коммита и посчитать churn.
fn analyze_and_churn(
    repo: &Path,
    mech_max_files: Option<u32>,
    cfg: &ChurnConfig,
) -> Vec<FileChurn> {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("cache.duckdb");
    let mut analysis_cfg = Config::new(repo);
    analysis_cfg.mechanical_commit_max_files = mech_max_files;

    let src = GitSource::open(&analysis_cfg).unwrap();
    {
        let mut store = DuckStore::open(&db).unwrap();
        run_analysis(&src, &mut store, &analysis_cfg, FIXED_NOW).unwrap();
    }
    let store = DuckStore::open(&db).unwrap();
    compute_churn(&store, cfg).unwrap()
}

/// Репо с известной историей:
/// - a.txt: создан и дважды изменён (старые даты 2019) → churn_total=3, давно
/// - b.txt → c.txt: создан (2019) и переименован (2021) → склейка истории
/// - g0..g5: один «механический» коммит (6 файлов) 2021
/// - e.txt: создан и удалён (2021) → is_alive=false
fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    let old = "2019-01-01T00:00:00 +0000";

    git(p, &["init", "-q", "-b", "main"]);

    write(p, "a.txt", "1\n");
    write(p, "b.txt", "b\n");
    git_at(p, old, &["add", "."]);
    git_at(p, old, &["commit", "-q", "-m", "c1"]); // add a.txt, b.txt

    write(p, "a.txt", "1\n2\n");
    git_at(p, old, &["add", "."]);
    git_at(p, old, &["commit", "-q", "-m", "c2"]); // modify a.txt

    write(p, "a.txt", "1\n2\n3\n");
    git_at(p, old, &["add", "."]);
    git_at(p, old, &["commit", "-q", "-m", "c3"]); // modify a.txt

    // Переименование b.txt → c.txt (недавнее, 2021).
    git_at(p, "2021-06-01T00:00:00 +0000", &["mv", "b.txt", "c.txt"]);
    git_at(
        p,
        "2021-06-01T00:00:00 +0000",
        &["commit", "-q", "-am", "c4"],
    );

    // «Механический» коммит — 6 файлов.
    for i in 0..6 {
        write(p, &format!("g{i}.txt"), "x\n");
    }
    git_at(p, "2021-06-01T00:00:01 +0000", &["add", "."]);
    git_at(
        p,
        "2021-06-01T00:00:01 +0000",
        &["commit", "-q", "-m", "c5-mechanical"],
    );

    // e.txt: создан (c6), затем удалён (c7, строго позже) → мёртвый.
    write(p, "e.txt", "e\n");
    git_at(p, "2021-06-01T00:00:02 +0000", &["add", "."]);
    git_at(
        p,
        "2021-06-01T00:00:02 +0000",
        &["commit", "-q", "-m", "c6"],
    );
    git_at(p, "2021-06-01T00:00:03 +0000", &["rm", "-q", "e.txt"]);
    git_at(
        p,
        "2021-06-01T00:00:03 +0000",
        &["commit", "-q", "-m", "c7"],
    );

    dir
}

#[test]
fn churn_counts_commits_per_file() {
    let dir = build_fixture();
    // Порог механического коммита = 5 файлов → c5 (6 файлов) помечается.
    let rows = analyze_and_churn(dir.path(), Some(5), &ChurnConfig::default());

    let a = find(&rows, "a.txt").expect("a.txt есть");
    assert_eq!(a.churn_total, 3, "a.txt тронут в c1,c2,c3");
    assert!(a.is_alive);

    // c.txt — канонический путь для b.txt: история склеена (c1 создал b, c4 rename).
    let c = find(&rows, "c.txt").expect("c.txt есть");
    assert_eq!(c.churn_total, 2, "история b.txt+c.txt склеена через rename");
    assert!(c.is_alive);
    // b.txt как отдельный файл в churn НЕ висит (слит в c.txt).
    assert!(find(&rows, "b.txt").is_none());
}

#[test]
fn mechanical_commit_is_excluded() {
    let dir = build_fixture();
    let rows = analyze_and_churn(dir.path(), Some(5), &ChurnConfig::default());
    // Файлы из механического коммита не дают churn-сигнала.
    for i in 0..6 {
        assert!(
            find(&rows, &format!("g{i}.txt")).is_none(),
            "g{i}.txt из механического коммита должен быть исключён"
        );
    }
}

#[test]
fn mechanical_filter_can_be_disabled() {
    let dir = build_fixture();
    // Без порога механических коммитов нет → g* присутствуют независимо от флага.
    let rows = analyze_and_churn(dir.path(), None, &ChurnConfig::default());
    assert!(find(&rows, "g0.txt").is_some());
}

#[test]
fn dead_file_marked_not_alive() {
    let dir = build_fixture();
    let rows = analyze_and_churn(dir.path(), Some(5), &ChurnConfig::default());
    let e = find(&rows, "e.txt").expect("e.txt есть");
    assert_eq!(e.churn_total, 2, "создан и удалён");
    assert!(!e.is_alive, "последнее изменение — удаление");
}

#[test]
fn sliding_windows_respect_anchor() {
    let dir = build_fixture();
    let rows = analyze_and_churn(dir.path(), Some(5), &ChurnConfig::default());

    // Якорь = max(committed_at) = 2021-06-01. a.txt менялся только в 2019 →
    // за все недавние окна 0, total = 3.
    let a = find(&rows, "a.txt").unwrap();
    assert_eq!(a.churn_total, 3);
    assert_eq!(
        a.churn_long, 0,
        "365 дней назад от 2021-06 не достаёт до 2019"
    );
    assert_eq!(a.churn_mid, 0);
    assert_eq!(a.churn_recent, 0);

    // c.txt: создание b в 2019 (вне окон) + rename в 2021 (внутри всех окон).
    let c = find(&rows, "c.txt").unwrap();
    assert_eq!(c.churn_total, 2);
    assert_eq!(c.churn_long, 1);
    assert_eq!(c.churn_mid, 1);
    assert_eq!(c.churn_recent, 1);
}

#[test]
fn churn_is_deterministic() {
    let dir = build_fixture();
    let a = analyze_and_churn(dir.path(), Some(5), &ChurnConfig::default());
    let b = analyze_and_churn(dir.path(), Some(5), &ChurnConfig::default());
    assert_eq!(a, b);
}

#[test]
fn name_reuse_in_single_commit_is_alive_and_stable() {
    // Регресс (ловилось на clap): в ОДНОМ коммите имя умирает и возрождается —
    // D(a.txt) + R(c.txt -> a.txt). Обе строки коммита мапятся на один canonical
    // с РАВНЫМИ (ts, sha); без полного тай-брейка row_number зависел от порядка
    // скана DuckDB и is_alive флипал между прогонами. Семантика: файл ЖИВ.
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);

    write(p, "a.txt", "old incarnation\n");
    write(p, "c.txt", "future a\nline2\nline3\n");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "c1"]);

    // Один коммит: удалить a.txt и переименовать c.txt в a.txt.
    std::fs::remove_file(p.join("a.txt")).unwrap();
    std::fs::rename(p.join("c.txt"), p.join("a.txt")).unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "c2-kill-and-rebirth"]);

    // Многократные прогоны: is_alive стабильно true (не флипает).
    for i in 0..10 {
        let rows = analyze_and_churn(p, None, &ChurnConfig::default());
        let a = find(&rows, "a.txt").expect("canonical a.txt есть");
        assert!(
            a.is_alive,
            "прогон {i}: файл жив (возрождён в том же коммите)"
        );
    }
}
