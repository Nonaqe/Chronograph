//! `chronograph-report` — self-contained HTML-репорт.
//!
//! Читает материализованные таблицы (`file_metrics`, `coupling`) из стора и
//! рендерит один `report.html`: Overview + Hotspots treemap (server-side SVG) +
//! Coupling таблица. Все ассеты (CSS) встроены через `include_str!` — **ноль
//! внешних/CDN-запросов** (ТЗ §6.2). Рендер детерминирован (байт-в-байт при
//! одинаковых данных): фикс. форматирование чисел, отсортированный вход,
//! server-side SVG, без wall-clock времени в выводе.

pub mod data;
pub mod render;
pub mod treemap;

pub use data::ReportData;
pub use render::render_html;

use chronograph_core::Result;
use chronograph_store::DuckStore;

/// Сгенерировать `report.html` из материализованных таблиц стора и записать в `out`.
pub fn generate(store: &DuckStore, out: impl AsRef<std::path::Path>) -> Result<()> {
    let data = ReportData::from_store(store)?;
    let html = render_html(&data);
    std::fs::write(out.as_ref(), html).map_err(|e| chronograph_core::Error::store(Box::new(e)))?;
    Ok(())
}
