//! Интеграционные тесты knowledge / bus factor на синтетическом multi-author
//! репозитории. Прогоняют НАСТОЯЩИЙ gix blame end-to-end (через `GitSource`),
//! с заранее известным распределением авторства.

use std::path::Path;
use std::process::Command;

use chronograph_core::{
    run_analysis, BlameHunk, BlameSource, Config, FileBlame, Result as CoreResult,
};
use chronograph_git::GitSource;
use chronograph_metrics::{compute_knowledge, FileKnowledge, KnowledgeConfig};
use chronograph_store::DuckStore;
use tempfile::TempDir;

const FIXED_NOW: i64 = 1_700_000_000;

/// git-команда с явными автором/датой (детерминизм). Автор варьируется по коммитам.
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

/// Репозиторий с известным авторством:
/// - f.rs: 4 строки от Alice, затем Bob меняет строку 2 → 3 Alice / 1 Bob.
/// - g.rs: 2 строки от Alice, затем Bob дописывает 2 → 2 Alice / 2 Bob.
fn build_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git_as(p, "Init", "init@example.com", &["init", "-q", "-b", "main"]);

    // c1 (Alice): создаёт f.rs (4 строки) и g.rs (2 строки).
    write(p, "f.rs", "l1\nl2\nl3\nl4\n");
    write(p, "g.rs", "a1\na2\n");
    git_as(p, "Alice", "alice@example.com", &["add", "."]);
    git_as(
        p,
        "Alice",
        "alice@example.com",
        &["commit", "-q", "-m", "c1"],
    );

    // c2 (Bob): меняет строку 2 в f.rs.
    write(p, "f.rs", "l1\nCHANGED\nl3\nl4\n");
    git_as(p, "Bob", "bob@example.com", &["commit", "-q", "-am", "c2"]);

    // c3 (Bob): дописывает 2 строки в g.rs.
    write(p, "g.rs", "a1\na2\nb1\nb2\n");
    git_as(p, "Bob", "bob@example.com", &["commit", "-q", "-am", "c3"]);

    dir
}

/// Прогнать analyze в БД, вернуть (переоткрытый стор, git-источник как blamer).
fn setup(repo: &Path, db: &Path) -> (DuckStore, GitSource) {
    let cfg = Config::new(repo);
    let src = GitSource::open(&cfg).unwrap();
    {
        let mut store = DuckStore::open(db).unwrap();
        run_analysis(&src, &mut store, &cfg, FIXED_NOW).unwrap();
    }
    (DuckStore::open(db).unwrap(), src)
}

/// author_id по email из таблицы authors (порядок присвоения — деталь стора).
fn author_id(store: &DuckStore, email: &str) -> i64 {
    store
        .conn()
        .query_row(
            "SELECT author_id FROM authors WHERE canonical_email = ?",
            [email],
            |r| r.get(0),
        )
        .unwrap()
}

fn find<'a>(rows: &'a [FileKnowledge], path: &str) -> &'a FileKnowledge {
    rows.iter().find(|r| r.path == path).expect("файл есть")
}

fn ratio_of(fk: &FileKnowledge, author: i64) -> f64 {
    fk.owners
        .iter()
        .find(|o| o.author_id == author)
        .map(|o| o.ownership_ratio)
        .unwrap_or(0.0)
}

#[test]
fn ownership_matches_known_authorship() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, blamer) = setup(repo.path(), &tmp.path().join("k.duckdb"));

    let rows = compute_knowledge(&store, &blamer, &KnowledgeConfig::default())
        .unwrap()
        .files;

    let alice = author_id(&store, "alice@example.com");
    let bob = author_id(&store, "bob@example.com");

    // f.rs: 3 строки Alice (0.75), 1 строка Bob (0.25); bus_factor 1; top 0.75.
    let f = find(&rows, "f.rs");
    assert_eq!(f.total_lines, 4);
    assert!((ratio_of(f, alice) - 0.75).abs() < 1e-9);
    assert!((ratio_of(f, bob) - 0.25).abs() < 1e-9);
    assert_eq!(f.bus_factor, 1);
    assert!((f.top_owner_ratio - 0.75).abs() < 1e-9);

    // g.rs: 2 строки Alice (0.5), 2 строки Bob (0.5); bus_factor 2 (ровно 50% не > порога).
    let g = find(&rows, "g.rs");
    assert_eq!(g.total_lines, 4);
    assert!((ratio_of(g, alice) - 0.5).abs() < 1e-9);
    assert!((ratio_of(g, bob) - 0.5).abs() < 1e-9);
    assert_eq!(g.bus_factor, 2);
    assert!((g.top_owner_ratio - 0.5).abs() < 1e-9);
}

