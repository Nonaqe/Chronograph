//! Интеграционные тесты blame-кэша: повторный прогон не переблеймливает
//! неизменившиеся файлы; изменившиеся — переблеймливаются; результат идентичен.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use chronograph_core::{run_analysis, BlameHunk, BlameSource, Config, FileBlame, Result};
use chronograph_git::GitSource;
use chronograph_metrics::{compute_knowledge, KnowledgeConfig};
use chronograph_store::DuckStore;
use tempfile::TempDir;

const FIXED_NOW: i64 = 1_700_000_000;

fn git_as(dir: &Path, name: &str, email: &str, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", name)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_AUTHOR_DATE", "2021-06-01T00:00:00 +0000")
        .env("GIT_COMMITTER_NAME", name)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_COMMITTER_DATE", "2021-06-01T00:00:00 +0000")
        .status()
        .expect("git доступен");
    assert!(status.success(), "git {args:?} провалилась");
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

/// Обёртка над настоящим blamer'ом, считающая, СКОЛЬКО путей реально блеймится.
struct Counting<'a> {
    inner: &'a GitSource,
    blamed_paths: AtomicUsize,
    last_batch: Mutex<Vec<String>>,
}

impl<'a> Counting<'a> {
    fn new(inner: &'a GitSource) -> Self {
        Counting {
            inner,
            blamed_paths: AtomicUsize::new(0),
            last_batch: Mutex::new(Vec::new()),
        }
    }
    fn count(&self) -> usize {
        self.blamed_paths.load(Ordering::SeqCst)
    }
    fn batch(&self) -> Vec<String> {
        self.last_batch.lock().unwrap().clone()
    }
}

impl BlameSource for Counting<'_> {
    fn blame_lines(&self, path: &str, at: &str) -> Result<Vec<BlameHunk>> {
        self.inner.blame_lines(path, at)
    }
    fn blame_many(&self, paths: &[String], at: &str) -> Result<Vec<FileBlame>> {
        self.blamed_paths.fetch_add(paths.len(), Ordering::SeqCst);
        *self.last_batch.lock().unwrap() = paths.to_vec();
        self.inner.blame_many(paths, at)
    }
}

/// Фикстура: f.rs (2 ревизии) и g.rs (1 ревизия) от двух авторов.
fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git_as(p, "Init", "init@example.com", &["init", "-q", "-b", "main"]);
    write(p, "f.rs", "l1\nl2\n");
    write(p, "g.rs", "a1\n");
    git_as(p, "Alice", "alice@example.com", &["add", "."]);
    git_as(
        p,
        "Alice",
        "alice@example.com",
        &["commit", "-q", "-m", "c1"],
    );
    write(p, "f.rs", "l1\nCHANGED\n");
    git_as(p, "Bob", "bob@example.com", &["commit", "-q", "-am", "c2"]);
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

#[test]
fn second_run_blames_nothing_and_matches() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, src) = setup(repo.path(), &tmp.path().join("k.duckdb"));

    // Первый прогон: оба файла блеймятся (кэш пуст).
    let counting = Counting::new(&src);
    let first = compute_knowledge(&store, &counting, &KnowledgeConfig::default()).unwrap();
    assert_eq!(counting.count(), 2, "холодный кэш: блеймятся оба файла");

    // Второй прогон: НИЧЕГО не блеймится (полный кэш-хит), результат идентичен.
    let second = compute_knowledge(&store, &counting, &KnowledgeConfig::default()).unwrap();
    assert_eq!(counting.count(), 2, "тёплый кэш: новых blame нет");
    assert_eq!(first, second, "кэшированный результат идентичен прямому");
}

