//! Конфигурация метрик.

/// Конфигурация churn-агрегаций.
///
/// Окна (в днях) — конфигурируемы; дефолты 30/90/365 взяты прямо из ТЗ (раздел
/// 3.1), не выдуманы. Окна отсчитываются назад от максимального `committed_at`
/// в истории (последняя активность репо), а не от wall-clock — ради
/// детерминизма (правило 4 CLAUDE.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChurnConfig {
    /// Короткое окно, дней (дефолт 30).
    pub window_recent_days: u32,
    /// Среднее окно, дней (дефолт 90).
    pub window_mid_days: u32,
    /// Длинное окно, дней (дефолт 365).
    pub window_long_days: u32,
    /// Исключать ли «механические» коммиты (`is_mechanical`) из churn.
    ///
    /// Дефолт `true` — это и есть назначение флага (ТЗ 3.1: гигантские/механические
    /// коммиты искажают churn). При выключенной эвристике (`mechanical_commit_max_files
    /// = None` на анализе) механических коммитов попросту нет, и фильтр ни на что
    /// не влияет.
    pub exclude_mechanical: bool,
}

impl Default for ChurnConfig {
    fn default() -> Self {
        ChurnConfig {
            window_recent_days: 30,
            window_mid_days: 90,
            window_long_days: 365,
            exclude_mechanical: true,
        }
    }
}
