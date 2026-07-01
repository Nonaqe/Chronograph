//! Тип ошибок ядра.
//!
//! Библиотечные крейты используют `thiserror` (правило CLAUDE.md). Конкретные
//! реализации трейтов ([`crate::source::CommitSource`], [`crate::store::Store`])
//! живут в крейтах с собственными ошибками (gix, duckdb), поэтому ядро
//! «оборачивает» их в боксированную ошибку, не завязываясь на чужие типы.

/// Боксированная ошибка реализации слоя (gix, duckdb и т.п.).
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Ошибка операций ядра Chronograph.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Ошибка слоя доступа к истории git (реализация [`crate::source::CommitSource`]).
    #[error("ошибка источника истории: {0}")]
    Source(#[source] BoxError),

    /// Ошибка слоя хранилища (реализация [`crate::store::Store`]).
    #[error("ошибка хранилища: {0}")]
    Store(#[source] BoxError),

    /// Некорректная или несогласованная конфигурация анализа.
    #[error("ошибка конфигурации: {0}")]
    Config(String),
}

impl Error {
    /// Обернуть ошибку реализации источника истории.
    pub fn source<E: Into<BoxError>>(err: E) -> Self {
        Error::Source(err.into())
    }

    /// Обернуть ошибку реализации хранилища.
    pub fn store<E: Into<BoxError>>(err: E) -> Self {
        Error::Store(err.into())
    }
}

/// Удобный alias `Result` ядра.
pub type Result<T> = std::result::Result<T, Error>;
