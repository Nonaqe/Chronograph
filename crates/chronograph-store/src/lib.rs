//! `chronograph-store` — слой хранилища на DuckDB.
//!
//! Реализует [`chronograph_core::Store`]: схема из `chronograph-tz.md` (раздел 7),
//! идемпотентная запись `commits`/`authors`/`file_changes`, чтение точки
//! инкрементального продолжения из `analysis_meta`.
//!
//! На Этапе 0 материализуются только сырые таблицы; аналитические
//! (`file_metrics`, `coupling`, ...) создаются пустыми (skeleton) и наполняются
//! на Этапах 1+.

mod store;

pub use store::DuckStore;
