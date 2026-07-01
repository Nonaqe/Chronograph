//! `chronograph-metrics` — агрегатные метрики над сырыми данными из стора.
//!
//! Каждая метрика — отдельный модуль. Этап 1: [`churn`] (изменчивость),
//! [`complexity`] (сложность из git-объектов), далее hotspot. Граница: этот крейт
//! читает плоские таблицы из `chronograph-store` и НЕ зависит от `chronograph-git`
//! (CLAUDE.md). Содержимое файлов для complexity приходит через трейт
//! [`chronograph_core::BlobReader`] — байты на вход, gix здесь не фигурирует.

pub mod churn;
pub mod complexity;
pub mod config;
pub mod coupling;
pub mod hotspot;
pub mod materialize;
mod paths;

pub use churn::{compute_churn, FileChurn};
pub use complexity::{compute_complexity, FileComplexityRow};
pub use config::ChurnConfig;
pub use coupling::{compute_coupling, Coupling, CouplingConfig};
pub use hotspot::{compute_hotspots, ChurnWindow, Hotspot, HotspotConfig};
pub use materialize::{materialize, MaterializeConfig, MaterializeSummary};

use chronograph_core::error::BoxError;
use chronograph_core::Error;

/// Обернуть ошибку duckdb/чтения в ошибку ядра (слой хранилища).
pub(crate) fn store_err<E: Into<BoxError>>(e: E) -> Error {
    Error::store(e)
}
