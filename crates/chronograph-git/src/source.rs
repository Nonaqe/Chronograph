//! Реализация [`CommitSource`] поверх gix.

use chronograph_core::error::BoxError;
use chronograph_core::{
    BlobReader, ChangeType, Commit, CommitSource, Config, Error, FileChange, Result,
};
use gix::bstr::ByteSlice;
use gix::diff::blob::sources::byte_lines;
use gix::diff::blob::{Algorithm, Diff, InternedInput};
use gix::diff::tree_with_rewrites::Change;
use gix::diff::{Options as DiffOptions, Rewrites};
use globset::{Glob, GlobSet, GlobSetBuilder};

/// Размер кэша объектов gix (декодирование переиспользуемых объектов).
///
/// Перф-настройка, не метрика: skill `gix-patterns` помечает object cache как
/// обязательный — без него обход больших репо «ползёт». Значение подбирается
/// бенчмарком (Этап CI), пока — разумный дефолт.
const OBJECT_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Алгоритм построчного diff для подсчёта added/deleted.
///
/// `Histogram` детерминирован и даёт «интуитивные» хунки; выбран фиксированно ради
/// воспроизводимости (правило 4 CLAUDE.md) — не зависит от git-конфига репозитория.
const BLOB_ALGORITHM: Algorithm = Algorithm::Histogram;

/// Источник истории на gix.
///
/// Держит уже открытый репозиторий, конфигурацию rename detection и
/// скомпилированный набор исключающих glob'ов — всё, что нужно реализации
/// [`CommitSource`], чьи методы конфиг не принимают.
pub struct GitSource {
    repo: gix::Repository,
    rewrites: Rewrites,
    /// `None` — исключений нет; иначе пути, матчащие набор, отбрасываются.
    exclude: Option<GlobSet>,
}

impl GitSource {
    /// Открыть репозиторий по пути из конфига и подготовить источник.
    ///
    /// Включает object cache (обязателен для перфоманса) и фиксирует параметры
    /// rename detection детерминированно (similarity 50%, без copy-tracking) —
    /// не полагаясь на git-конфиг анализируемого репозитория.
    pub fn open(cfg: &Config) -> Result<Self> {
        let mut repo = gix::discover(&cfg.repo_path).map_err(se)?;
        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);

        // Явная, детерминированная конфигурация rewrite-трекинга.
        let rewrites = Rewrites {
            copies: None,
            percentage: Some(0.5),
            limit: 1000,
            track_empty: false,
        };

        let exclude = build_globset(&cfg.exclude)?;