#[test]
fn ownership_sums_to_one_per_file() {
    // Property (CLAUDE.md): сумма ownership по файлу ≈ 1.0.
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, blamer) = setup(repo.path(), &tmp.path().join("k.duckdb"));

    let rows = compute_knowledge(&store, &blamer, &KnowledgeConfig::default())
        .unwrap()
        .files;
    assert!(!rows.is_empty());
    for fk in &rows {
        let sum: f64 = fk.owners.iter().map(|o| o.ownership_ratio).sum();
        assert!((sum - 1.0).abs() < 1e-9, "{}: сумма долей {sum}", fk.path);
        assert!(fk.bus_factor >= 1, "{}: bus_factor ≥ 1", fk.path);
        // Строки владельцев суммируются в total_lines.
        let lines: u32 = fk.owners.iter().map(|o| o.lines).sum();
        assert_eq!(lines, fk.total_lines);
    }
}

#[test]
fn knowledge_is_deterministic() {
    // Property (правило 4): два прогона → идентичный результат.
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, blamer) = setup(repo.path(), &tmp.path().join("k.duckdb"));

    let a = compute_knowledge(&store, &blamer, &KnowledgeConfig::default()).unwrap();
    let b = compute_knowledge(&store, &blamer, &KnowledgeConfig::default()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn mailmap_collapses_one_person_multiple_emails() {
    // .mailmap схлопывает второй email Alice в основной → f.rs целиком её,
    // bus_factor 1 (без mailmap было бы два автора 0.75/0.25).
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git_as(p, "Init", "init@example.com", &["init", "-q", "-b", "main"]);

    // .mailmap: alice2@ → alice@ (стандартный формат "Name <canon> <other>").
    write(
        p,
        ".mailmap",
        "Alice <alice@example.com> <alice2@example.com>\n",
    );

    write(p, "f.rs", "l1\nl2\nl3\nl4\n");
    git_as(p, "Alice", "alice@example.com", &["add", "."]);
    git_as(
        p,
        "Alice",
        "alice@example.com",
        &["commit", "-q", "-m", "c1"],
    );

    // Тот же человек под ВТОРЫМ email меняет строку 2.
    write(p, "f.rs", "l1\nCHANGED\nl3\nl4\n");
    git_as(
        p,
        "Alice",
        "alice2@example.com",
        &["commit", "-q", "-am", "c2"],
    );

    let tmp = TempDir::new().unwrap();
    let (store, blamer) = setup(p, &tmp.path().join("k.duckdb"));

    // После mailmap-ingestion — ровно один автор в сторе.
    let author_count: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM authors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        author_count, 1,
        "mailmap схлопнул два email в одного автора"
    );

    let rows = compute_knowledge(&store, &blamer, &KnowledgeConfig::default())
        .unwrap()
        .files;
    let f = find(&rows, "f.rs");
    assert_eq!(
        f.owners.len(),
        1,
        "все строки у одного (канонического) автора"
    );
    assert!((f.top_owner_ratio - 1.0).abs() < 1e-9);
    assert_eq!(f.bus_factor, 1);
}

/// Обёртка над реальным blamer, форсирующая [`FileBlame::Failed`] для одного пути.
///
/// Реальный триггер паники gix-blame невоспроизводим синтетикой, поэтому
/// эмулируем сбой на уровне трейта — проверяем именно логику УЧЁТА пропусков.
struct FailOne<'a> {
    inner: &'a GitSource,
    fail: String,
}

impl BlameSource for FailOne<'_> {
    fn blame_lines(&self, path: &str, at: &str) -> CoreResult<Vec<BlameHunk>> {
        self.inner.blame_lines(path, at)
    }

    fn blame_many(&self, paths: &[String], at: &str) -> CoreResult<Vec<FileBlame>> {
        let inner = self.inner.blame_many(paths, at)?;
        Ok(paths
            .iter()
            .zip(inner)
            .map(|(p, fb)| {
                if *p == self.fail {
                    FileBlame::Failed
                } else {
                    fb
                }
            })
            .collect())
    }
}

#[test]
fn blame_failure_is_counted_not_lost() {
    let repo = build_fixture();
    let tmp = TempDir::new().unwrap();
    let (store, gitsrc) = setup(repo.path(), &tmp.path().join("k.duckdb"));

    let blamer = FailOne {
        inner: &gitsrc,
        fail: "f.rs".to_string(),
    };
    let report = compute_knowledge(&store, &blamer, &KnowledgeConfig::default()).unwrap();

    // f.rs «упал» → выпал из результата, НО посчитан; g.rs обработан нормально.
    assert_eq!(report.blame_skipped, 1, "пропуск учтён, не потерян");
    assert!(
        report.files.iter().all(|fk| fk.path != "f.rs"),
        "упавший файл не в результате"
    );
    assert!(
        report.files.iter().any(|fk| fk.path == "g.rs"),
        "остальные файлы посчитаны"
    );
}
