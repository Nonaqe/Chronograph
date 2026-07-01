//! `chronograph-git` — обход истории git через gix.
//!
//! Реализует [`chronograph_core::CommitSource`]: открывает репозиторий, обходит
//! историю, извлекает (sha, author, timestamp, file_changes) с rename detection и
//! поддерживает инкрементальный «дельта»-обход (исключение уже обработанного HEAD).
//!
//! Все сигнатуры gix сверены по docs.rs для пина 0.85 (см. CONTEXT.md, раздел
//! «Лог обновлений версий»), а не написаны по памяти (skill `gix-patterns`).

mod source;

pub use source::GitSource;
