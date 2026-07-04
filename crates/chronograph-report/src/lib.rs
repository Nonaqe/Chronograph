//! `chronograph-report` — self-contained HTML-репорт.
//!
//! Читает материализованные таблицы (`file_metrics`, `coupling`, `knowledge`,
//! `module_bus_factor`) из стора и рендерит один `report.html`: Overview + Hotspots
//! treemap (server-side SVG) + Coupling + Knowledge/Bus factor. Все ассеты (CSS)
//! встроены через `include_str!` — **ноль внешних/CDN-запросов** (ТЗ §6.2). Рендер
//! детерминирован (байт-в-байт при одинаковых данных): фикс. форматирование чисел,
//! отсортированный вход, server-side SVG, без wall-clock времени в выводе. Авторы в
//! knowledge-секции анонимизированы (Author #N) — принцип 2.4.
//!
//! Вторая точка выхода — [`export`]: детерминированный JSON-экспорт тех же таблиц
//! плюс поток событий per-commit (§4.1/§6.1) — потребляется Web UI (`web/`).

pub mod data;
pub mod export;
pub mod render;
pub mod treemap;

pub use data::ReportData;
pub use export::{export_json, ExportOptions, EXPORT_SCHEMA_VERSION};
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

/// Сгенерировать `chronograph.json` (детерминированный JSON-экспорт для Web UI,
/// §4.1/§6.1) из материализованных таблиц стора и записать в `out`.
pub fn generate_json(
    store: &DuckStore,
    out: impl AsRef<std::path::Path>,
    opts: &ExportOptions,
) -> Result<()> {
    let json = export_json(store, opts)?;
    std::fs::write(out.as_ref(), json).map_err(|e| chronograph_core::Error::store(Box::new(e)))?;
    Ok(())
}
