//! Оркестрация инкрементального анализа.
//!
//! Связывает [`CommitSource`] и [`Store`] через трейты, не зная их реализаций.
//! Это «оркестрация» из CLAUDE.md, живущая в лёгком ядре.

use crate::config::Config;
use crate::model::AnalysisMeta;
use crate::source::CommitSource;
use crate::store::Store;
use crate::{Result, ENGINE_VERSION};

/// Итог прогона анализа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisOutcome {
    /// HEAD, на котором завершился прогон (`None` — репозиторий пуст).
    pub head_sha: Option<String>,
    /// Сколько новых коммитов обработано в этом прогоне.
    pub new_commits: u64,
    /// Был ли стор уже актуален (HEAD не изменился) — обработано 0 коммитов.
    pub up_to_date: bool,
}

/// Прогнать инкрементальный анализ: обойти новые коммиты и записать их в стор.
///
/// `now_unix` — текущее время (unix-секунды UTC) для `analysis_meta.analyzed_at`;
/// передаётся извне, чтобы ядро не зависело от часов и оставалось детерминированным.
///
/// Алгоритм инкрементальности:
/// 1. Узнать новый HEAD у источника. Пусто → ничего не делаем.
/// 2. В инкрементальном режиме взять `last_head` из стора. Если совпал с новым —
///    стор актуален, выходим.
/// 3. Обойти коммиты, достижимые из нового HEAD, но не из `last_head` (он
///    передаётся как `hidden`), и записать каждый.
/// 4. Записать `analysis_meta` с новым HEAD как точкой продолжения.
pub fn run_analysis<S, T>(
    source: &S,
    store: &mut T,
    cfg: &Config,
    now_unix: i64,
) -> Result<AnalysisOutcome>
where
    S: CommitSource,
    T: Store,
{
    let Some(new_head) = source.head_sha()? else {
        return Ok(AnalysisOutcome {
            head_sha: None,
            new_commits: 0,
            up_to_date: true,
        });
    };

    let last_head = if cfg.incremental {
        store.last_head()?
    } else {
        None
    };

    if cfg.incremental && last_head.as_deref() == Some(new_head.as_str()) {
        return Ok(AnalysisOutcome {
            head_sha: Some(new_head),
            new_commits: 0,
            up_to_date: true,
        });
    }

    let hidden = if cfg.incremental {
        last_head.as_deref()
    } else {
        None
    };

    let max_files = cfg.mechanical_commit_max_files;
    let mut new_commits: u64 = 0;
    source.for_each_commit(&new_head, hidden, &mut |commit| {
        let is_mechanical = match max_files {
            Some(limit) => commit.files_changed() as u64 > limit as u64,
            None => false,
        };
        store.write_commit(&commit, is_mechanical)?;
        new_commits += 1;
        Ok(())
    })?;

    store.write_meta(&AnalysisMeta {
        engine_version: ENGINE_VERSION.to_string(),
        config_hash: cfg.config_hash(),
        analyzed_at: now_unix,
        head_sha: new_head.clone(),
    })?;
    store.flush()?;

    Ok(AnalysisOutcome {
        head_sha: Some(new_head),
        new_commits,
        up_to_date: new_commits == 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Author, ChangeType, Commit, FileChange};
    use std::cell::RefCell;

    /// Мок-источник: фиксированный HEAD и список коммитов от него.
    struct MockSource {
        head: Option<String>,
        // (sha, commit) в порядке обхода
        commits: Vec<Commit>,
        // sha → его «возраст» для эмуляции hidden (предки hidden исключаются)
    }

    fn commit(sha: &str, n_files: usize) -> Commit {
        Commit {
            sha: sha.to_string(),
            parent_shas: vec![],
            author: Author {
                name: "Tester".into(),
                email: "t@example.com".into(),
            },
            committed_at: 1_700_000_000,
            file_changes: (0..n_files)
                .map(|i| FileChange {
                    path: format!("f{i}.rs"),
                    old_path: None,
                    added: 1,
                    deleted: 0,
                    change_type: ChangeType::Modified,
                    blob_sha: format!("blob{i}"),
                })
                .collect(),
        }
    }

    impl CommitSource for MockSource {
        fn head_sha(&self) -> Result<Option<String>> {
            Ok(self.head.clone())
        }

        fn for_each_commit(
            &self,
            _tip: &str,
            hidden: Option<&str>,
            f: &mut dyn FnMut(Commit) -> Result<()>,
        ) -> Result<()> {
            // Эмуляция: отдаём коммиты до первого, равного hidden (он и его
            // «предки» считаются уже обработанными).
            for c in &self.commits {
                if Some(c.sha.as_str()) == hidden {
                    break;
                }
                f(c.clone())?;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockStore {
        head: Option<String>,
        written: RefCell<Vec<(String, bool)>>,
        meta: Option<AnalysisMeta>,
    }

    impl Store for MockStore {
        fn last_head(&self) -> Result<Option<String>> {
            Ok(self.head.clone())
        }
        fn write_commit(&mut self, commit: &Commit, is_mechanical: bool) -> Result<()> {
            self.written
                .borrow_mut()
                .push((commit.sha.clone(), is_mechanical));
            Ok(())
        }
        fn write_meta(&mut self, meta: &AnalysisMeta) -> Result<()> {
            self.meta = Some(meta.clone());
            Ok(())
        }
        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn empty_repo_does_nothing() {
        let src = MockSource {
            head: None,
            commits: vec![],
        };
        let mut store = MockStore::default();
        let out = run_analysis(&src, &mut store, &Config::new("/r"), 0).unwrap();
        assert!(out.up_to_date);
        assert_eq!(out.new_commits, 0);
        assert!(store.written.borrow().is_empty());
    }

    #[test]
    fn cold_run_processes_all() {
        let src = MockSource {
            head: Some("c3".into()),
            commits: vec![commit("c3", 1), commit("c2", 1), commit("c1", 1)],
        };
        let mut store = MockStore::default();
        let out = run_analysis(&src, &mut store, &Config::new("/r"), 42).unwrap();
        assert_eq!(out.new_commits, 3);
        assert_eq!(out.head_sha.as_deref(), Some("c3"));
        assert_eq!(store.meta.as_ref().unwrap().head_sha, "c3");
        assert_eq!(store.meta.as_ref().unwrap().analyzed_at, 42);
    }

    #[test]
    fn incremental_skips_processed() {
        // Стор уже на c1; новый HEAD c3 → обработать только c3, c2.
        let src = MockSource {
            head: Some("c3".into()),
            commits: vec![commit("c3", 1), commit("c2", 1), commit("c1", 1)],
        };
        let mut store = MockStore {
            head: Some("c1".into()),
            ..Default::default()
        };
        let out = run_analysis(&src, &mut store, &Config::new("/r"), 0).unwrap();
        assert_eq!(out.new_commits, 2);
        let shas: Vec<_> = store
            .written
            .borrow()
            .iter()
            .map(|(s, _)| s.clone())
            .collect();
        assert_eq!(shas, vec!["c3", "c2"]);
    }

    #[test]
    fn up_to_date_when_head_unchanged() {
        let src = MockSource {
            head: Some("c3".into()),
            commits: vec![commit("c3", 1)],
        };
        let mut store = MockStore {
            head: Some("c3".into()),
            ..Default::default()
        };
        let out = run_analysis(&src, &mut store, &Config::new("/r"), 0).unwrap();
        assert!(out.up_to_date);
        assert_eq!(out.new_commits, 0);
        assert!(store.written.borrow().is_empty());
    }

    #[test]
    fn mechanical_flag_respects_threshold() {
        let src = MockSource {
            head: Some("big".into()),
            commits: vec![commit("big", 10)],
        };
        let mut store = MockStore::default();
        let mut cfg = Config::new("/r");
        cfg.mechanical_commit_max_files = Some(5);
        run_analysis(&src, &mut store, &cfg, 0).unwrap();
        assert!(store.written.borrow()[0].1); // 10 файлов > 5 → mechanical
    }
}