        Ok(GitSource {
            repo,
            rewrites,
            exclude,
        })
    }

    /// Извлечь [`Commit`] ядра из info обхода и загруженного объекта коммита.
    fn build_commit(
        &self,
        info: &gix::revision::walk::Info<'_>,
        commit: &gix::Commit<'_>,
    ) -> Result<Commit> {
        let author = commit.author().map_err(se)?;
        let committed_at = commit.time().map_err(se)?.seconds;

        let file_changes = self.collect_file_changes(info, commit)?;

        Ok(Commit {
            sha: info.id.to_string(),
            parent_shas: info.parent_ids.iter().map(|p| p.to_string()).collect(),
            author: chronograph_core::Author {
                name: author.name.to_str_lossy().into_owned(),
                email: author.email.to_str_lossy().into_owned(),
            },
            committed_at,
            file_changes,
        })
    }

    /// Diff коммита против ПЕРВОГО родителя (для merge — только первый родитель;
    /// корневой — против пустого дерева) и маппинг изменений в [`FileChange`].
    ///
    /// Выбор «против первого родителя» детерминирован и соответствует обычной
    /// атрибуции изменений для churn; зафиксирован в CONTEXT.md.
    fn collect_file_changes(
        &self,
        info: &gix::revision::walk::Info<'_>,
        commit: &gix::Commit<'_>,
    ) -> Result<Vec<FileChange>> {
        let new_tree = commit.tree().map_err(se)?;

        let empty;
        let parent_tree = match info.parent_ids.first() {
            Some(pid) => self
                .repo
                .find_commit(*pid)
                .map_err(se)?
                .tree()
                .map_err(se)?,
            None => {
                empty = self.repo.empty_tree();
                empty
            }
        };

        let opts = DiffOptions::default().with_rewrites(Some(self.rewrites));
        let changes = self
            .repo
            .diff_tree_to_tree(Some(&parent_tree), Some(&new_tree), Some(opts))
            .map_err(se)?;

        let mut out = Vec::with_capacity(changes.len());
        for change in changes {
            if let Some(fc) = self.map_change(change)? {
                out.push(fc);
            }
        }
        Ok(out)
    }

    /// Перевести один gix-`Change` в [`FileChange`], считая added/deleted построчно.
    ///
    /// Возвращает `None`, если изменение не является файлом-блобом (gix tree-diff
    /// выдаёт ещё и записи поддеревьев-каталогов и сабмодулей — их пропускаем,
    /// иначе churn/hotspot загрязняются «изменениями директорий») или если путь
    /// исключён по glob.
    fn map_change(&self, change: Change) -> Result<Option<FileChange>> {
        let fc = match change {
            Change::Addition {
                location,
                id,
                entry_mode,
                ..
            } => {
                if !entry_mode.is_blob_or_symlink() {
                    return Ok(None);
                }
                let path = location.to_string();
                if self.is_excluded(&path) {
                    return Ok(None);
                }
                let (added, deleted) = self.line_delta(None, Some(id))?;
                FileChange {
                    path,
                    old_path: None,
                    added,
                    deleted,
                    change_type: ChangeType::Added,
                    blob_sha: id.to_string(),
                }
            }
            Change::Deletion {
                location,
                id,
                entry_mode,
                ..
            } => {
                if !entry_mode.is_blob_or_symlink() {
                    return Ok(None);
                }
                let path = location.to_string();
                if self.is_excluded(&path) {
                    return Ok(None);
                }
                let (added, deleted) = self.line_delta(Some(id), None)?;
                FileChange {
                    path,
                    old_path: None,
                    added,
                    deleted,
                    change_type: ChangeType::Deleted,
                    blob_sha: id.to_string(),
                }
            }
            Change::Modification {
                location,
                previous_id,
                id,
                entry_mode,
                ..
            } => {
                if !entry_mode.is_blob_or_symlink() {
                    return Ok(None);
                }
                let path = location.to_string();
                if self.is_excluded(&path) {
                    return Ok(None);
                }
                let (added, deleted) = self.line_delta(Some(previous_id), Some(id))?;
                FileChange {
                    path,
                    old_path: None,
                    added,
                    deleted,
                    change_type: ChangeType::Modified,
                    blob_sha: id.to_string(),
                }
            }
            Change::Rewrite {
                source_location,
                source_id,
                location,
                id,
                copy,
                entry_mode,
                ..
            } => {
                if !entry_mode.is_blob_or_symlink() {
                    return Ok(None);
                }
                let path = location.to_string();
                if self.is_excluded(&path) {
                    return Ok(None);
                }
                let (added, deleted) = self.line_delta(Some(source_id), Some(id))?;
                FileChange {
                    path,
                    old_path: Some(source_location.to_string()),
                    added,
                    deleted,
                    change_type: if copy {
                        ChangeType::Copied
                    } else {
                        ChangeType::Renamed
                    },
                    blob_sha: id.to_string(),
                }
            }
        };
        Ok(Some(fc))
    }

    /// Построчная дельта между старым и новым блобами (любой может отсутствовать:
    /// `None` = пустой контент → чистое добавление/удаление).
    fn line_delta(
        &self,
        old: Option<gix::ObjectId>,
        new: Option<gix::ObjectId>,
    ) -> Result<(u64, u64)> {
        let old_bytes = self.blob_bytes(old)?;
        let new_bytes = self.blob_bytes(new)?;
        let input = InternedInput::new(byte_lines(&old_bytes), byte_lines(&new_bytes));
        let diff = Diff::compute(BLOB_ALGORITHM, &input);
        Ok((diff.count_additions() as u64, diff.count_removals() as u64))
    }

    /// Содержимое блоба по id; `None` → пустой срез.
    fn blob_bytes(&self, id: Option<gix::ObjectId>) -> Result<Vec<u8>> {
        match id {
            None => Ok(Vec::new()),
            Some(id) => Ok(self.repo.find_object(id).map_err(se)?.data.clone()),
        }
    }

    fn is_excluded(&self, path: &str) -> bool {
        self.exclude.as_ref().is_some_and(|set| set.is_match(path))
    }
}

