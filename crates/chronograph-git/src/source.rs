//! Реализация [`CommitSource`] поверх gix.

use chronograph_core::error::BoxError;
use chronograph_core::{
    BlameHunk, BlameSource, BlobReader, ChangeType, Commit, CommitSource, Config, Error, FileBlame,
    FileChange, Result,
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
    /// Путь, по которому репозиторий был найден — чтобы параллельный blame мог
    /// открыть НЕЗАВИСИМЫЙ `Repository` на каждый rayon-поток (свой ODB, без гонки
    /// ленивой загрузки паков в общем сторе).
    repo_path: std::path::PathBuf,
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

        // Снимок `.mailmap` (канонизация авторов, §3.5 ТЗ) НЕ хранится здесь:
        // diff-воркеры параллельного обхода открывают свой снимок per-thread
        // (см. for_each_commit). Без .mailmap-файла resolve — тождество.
        Ok(GitSource {
            repo,
            repo_path: cfg.repo_path.clone(),
            rewrites,
            exclude,
        })
    }
}

/// Построить [`Commit`] ядра: mailmap-канонизация автора + diff против первого
/// родителя. Свободная функция: параллельный diff зовёт её из rayon-воркеров с
/// НЕЗАВИСИМЫМ per-thread `Repository` (gix не Send) — как blame.
///
/// Diff коммита — чистая функция пары деревьев (коммит, первый родитель): никакого
/// состояния между коммитами, поэтому параллелизация по коммитам безопасна.
fn build_commit_standalone(
    repo: &gix::Repository,
    mailmap: &gix::mailmap::Snapshot,
    rewrites: Rewrites,
    exclude: Option<&GlobSet>,
    id: gix::ObjectId,
    parent_ids: &[gix::ObjectId],
    commit: &gix::Commit<'_>,
) -> Result<Commit> {
    // Канонизируем автора через mailmap ДО построения модели: несколько email
    // одного человека схлопываются в одну личность (§3.5 ТЗ). Без .mailmap —
    // тождество (аддитивно для метрик, не читающих автора).
    let raw_author = commit.author().map_err(se)?;
    let author = mailmap.resolve(raw_author);
    let committed_at = commit.time().map_err(se)?.seconds;

    let file_changes = collect_file_changes(repo, rewrites, exclude, parent_ids, commit)?;

    Ok(Commit {
        sha: id.to_string(),
        parent_shas: parent_ids.iter().map(|p| p.to_string()).collect(),
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
    repo: &gix::Repository,
    rewrites: Rewrites,
    exclude: Option<&GlobSet>,
    parent_ids: &[gix::ObjectId],
    commit: &gix::Commit<'_>,
) -> Result<Vec<FileChange>> {
    let new_tree = commit.tree().map_err(se)?;

    let empty;
    let parent_tree = match parent_ids.first() {
        Some(pid) => repo.find_commit(*pid).map_err(se)?.tree().map_err(se)?,
        None => {
            empty = repo.empty_tree();
            empty
        }
    };

    let opts = DiffOptions::default().with_rewrites(Some(rewrites));
    let changes = repo
        .diff_tree_to_tree(Some(&parent_tree), Some(&new_tree), Some(opts))
        .map_err(se)?;

    let mut out = Vec::with_capacity(changes.len());
    for change in changes {
        if let Some(fc) = map_change(repo, exclude, change)? {
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
fn map_change(
    repo: &gix::Repository,
    exclude: Option<&GlobSet>,
    change: Change,
) -> Result<Option<FileChange>> {
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
            if is_path_excluded(exclude, &path) {
                return Ok(None);
            }
            let (added, deleted) = line_delta(repo, None, Some(id))?;
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
            if is_path_excluded(exclude, &path) {
                return Ok(None);
            }
            let (added, deleted) = line_delta(repo, Some(id), None)?;
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
            if is_path_excluded(exclude, &path) {
                return Ok(None);
            }
            let (added, deleted) = line_delta(repo, Some(previous_id), Some(id))?;
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
            if is_path_excluded(exclude, &path) {
                return Ok(None);
            }
            let (added, deleted) = line_delta(repo, Some(source_id), Some(id))?;
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

/// Окно git-детекта бинарности: NUL-байт в первых 8000 байт (семантика
/// `buffer_is_binary` в git). Не конфигурируемый метрический порог, а стандартное
/// git-поведение определения «текстовости».
const BINARY_SNIFF_LEN: usize = 8000;

/// Бинарный ли контент — как это определяет git (NUL в начале файла).
fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_SNIFF_LEN)].contains(&0)
}

/// Построчная дельта между старым и новым блобами (любой может отсутствовать:
/// `None` = пустой контент → чистое добавление/удаление).
///
/// БИНАРНИКИ (git-детект: NUL в первых 8000 байт) построчно НЕ диффаются —
/// added/deleted = 0, как `git diff --shortstat` показывает «-» для binary. Это и
/// семантика (у бинарника нет «строк» — прежние счётчики были мусором в churn), и
/// жизнеспособность: Histogram-diff на 57-МБ БД в истории (реальный случай
/// OmniRoute: .codegraph/codegraph.db) жевал CPU-часы и вешал analyze.
fn line_delta(
    repo: &gix::Repository,
    old: Option<gix::ObjectId>,
    new: Option<gix::ObjectId>,
) -> Result<(u64, u64)> {
    let old_bytes = blob_bytes(repo, old)?;
    let new_bytes = blob_bytes(repo, new)?;
    if is_binary(&old_bytes) || is_binary(&new_bytes) {
        return Ok((0, 0));
    }
    let input = InternedInput::new(byte_lines(&old_bytes), byte_lines(&new_bytes));
    let diff = Diff::compute(BLOB_ALGORITHM, &input);
    Ok((diff.count_additions() as u64, diff.count_removals() as u64))
}

/// Содержимое блоба по id; `None` → пустой срез.
fn blob_bytes(repo: &gix::Repository, id: Option<gix::ObjectId>) -> Result<Vec<u8>> {
    match id {
        None => Ok(Vec::new()),
        Some(id) => Ok(repo.find_object(id).map_err(se)?.data.clone()),
    }
}

fn is_path_excluded(exclude: Option<&GlobSet>, path: &str) -> bool {
    exclude.is_some_and(|set| set.is_match(path))
}

impl CommitSource for GitSource {
    fn head_sha(&self) -> Result<Option<String>> {
        // head() пробрасывает реальные ошибки; id() == None у «нерождённого»
        // HEAD (пустой репозиторий) → трактуем как отсутствие истории.
        let head = self.repo.head().map_err(se)?;
        Ok(head.id().map(|id| id.to_string()))
    }

    /// Обход в ДВЕ фазы (см. CONTEXT.md, оптимизация analyze):
    ///
    /// 1. ПОСЛЕДОВАТЕЛЬНЫЙ rev-walk собирает порядок коммитов (id + родители) —
    ///    ровно та последовательность, что и раньше; инкрементальность (`hidden`)
    ///    и walk-порядок не тронуты (ограничение skill gix-patterns соблюдено).
    /// 2. Diff per-commit — ПАРАЛЛЕЛЬНО чанками (rayon): diff — чистая функция пары
    ///    деревьев, независим между коммитами (профилирование: ~31% analyze).
    ///    Каждый поток — свой независимый `Repository` (`map_init`, как blame —
    ///    общий ODB-store гонялся на ленивой загрузке паков).
    ///
    /// `f` вызывается строго в walk-порядке (par collect сохраняет порядок входа) —
    /// назначение author_id по порядку первого появления и байт-идентичность
    /// вывода не меняются. Чанки ограничивают пиковую память (не держим все
    /// `Commit` гигантского репо разом).
    fn for_each_commit(
        &self,
        tip: &str,
        hidden: Option<&str>,
        f: &mut dyn FnMut(Commit) -> Result<()>,
    ) -> Result<()> {
        use rayon::prelude::*;

        /// Коммитов на чанк параллельного diff. Перф-настройка (память vs
        /// амортизация), НЕ влияет на порядок/результат при любом значении.
        const DIFF_CHUNK: usize = 512;

        let tip_id = gix::ObjectId::from_hex(tip.as_bytes()).map_err(se)?;

        let mut platform = self.repo.rev_walk(Some(tip_id));
        if let Some(hidden) = hidden {
            let hidden_id = gix::ObjectId::from_hex(hidden.as_bytes()).map_err(se)?;
            platform = platform.with_hidden(Some(hidden_id));
        }

        // Фаза 1: последовательный walk — только порядок (id + родители), без diff.
        let walk = platform.all().map_err(se)?;
        let mut order: Vec<(gix::ObjectId, Vec<gix::ObjectId>)> = Vec::new();
        for info in walk {
            let info = info.map_err(se)?;
            order.push((info.id, info.parent_ids.iter().copied().collect()));
        }

        // Фаза 2+3: параллельный diff чанками, выдача f строго в walk-порядке.
        let repo_path = &self.repo_path;
        let rewrites = self.rewrites;
        let exclude = self.exclude.as_ref();
        for chunk in order.chunks(DIFF_CHUNK) {
            let commits: Vec<Result<Commit>> = chunk
                .par_iter()
                .map_init(
                    || {
                        // Инвариант: репо уже открывалось в GitSource::open по тому же
                        // пути → повторное открытие не может провалиться.
                        let mut repo = gix::discover(repo_path)
                            .expect("репозиторий уже открывался в GitSource::open");
                        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
                        let mailmap = repo.open_mailmap();
                        (repo, mailmap)
                    },
                    |(repo, mailmap), (id, parents)| {
                        let commit = repo.find_commit(*id).map_err(se)?;
                        build_commit_standalone(
                            repo, mailmap, rewrites, exclude, *id, parents, &commit,
                        )
                    },
                )
                .collect();
            for model in commits {
                f(model?)?;
            }
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

impl BlameSource for GitSource {
    fn blame_lines(&self, path: &str, at_commit: &str) -> Result<Vec<BlameHunk>> {
        let suspect = gix::ObjectId::from_hex(at_commit.as_bytes()).map_err(se)?;
        // Одиночный API не различает пропуск: паника → пустой результат.
        Ok(
            match with_quiet_panic_hook(|| blame_one(&self.repo, path, suspect))? {
                FileBlame::Blamed(hunks) => hunks,
                FileBlame::Failed => Vec::new(),
            },
        )
    }

    /// Параллельный blame по файлам (rayon).
    ///
    /// blame по разным файлам независим и CPU-bound (профилирование: blame — узкое
    /// место, ×20 к остальным метрикам вместе).
    ///
    /// Каждый rayon-поток открывает СВОЙ независимый `Repository` (`map_init` — раз на
    /// поток, не на файл). Почему не общий `ThreadSafeRepository`+`to_thread_local`:
    /// потоки делили бы один ODB-store и гонялись на ЛЕНИВОЙ загрузке pack-индексов
    /// (gix грузит паки при первом промахе), что давало интермиттентный
    /// «object could not be found» на холодном кэше. Независимые сторы это исключают.
    /// Порядок результата = порядку `paths` (`map_init(...).collect()` сохраняет) —
    /// детерминизм не нарушается.
    fn blame_many(&self, paths: &[String], at_commit: &str) -> Result<Vec<FileBlame>> {
        use rayon::prelude::*;

        let suspect = gix::ObjectId::from_hex(at_commit.as_bytes()).map_err(se)?;
        let repo_path = &self.repo_path;

        // Тихий panic-hook ставим ОДИН РАЗ на всю параллельную секцию: take/set_hook
        // глобальны и не потокобезопасны, внутри blame_one их трогать нельзя.
        let results: Vec<Result<FileBlame>> = with_quiet_panic_hook(|| {
            paths
                .par_iter()
                .map_init(
                    || {
                        // Инвариант: репозиторий уже успешно открыт в GitSource::open по
                        // тому же пути → повторное открытие не может провалиться (кроме
                        // гонки удаления репо во время анализа — не рядовой случай).
                        let mut repo = gix::discover(repo_path)
                            .expect("репозиторий уже открывался в GitSource::open");
                        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
                        repo
                    },
                    |repo, path| blame_one(repo, path, suspect),
                )
                .collect()
        });
        results.into_iter().collect()
    }
}

/// Blame одного файла на коммит `suspect` через gix `blame_file`.
///
/// Опции фиксированы детерминированно: `Histogram` (как в построчном churn),
/// весь файл (`BlameRanges::default()`), без rename-following в v1. Участки
/// схлопываются по `commit_id`.
///
/// Устойчивость (различаем в [`FileBlame`], чтобы аналитический слой мог СЧИТАТЬ
/// пропуски, а не терять молча):
/// - файла нет в дереве коммита (canonical over-approximation) → `Blamed(пусто)`
///   (легитимно, не «упал»);
/// - gix-blame 0.15 на некоторых входах ПАНИКУЕТ (index OOB, upstream-баг) —
///   изолируем `catch_unwind` → `Failed` (детерминировано: тот же вход → та же паника);
/// - реальная (не паника) ошибка blame → `Err` (пробрасываем, это не рядовой случай).
///
/// panic-hook НЕ трогает (глобален, не потокобезопасен) — его заглушает вызывающий.
fn blame_one(repo: &gix::Repository, path: &str, suspect: gix::ObjectId) -> Result<FileBlame> {
    let tree = repo.find_commit(suspect).map_err(se)?.tree().map_err(se)?;
    if tree.lookup_entry_by_path(path).map_err(se)?.is_none() {
        return Ok(FileBlame::Blamed(Vec::new()));
    }

    let opts = gix::repository::blame_file::Options {
        diff_algorithm: Some(BLOB_ALGORITHM),
        ..Default::default()
    };
    let path_bstr = path.as_bytes().as_bstr();
    // Ошибку сворачиваем в маленький core::Error ВНУТРИ замыкания: иначе Err — это
    // крупный `blame_file::Error` (clippy `result_large_err`).
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        repo.blame_file(path_bstr, suspect, opts).map_err(se)
    }));
    let outcome = match caught {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(FileBlame::Failed), // паника gix-blame → пропуск + учёт
    };

    Ok(FileBlame::Blamed(
        outcome
            .entries
            .into_iter()
            .map(|e| BlameHunk {
                commit_sha: e.commit_id.to_string(),
                lines: e.len.get(),
            })
            .collect(),
    ))
}

/// Выполнить `f`, заглушив дефолтный panic-hook на время (бэктрейсы пойманных
/// gix-blame паник не засоряют вывод). Хук глобальный — ставим/снимаем один раз.
fn with_quiet_panic_hook<T>(f: impl FnOnce() -> T) -> T {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = f();
    std::panic::set_hook(prev);
    out
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
    fn blame_lines_attributes_and_skips_missing() {
        use chronograph_core::BlameSource;

        let dir = build_fixture();
        let cfg = Config::new(dir.path());
        let src = GitSource::open(&cfg).unwrap();
        let head = src.head_sha().unwrap().unwrap();

        // a.txt существует на HEAD (3 строки) → blame отдаёт участки.
        let hunks = src.blame_lines("a.txt", &head).unwrap();
        let total: u32 = hunks.iter().map(|h| h.lines).sum();
        assert_eq!(total, 3, "a.txt на HEAD — 3 строки");

        // Отсутствующий на HEAD путь → пусто, без ошибки (guard существования).
        let missing = src.blame_lines("does-not-exist.rs", &head).unwrap();
        assert!(missing.is_empty());

        // b.txt был переименован в c.txt → на HEAD его нет → пусто.
        let renamed_away = src.blame_lines("b.txt", &head).unwrap();
        assert!(renamed_away.is_empty());
    }

    #[test]
    fn binary_files_have_zero_line_counts() {
        // Бинарник (NUL-байты) не диффается построчно: added/deleted = 0 (git-
        // семантика), но file_change записан (коммиты по бинарям видны в churn).
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("blob.bin"), b"\x00\x01\x02binary\x00data").unwrap();
        write(p, "text.txt", "line1\nline2\n");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "c1"]);

        let cfg = Config::new(p);
        let src = GitSource::open(&cfg).unwrap();
        let head = src.head_sha().unwrap().unwrap();
        let commits = collect(&src, &head, None);

        let bin = commits[0]
            .file_changes
            .iter()
            .find(|f| f.path == "blob.bin")
            .expect("бинарник записан");
        assert_eq!((bin.added, bin.deleted), (0, 0), "бинарник без строк");

        let txt = commits[0]
            .file_changes
            .iter()
            .find(|f| f.path == "text.txt")
            .expect("текст записан");
        assert_eq!(txt.added, 2, "текст диффается как раньше");
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
