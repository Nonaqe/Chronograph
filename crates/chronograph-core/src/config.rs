//! Конфигурация анализа.
//!
//! `config_hash` пишется в `analysis_meta` — он должен быть **детерминирован** и
//! зависеть только от значений, влияющих на результат анализа (правило 4 CLAUDE.md).

use std::path::PathBuf;

/// Конфигурация одного прогона анализа.
///
/// Пороговые значения здесь — конфигурируемые поля, а не зашитые константы
/// (запрет «магических» порогов из CLAUDE.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Путь к анализируемому git-репозиторию.
    pub repo_path: PathBuf,
    /// Нижняя граница истории (unix-секунды UTC); `None` — вся история.
    pub since: Option<i64>,
    /// Glob-паттерны исключаемых путей (vendored/generated).
    pub exclude: Vec<String>,
    /// Инкрементальный режим (продолжать от закэшированного `head_sha`).
    pub incremental: bool,
    /// Порог «механического» коммита по числу изменённых файлов.
    ///
    /// `None` — эвристика выключена (`is_mechanical` всегда `false`). Конкретный
    /// дефолт ТЗ не задаёт (раздел 3.1: «> N файлов»), поэтому не выдумывается —
    /// см. открытый вопрос в `CONTEXT.md`.
    pub mechanical_commit_max_files: Option<u32>,
}

impl Config {
    /// Конфиг по умолчанию для заданного репозитория: вся история, инкрементально,
    /// без исключений и без эвристики механических коммитов.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Config {
            repo_path: repo_path.into(),
            since: None,
            exclude: Vec::new(),
            incremental: true,
            mechanical_commit_max_files: None,
        }
    }

    /// Детерминированный хэш значений, влияющих на результат анализа.
    ///
    /// Намеренно НЕ включает `repo_path` (путь не меняет содержимое анализа) и
    /// `incremental` (режим вычисления, не результат). Реализован как FNV-1a по
    /// каноничной строке — без внешних зависимостей и стабильно между сборками.
    pub fn config_hash(&self) -> String {
        let mut canon = String::new();
        match self.since {
            Some(ts) => canon.push_str(&format!("since={ts};")),
            None => canon.push_str("since=all;"),
        }
        // Порядок exclude влияет на каноничность — сортируем для стабильности.
        let mut excl = self.exclude.clone();
        excl.sort();
        canon.push_str("exclude=[");
        for g in &excl {
            canon.push_str(g);
            canon.push(',');
        }
        canon.push_str("];");
        match self.mechanical_commit_max_files {
            Some(n) => canon.push_str(&format!("mech_max_files={n};")),
            None => canon.push_str("mech_max_files=off;"),
        }
        fnv1a_hex(canon.as_bytes())
    }
}

/// FNV-1a (64-bit) → hex. Детерминированно и без зависимостей.
fn fnv1a_hex(bytes: &[u8]) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_hash_is_deterministic() {
        let a = Config::new("/repo/one");
        let b = Config::new("/repo/two");
        // repo_path не влияет на хэш — одинаковые настройки → одинаковый хэш.
        assert_eq!(a.config_hash(), b.config_hash());
    }

    #[test]
    fn config_hash_changes_with_settings() {
        let base = Config::new("/repo");
        let mut changed = base.clone();
        changed.since = Some(1_700_000_000);
        assert_ne!(base.config_hash(), changed.config_hash());
    }

    #[test]
    fn config_hash_independent_of_exclude_order() {
        let mut a = Config::new("/repo");
        a.exclude = vec!["a/*".into(), "b/*".into()];
        let mut b = Config::new("/repo");
        b.exclude = vec!["b/*".into(), "a/*".into()];
        assert_eq!(a.config_hash(), b.config_hash());
    }
}