impl CommitSource for GitSource {
    fn head_sha(&self) -> Result<Option<String>> {
        // head() пробрасывает реальные ошибки; id() == None у «нерождённого»
        // HEAD (пустой репозиторий) → трактуем как отсутствие истории.
        let head = self.repo.head().map_err(se)?;
        Ok(head.id().map(|id| id.to_string()))
    }

    fn for_each_commit(
        &self,
        tip: &str,
        hidden: Option<&str>,
        f: &mut dyn FnMut(Commit) -> Result<()>,
    ) -> Result<()> {
        let tip_id = gix::ObjectId::from_hex(tip.as_bytes()).map_err(se)?;

        let mut platform = self.repo.rev_walk(Some(tip_id));
        if let Some(hidden) = hidden {
            let hidden_id = gix::ObjectId::from_hex(hidden.as_bytes()).map_err(se)?;
            platform = platform.with_hidden(Some(hidden_id));
        }

        let walk = platform.all().map_err(se)?;
        for info in walk {
            let info = info.map_err(se)?;
            let commit = self.repo.find_commit(info.id).map_err(se)?;
            let model = self.build_commit(&info, &commit)?;
            f(model)?;
        }
        Ok(())
    }
}

impl BlobReader for GitSource {
    fn read_blob(&self, blob_sha: &str) -> Result<Vec<u8>> {
        let oid = gix::ObjectId::from_hex(blob_sha.as_bytes()).map_err(se)?;
        Ok(self.repo.find_object(oid).map_err(se)?.data.clone())
    }
}

/// Скомпилировать набор исключающих glob'ов; `None`, если паттернов нет.
fn build_globset(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = Glob::new(p).map_err(|e| Error::Config(format!("плохой glob '{p}': {e}")))?;
        builder.add(glob);
    }
    let set = builder
        .build()
        .map_err(|e| Error::Config(format!("сборка glob-набора: {e}")))?;
    Ok(Some(set))
}

