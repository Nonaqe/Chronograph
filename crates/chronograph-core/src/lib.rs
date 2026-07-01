//! `chronograph-core` — ядро модели данных и оркестрации Chronograph.
//!
//! Этот крейт намеренно «лёгкий»: он не зависит от gix, tree-sitter или duckdb.
//! Он определяет:
//! - модель данных истории git ([`model`]): [`Commit`], [`FileChange`], [`Author`];
//! - конфигурацию анализа ([`config`]): [`Config`];
//! - трейты-границы между слоями ([`source::CommitSource`], [`store::Store`]);
//! - оркестрацию инкрементального анализа ([`analyze::run_analysis`]).
//!
//! Конкретные реализации трейтов живут в других крейтах
//! (`chronograph-git` наполняет историю, `chronograph-store` пишет в DuckDB),
//! а `chronograph-cli` связывает их вместе.

pub mod analyze;
pub mod config;
pub mod error;
pub mod model;
pub mod source;
pub mod store;

pub use analyze::{run_analysis, AnalysisOutcome};
pub use config::Config;
pub use error::{Error, Result};
pub use model::{AnalysisMeta, Author, ChangeType, Commit, FileChange};
pub use source::{BlobReader, CommitSource};
pub use store::Store;

/// Версия движка, попадающая в `analysis_meta` каждого отчёта (детерминизм).
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
