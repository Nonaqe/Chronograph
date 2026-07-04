//! Профайлер analyze: куда уходит время — обход истории, tree-diff (rename
//! detection), построчный blob-diff (added/deleted) или запись в стор.
//!
//! Запуск: `cargo run -p chronograph-git --example profile_analyze -- <repo>`
//!
//! Фазы 1–3 гоняются напрямую через gix (реплика существенной части пайплайна,
//! только для замера); фаза 4 — продакшен-путь `run_analysis` с no-op стором
//! (walk+diff без DuckDB). Стоимость записи в DuckDB = число из
//! `chronograph-metrics/examples/profile.rs` (analyze) минус фаза 4.

use std::time::Instant;

use chronograph_core::{run_analysis, AnalysisMeta, Commit, Config, Result, Store};
use chronograph_git::GitSource;
use gix::diff::blob::sources::byte_lines;
use gix::diff::blob::{Algorithm, Diff, InternedInput};
use gix::diff::tree_with_rewrites::Change;
use gix::diff::{Options as DiffOptions, Rewrites};

/// Стор-заглушка: считает вызовы, ничего не пишет (мерим пайплайн без DuckDB).
struct NoopStore {
    commits: usize,
}

impl Store for NoopStore {
    fn last_head(&self) -> Result<Option<String>> {
        Ok(None)
    }
    fn write_commit(&mut self, _commit: &Commit, _is_mechanical: bool) -> Result<()> {
        self.commits += 1;
        Ok(())
    }
    fn write_meta(&mut self, _meta: &AnalysisMeta) -> Result<()> {
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

fn rewrites() -> Rewrites {
    // Как в GitSource::open — детерминированная конфигурация rename detection.
    Rewrites {
        copies: None,
        percentage: Some(0.5),
        limit: 1000,
        track_empty: false,
    }
}

fn main() {
    let repo_path = std::env::args()
        .nth(1)
        .expect("usage: profile_analyze <repo>");

    let mut repo = gix::discover(&repo_path).expect("открытие репо");
    repo.object_cache_size_if_unset(32 * 1024 * 1024);
    let head = repo
        .head()
        .expect("HEAD")
        .id()
        .expect("есть история")
        .detach();

    // --- Фаза 1: последовательный обход + декодирование коммитов (без diff) ---
    let t = Instant::now();
    let mut order: Vec<(gix::ObjectId, Vec<gix::ObjectId>)> = Vec::new();
    for info in repo.rev_walk(Some(head)).all().expect("walk") {
        let info = info.expect("info");
        let commit = repo.find_commit(info.id).expect("commit");
        let _author = commit.author().expect("author");
        let _time = commit.time().expect("time");
        order.push((info.id, info.parent_ids.iter().copied().collect()));
    }
    let t_walk = t.elapsed();

    // --- Фаза 2: + tree-diff с rename detection (без построчных счётчиков) ---
    let t = Instant::now();
    let mut change_count = 0usize;
    for (id, parents) in &order {
        let commit = repo.find_commit(*id).expect("commit");
        let new_tree = commit.tree().expect("tree");
        let empty;
        let parent_tree = match parents.first() {
            Some(pid) => repo.find_commit(*pid).expect("parent").tree().expect("t"),
            None => {
                empty = repo.empty_tree();
                empty
            }
        };
        let opts = DiffOptions::default().with_rewrites(Some(rewrites()));
        let changes = repo
            .diff_tree_to_tree(Some(&parent_tree), Some(&new_tree), Some(opts))
            .expect("diff");
        change_count += changes.len();
    }
    let t_treediff = t.elapsed();

    // --- Фаза 3: + построчный blob-diff (added/deleted) для каждого изменения ---
    let t = Instant::now();
    let mut line_ops = 0usize;
    for (id, parents) in &order {
        let commit = repo.find_commit(*id).expect("commit");
        let new_tree = commit.tree().expect("tree");
        let empty;
        let parent_tree = match parents.first() {
            Some(pid) => repo.find_commit(*pid).expect("parent").tree().expect("t"),
            None => {
                empty = repo.empty_tree();
                empty
            }
        };
        let opts = DiffOptions::default().with_rewrites(Some(rewrites()));
        let changes = repo
            .diff_tree_to_tree(Some(&parent_tree), Some(&new_tree), Some(opts))
            .expect("diff");
        for change in changes {
            let (old, new) = match change {
                Change::Addition { id, entry_mode, .. } => {
                    if !entry_mode.is_blob_or_symlink() {
                        continue;
                    }
                    (None, Some(id))
                }
                Change::Deletion { id, entry_mode, .. } => {
                    if !entry_mode.is_blob_or_symlink() {
                        continue;
                    }
                    (Some(id), None)
                }
                Change::Modification {
                    previous_id,
                    id,
                    entry_mode,
                    ..
                } => {
                    if !entry_mode.is_blob_or_symlink() {
                        continue;
                    }
                    (Some(previous_id), Some(id))
                }
                Change::Rewrite {
                    source_id,
                    id,
                    entry_mode,
                    ..
                } => {
                    if !entry_mode.is_blob_or_symlink() {
                        continue;
                    }
                    (Some(source_id), Some(id))
                }
            };
            let old_bytes = old
                .map(|o| repo.find_object(o).expect("blob").data.clone())
                .unwrap_or_default();
            let new_bytes = new
                .map(|o| repo.find_object(o).expect("blob").data.clone())
                .unwrap_or_default();
            let input = InternedInput::new(byte_lines(&old_bytes), byte_lines(&new_bytes));
            let diff = Diff::compute(Algorithm::Histogram, &input);
            line_ops += (diff.count_additions() + diff.count_removals()) as usize;
        }
    }
    let t_blobdiff = t.elapsed();

    // --- Фаза 4: продакшен-путь (GitSource + run_analysis) с no-op стором ---
    let t = Instant::now();
    let cfg = Config::new(&repo_path);
    let src = GitSource::open(&cfg).expect("GitSource");
    let mut noop = NoopStore { commits: 0 };
    run_analysis(&src, &mut noop, &cfg, 0).expect("analyze");
    let t_pipeline = t.elapsed();

    println!(
        "коммитов: {}, изменений: {change_count}, строк±: {line_ops}",
        order.len()
    );
    println!("{:<34} {:>8}", "фаза", "сек");
    println!(
        "{:<34} {:>8.2}",
        "1 walk+decode (без diff)",
        t_walk.as_secs_f64()
    );
    println!(
        "{:<34} {:>8.2}",
        "2 walk+tree-diff (rename)",
        t_treediff.as_secs_f64()
    );
    println!(
        "{:<34} {:>8.2}",
        "3 walk+tree-diff+blob-diff",
        t_blobdiff.as_secs_f64()
    );
    println!(
        "{:<34} {:>8.2}",
        "4 продакшен walk→diff (no-op стор)",
        t_pipeline.as_secs_f64()
    );
    println!();
    println!(
        "  ≈tree-diff = ф2−ф1: {:.2}s",
        (t_treediff - t_walk).as_secs_f64()
    );
    println!(
        "  ≈blob-diff = ф3−ф2: {:.2}s",
        t_blobdiff.as_secs_f64() - t_treediff.as_secs_f64()
    );
}