/// Обернуть ошибку gix в ошибку источника ядра.
fn se<E: Into<BoxError>>(e: E) -> Error {
    Error::source(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// Выполнить git-команду в каталоге репозитория (только для построения фикстур).
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            // Детерминированные имена/даты автора и коммиттера.
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

    /// Построить синтетический репозиторий с известной историей.
    ///
    /// c1: добавить a.txt (2 строки)
    /// c2: изменить a.txt (+1 строка), добавить b.txt
    /// c3: переименовать b.txt → c.txt (без изменения содержимого)
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

    fn collect(src: &GitSource, tip: &str, hidden: Option<&str>) -> Vec<Commit> {
        let mut out = Vec::new();
        src.for_each_commit(tip, hidden, &mut |c| {
            out.push(c);
            Ok(())
        })
        .unwrap();
        out
    }

    #[test]
    fn skips_directory_tree_entries() {
        // gix tree-diff выдаёт и записи поддеревьев-каталогов; в file_changes
        // должны попадать только файлы-блобы, а не директории.
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::create_dir(p.join("sub")).unwrap();
        write(p, "sub/inner.txt", "x\n");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "c1"]);

        let cfg = Config::new(p);
        let src = GitSource::open(&cfg).unwrap();
        let head = src.head_sha().unwrap().unwrap();
        let commits = collect(&src, &head, None);

        let paths: Vec<&str> = commits[0]
            .file_changes
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(paths.contains(&"sub/inner.txt"), "файл-блоб записан");
        assert!(
            !paths.contains(&"sub"),
            "директория не должна попадать в file_changes"
        );
        assert_eq!(
            commits[0].file_changes.len(),
            1,
            "ровно один файл, без дерева"
        );
    }

    #[test]
    fn walks_full_history_and_extracts_changes() {
        let dir = build_fixture();
        let cfg = Config::new(dir.path());
        let src = GitSource::open(&cfg).unwrap();

        let head = src.head_sha().unwrap().expect("история есть");
        let commits = collect(&src, &head, None);

        // Три коммита, newest-first.
        assert_eq!(commits.len(), 3);
        let messages_order: Vec<_> = commits.iter().map(|c| c.file_changes.len()).collect();
        // c3 (rename) → 1 change; c2 → 2 changes; c1 → 1 change.
        assert_eq!(messages_order, vec![1, 2, 1]);

        // Автор извлечён.
        assert_eq!(commits[0].author.email, "fixture@example.com");

        // c1 (последний в обходе) — добавление a.txt на 2 строки.
        let c1 = commits.last().unwrap();
        assert_eq!(c1.parent_shas.len(), 0); // корневой
        let a = &c1.file_changes[0];
        assert_eq!(a.path, "a.txt");
        assert_eq!(a.change_type, ChangeType::Added);
        assert_eq!(a.added, 2);
        assert_eq!(a.deleted, 0);

        // c2 — модификация a.txt (+1) и добавление b.txt.
        let c2 = &commits[1];
        let a_mod = c2.file_changes.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(a_mod.change_type, ChangeType::Modified);
        assert_eq!(a_mod.added, 1);
        assert_eq!(a_mod.deleted, 0);

        // c3 — переименование b.txt → c.txt, обнаружено rename detection.
        let c3 = &commits[0];
        let r = &c3.file_changes[0];
        assert_eq!(r.change_type, ChangeType::Renamed);
        assert_eq!(r.path, "c.txt");
        assert_eq!(r.old_path.as_deref(), Some("b.txt"));
    }

    #[test]
    fn incremental_walk_excludes_hidden_ancestors() {
        let dir = build_fixture();
        let cfg = Config::new(dir.path());
        let src = GitSource::open(&cfg).unwrap();

        let head = src.head_sha().unwrap().unwrap();
        let all = collect(&src, &head, None);
        assert_eq!(all.len(), 3);

        // «Уже обработан» самый старый коммит (c1) → дельта = c3, c2.
        let c1_sha = all.last().unwrap().sha.clone();
        let delta = collect(&src, &head, Some(&c1_sha));
        assert_eq!(delta.len(), 2);
        assert!(delta.iter().all(|c| c.sha != c1_sha));
    }

    #[test]
    fn empty_repo_has_no_head() {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        let cfg = Config::new(dir.path());
        let src = GitSource::open(&cfg).unwrap();
        assert_eq!(src.head_sha().unwrap(), None);
    }

    #[test]
    fn exclude_glob_drops_paths() {
        let dir = build_fixture();
        let mut cfg = Config::new(dir.path());
        cfg.exclude = vec!["*.txt".into()];
        let src = GitSource::open(&cfg).unwrap();

        let head = src.head_sha().unwrap().unwrap();
        let commits = collect(&src, &head, None);
        // Все пути — *.txt → после исключения изменений не остаётся.
        assert!(commits.iter().all(|c| c.file_changes.is_empty()));
    }
}
