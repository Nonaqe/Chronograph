//! Модель данных истории git.
//!
//! Все таймстемпы — `i64` unix-секунды в **UTC** (правило детерминизма CLAUDE.md).
//! Имена полей соответствуют схеме DuckDB из `chronograph-tz.md`, раздел 7.

/// Тип изменения файла в коммите (`change_type` в схеме: A/M/D/R/C).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeType {
    /// Файл добавлен (A).
    Added,
    /// Файл изменён (M).
    Modified,
    /// Файл удалён (D).
    Deleted,
    /// Файл переименован (R) — `old_path` в [`FileChange`] хранит прежний путь.
    Renamed,
    /// Файл скопирован (C).
    Copied,
}

impl ChangeType {
    /// Однобуквенный код, как в git и в колонке `change_type` DuckDB.
    pub fn code(self) -> char {
        match self {
            ChangeType::Added => 'A',
            ChangeType::Modified => 'M',
            ChangeType::Deleted => 'D',
            ChangeType::Renamed => 'R',
            ChangeType::Copied => 'C',
        }
    }
}

/// Автор коммита. На Этапе 0 нормализуется по email; `.mailmap` — Этап 4.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Author {
    /// Отображаемое имя.
    pub name: String,
    /// Email (ключ нормализации до подключения mailmap).
    pub email: String,
}

/// Одно изменение файла внутри коммита.
///
/// Соответствует строке таблицы `file_changes`. `path` — путь *после* резолва
/// переименований; `old_path` заполняется только для [`ChangeType::Renamed`]/
/// [`ChangeType::Copied`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Актуальный путь файла после rename-резолва.
    pub path: String,
    /// Прежний путь (только для rename/copy).
    pub old_path: Option<String>,
    /// Добавлено строк.
    pub added: u64,
    /// Удалено строк.
    pub deleted: u64,
    /// Тип изменения.
    pub change_type: ChangeType,
    /// SHA git-блоба этого состояния файла (hex).
    ///
    /// Для A/M/R/C — oid нового содержимого; для D — oid удалённого блоба.
    /// Контент-адресный: по нему `BlobReader` достаёт байты детерминированно
    /// (используется для complexity — контент на конкретный коммит, не с диска).
    pub blob_sha: String,
}

/// Коммит с уже извлечёнными метаданными и списком изменений файлов.
///
/// Это единица потока от [`crate::source::CommitSource`] к [`crate::store::Store`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Полный SHA-1 коммита (hex).
    pub sha: String,
    /// SHA родителей (для merge-коммитов их несколько; пусто для корневого).
    pub parent_shas: Vec<String>,
    /// Автор изменений.
    pub author: Author,
    /// Время коммита — unix-секунды, UTC.
    pub committed_at: i64,
    /// Изменения файлов в этом коммите.
    pub file_changes: Vec<FileChange>,
}

impl Commit {
    /// Число изменённых файлов (`files_changed` в таблице `commits`).
    pub fn files_changed(&self) -> usize {
        self.file_changes.len()
    }
}

/// Метаданные одного прогона анализа (таблица `analysis_meta`).
///
/// Пишутся в каждый отчёт ради воспроизводимости. `analyzed_at` — единственное
/// поле, легитимно различающееся между двумя прогонами одного репо.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisMeta {
    /// Версия движка ([`crate::ENGINE_VERSION`]).
    pub engine_version: String,
    /// Хэш конфигурации анализа ([`crate::Config::config_hash`]).
    pub config_hash: String,
    /// Время прогона — unix-секунды, UTC.
    pub analyzed_at: i64,
    /// HEAD, на котором закончился прогон (точка инкрементального продолжения).
    pub head_sha: String,
}
