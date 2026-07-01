//! `chronograph-lang` — complexity по AST через tree-sitter.
//!
//! Считает сложность файла обходом синтаксического дерева (а не регэкспами,
//! принцип 2.3 ТЗ). Поддержаны 4 языка (JS/TS, Python, Go, Rust) —
//! **cyclomatic complexity**; для остальных — грубый indentation-based fallback
//! (ТЗ 3.2: «не нулевой сигнал»).
//!
//! Крейт чистый: на вход — байты исходника, наружу — [`FileComplexity`]. Чтение
//! файлов и склейка с churn — выше по стеку (cli/metrics).

pub mod complexity;

pub use complexity::{file_complexity, ComplexityMethod, FileComplexity, SupportedLanguage};