#[test]
fn only_changed_file_is_reblamed() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("k.duckdb");
    let (store, src) = setup(repo.path(), &db);

    // Прогреть кэш.
    let counting = Counting::new(&src);
    let _ = compute_knowledge(&store, &counting, &KnowledgeConfig::default()).unwrap();
    assert_eq!(counting.count(), 2);
    drop(store);

    // Новый коммит меняет ТОЛЬКО g.rs → инкрементальный analyze → knowledge.
    write(repo.path(), "g.rs", "a1\nADDED\n");
    git_as(
        repo.path(),
        "Bob",
        "bob@example.com",
        &["commit", "-q", "-am", "c3"],
    );
    let cfg = Config::new(repo.path());
    let src2 = GitSource::open(&cfg).unwrap();
    {
        let mut store = DuckStore::open(&db).unwrap();
        run_analysis(&src2, &mut store, &cfg, FIXED_NOW).unwrap();
    }
    let store = DuckStore::open(&db).unwrap();

    let counting2 = Counting::new(&src2);
    let report = compute_knowledge(&store, &counting2, &KnowledgeConfig::default()).unwrap();
    assert_eq!(
        counting2.count(),
        1,
        "переблеймлен только изменившийся файл"
    );
    assert_eq!(counting2.batch(), vec!["g.rs".to_string()]);

    // Метрика при этом корректна: g.rs теперь 1/2 Alice + 1/2 Bob... точнее:
    // g.rs: a1 (Alice) + ADDED (Bob) → два владельца.
    let g = report.files.iter().find(|f| f.path == "g.rs").unwrap();
    assert_eq!(g.owners.len(), 2, "новое владение g.rs учтено");
    // f.rs — из кэша, владение прежнее (1 строка Alice + 1 Bob).
    let f = report.files.iter().find(|f| f.path == "f.rs").unwrap();
    assert_eq!(f.owners.len(), 2);
}

#[test]
fn over_budget_files_are_skipped_with_reason() {
    use chronograph_metrics::SkipReason;

    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, src) = setup(repo.path(), &tmp.path().join("k.duckdb"));

    // Стоимости: f.rs = 2 рев × 3 added = 6; g.rs = 1 × 1 = 1. Бюджет 1 →
    // f.rs дороже бюджета и пропускается, g.rs блеймится.
    let counting = Counting::new(&src);
    let cfg = chronograph_metrics::KnowledgeConfig {
        blame_budget: 1,
        ..Default::default()
    };
    let report = compute_knowledge(&store, &counting, &cfg).unwrap();

    assert_eq!(
        counting.batch(),
        vec!["g.rs".to_string()],
        "f.rs не блеймился"
    );
    assert_eq!(report.blame_skipped, 1);
    assert_eq!(report.skips.len(), 1);
    assert_eq!(report.skips[0].path, "f.rs");
    assert!(
        matches!(
            report.skips[0].reason,
            SkipReason::OverBudget { cost: 6, budget: 1 }
        ),
        "причина с числами: {:?}",
        report.skips[0].reason
    );
    // f.rs выпал из метрики, g.rs посчитан.
    assert!(report.files.iter().all(|f| f.path != "f.rs"));
    assert!(report.files.iter().any(|f| f.path == "g.rs"));

    // Бюджет 0 = безлимит: всё блеймится, пропусков нет.
    let report_unlim = compute_knowledge(&store, &counting, &KnowledgeConfig::default()).unwrap();
    assert_eq!(
        report_unlim.blame_skipped, 0,
        "дефолтный бюджет фикстуру не режет"
    );
    assert!(report_unlim.files.iter().any(|f| f.path == "f.rs"));
}

#[test]
fn largest_first_orders_by_revisions() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, src) = setup(repo.path(), &tmp.path().join("k.duckdb"));

    // f.rs имеет 2 ревизии, g.rs — 1 → в blame-очереди f.rs идёт ПЕРВЫМ.
    let counting = Counting::new(&src);
    let _ = compute_knowledge(&store, &counting, &KnowledgeConfig::default()).unwrap();
    assert_eq!(
        counting.batch(),
        vec!["f.rs".to_string(), "g.rs".to_string()],
        "гиганты (больше ревизий) первыми"
    );
}
