//! Трейт-граница для слоя доступа к истории git.
//!
//! Реализуется в `chronograph-git` (через gix). Ядро работает только с этим
//! трейтом и ничего не знает про gix — это держит зависимости по местам
//! (CLAUDE.md: core не тянет gix).

use crate::model::Commit;
use crate::Result;

/// Источник истории коммитов репозитория.
///
/// Реализация инкапсулирует обход истории; для инкрементальности оркестратор
/// передаёт `hidden` — SHA уже обработанного HEAD, чьи предки нужно исключить.
pub trait CommitSource {
    /// SHA текущего HEAD. `None`, если в репозитории ещё нет коммитов.
    fn head_sha(&self) -> Result<Option<String>>;

    /// Обойти коммиты, достижимые из `tip`, но не из `hidden`, и вызвать `f`
    /// на каждом.
    ///
    /// Порядок обхода детерминирован (требование воспроизводимости). Когда
    /// `hidden` = `None`, обходится вся история от `tip`; иначе — только коммиты,
    /// не достижимые из `hidden` (инкрементальный «дельта»-обход).
    ///
    /// `f` возвращает [`Result`], чтобы ошибка записи в стор прерывала обход.
    fn for_each_commit(
        &self,
        tip: &str,
        hidden: Option<&str>,
        f: &mut dyn FnMut(Commit) -> Result<()>,
    ) -> Result<()>;
}

/// Источник содержимого git-блобов по их SHA.
///
/// Реализуется в `chronograph-git` (через gix). Даёт аналитическому слою
/// (`chronograph-metrics`, complexity) байты файла на **конкретный коммит** —
/// контент-адресно, детерминированно, не с диска рабочего дерева. Так metrics
/// получает содержимое, не завися от gix напрямую.
pub trait BlobReader {
    /// Прочитать содержимое git-блоба по его hex-SHA.
    fn read_blob(&self, blob_sha: &str) -> Result<Vec<u8>>;
}

/// Один непрерывный участок blame: строки файла, которые в последний раз тронул
/// один и тот же коммит.
///
/// Чистые данные (без gix-типов) — так `chronograph-metrics` считает knowledge/
/// bus factor, не завися от gix (та же схема, что и с [`BlobReader`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameHunk {
    /// SHA коммита, последним изменившего эти строки.
    pub commit_sha: String,
    /// Сколько строк в участке (всегда ≥ 1).
    pub lines: u32,
}

/// Результат blame одного файла.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileBlame {
    /// Blame отработал. Участки (пусто — пустой/отсутствующий на коммите файл).
    Blamed(Vec<BlameHunk>),
    /// Blame НЕ удался (паника внутри gix-blame — upstream-баг). Файл выпадает из
    /// knowledge и должен считаться отдельно (не молчаливая потеря).
    Failed,
}

/// Источник построчной атрибуции (blame) файла на конкретный коммит.
///
/// Реализуется в `chronograph-git` (через gix `blame_file`). Отдаёт аналитическому
/// слою участки [`BlameHunk`] — «эти N строк последним тронул коммит X» — из
/// которых metrics агрегирует ownership и bus factor (§3.5 ТЗ), не завися от gix.
///
/// Дорого на больших файлах/репо: реализация вызывается лениво, только для живых
/// файлов на стадии knowledge, а не для всех файлов на каждом анализе.
pub trait BlameSource {
    /// Blame файла `path` в состоянии на коммит `at_commit` (hex-SHA, обычно HEAD).
    ///
    /// Возвращает участки в порядке реализации; агрегатор от порядка не зависит.
    /// Пустой результат — файла нет/он пуст на этом коммите (или blame упал —
    /// одиночный API это не различает; для учёта пропусков используй [`blame_many`]).
    ///
    /// [`blame_many`]: BlameSource::blame_many
    fn blame_lines(&self, path: &str, at_commit: &str) -> Result<Vec<BlameHunk>>;

    /// Blame нескольких файлов на один коммит `at_commit`.
    ///
    /// Возврат — по [`FileBlame`] на каждый путь, **в том же порядке, что `paths`**
    /// (детерминизм). [`FileBlame::Failed`] позволяет аналитическому слою СЧИТАТЬ
    /// файлы, на которых blame упал, а не терять их молча. Дефолт — последовательный
    /// (все — `Blamed`); реализации могут распараллелить и сигналить `Failed`.
    fn blame_many(&self, paths: &[String], at_commit: &str) -> Result<Vec<FileBlame>> {
        paths
            .iter()
            .map(|p| self.blame_lines(p, at_commit).map(FileBlame::Blamed))
            .collect()
    }
}
