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

/// Конфигурация knowledge / bus factor (§3.5 ТЗ).
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeConfig {
    /// Доля знаний, которую должны покрыть авторы, чтобы задать bus factor.
    ///
    /// bus_factor = минимальное число топ-владельцев, чья суммарная доля СТРОГО
    /// превышает этот порог. Дефолт `0.5` — прямо из §3.5 ТЗ («> 50% знаний о
    /// модуле»). Конфигурируем: CLAUDE.md запрещает зашивать пороги константой.
    pub bus_factor_threshold: f64,
    /// Бюджет blame на файл, см. [`DEFAULT_BLAME_BUDGET`]. `0` — безлимит.
    pub blame_budget: u64,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        KnowledgeConfig {
            bus_factor_threshold: 0.5,
            blame_budget: DEFAULT_BLAME_BUDGET,
        }
    }
}

/// Бюджет стоимости blame одного файла: `cost = revisions × total_added`.
///
/// Файлы дороже бюджета НЕ блеймятся (выпадают из knowledge/age) и ЯВНО
/// учитываются с причиной — в счётчике «blame skipped» и списке пропусков отчёта.
/// Зачем: blame одного гигантского частопеременного файла (CHANGELOG 1.3МБ × 618
/// ревизий) неделим и жуёт 30+ минут в одном потоке; кэш не спасает — такой файл
/// меняется чаще всех и инвалидируется почти каждым коммитом.
///
/// Дефолт 10M выбран ПО ДАННЫМ (OmniRoute, 2026-07-02): нормальный код —
/// cost < 1M (p99 файлов = 15k added × десятки ревизий), патологические
/// генерируемые гиганты — cost > 30M (package-lock 64M, CHANGELOG 37M, i18n 32M);
/// 10M режет разрыв с запасом ×10 в обе стороны. На ripgrep не отсекает ничего
/// (max ≈ 1M). Конфигурируемо (`--blame-budget`); `0` = безлимит.
pub const DEFAULT_BLAME_BUDGET: u64 = 10_000_000;
