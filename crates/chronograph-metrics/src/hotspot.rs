//! Hotspot — главный агрегат: «сложный И часто меняемый» код.
//!
//! **Что считаем:** для каждого файла `hotspot_score = churn_pct × cx_pct`, где
//! `churn_pct`/`cx_pct` — ранг-перцентили файла по churn и по complexity в [0,1].
//!
//! **Почему так (обоснование формулы):**
//! - Перемножать сырые churn (в коммитах) и complexity (в очках) нельзя — разные
//!   шкалы (правило CLAUDE.md). Поэтому нормализуем в ранг-перцентили.
//! - Произведение перцентилей высоко ТОЛЬКО когда файл в верхах и по churn, и по
//!   complexity — это ровно «верхний квантиль по обоим» из ТЗ 3.3. Файл сложный,
//!   но стабильный (или изменчивый, но простой) высокого score не получит.
//! - Прозрачно и раскрываемо до составляющих (никаких «health score 0–100»).
//!
//! **Фильтр (решение v1, см. CONTEXT.md):** ранжируются только живые файлы с
//! **cyclomatic** complexity (поддержанные языки). Indentation-fallback (не-код:
//! доки/конфиги/снапшоты) в hotspot НЕ участвует — иначе LICENSE/YAML забивают
//! реальный код.

use std::collections::HashMap;

use chronograph_lang::ComplexityMethod;

use crate::churn::FileChurn;
use crate::complexity::FileComplexityRow;

/// Окно churn, используемое для hotspot-ранга.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChurnWindow {
    /// Вся история (дефолт).
    #[default]
    Total,
    /// Короткое окно.
    Recent,
    /// Среднее окно.
    Mid,
    /// Длинное окно.
    Long,
}

/// Конфигурация hotspot-ранжирования.
#[derive(Debug, Clone, Default)]
pub struct HotspotConfig {
    /// Какое окно churn брать для ранга.
    pub churn_window: ChurnWindow,
}

/// Одна строка hotspot-рейтинга (раскрывается до составляющих).
#[derive(Debug, Clone, PartialEq)]
pub struct Hotspot {
    /// Канонический путь файла.
    pub path: String,
    /// Churn (в выбранном окне), коммитов.
    pub churn: u64,
    /// Complexity (cyclomatic).
    pub complexity: f64,
    /// Ранг-перцентиль по churn, [0,1].
    pub churn_pct: f64,
    /// Ранг-перцентиль по complexity, [0,1].
    pub complexity_pct: f64,
    /// Итоговый score = churn_pct × cx_pct.
    pub score: f64,
    /// Позиция в рейтинге (1 = самый горячий).
    pub rank: u32,
}

/// Посчитать hotspot-рейтинг из готовых churn и complexity.
///
/// Чистая функция (без БД): комбинирует уже посчитанные метрики. Ранжируются
/// только живые файлы, у которых complexity считалась по AST (cyclomatic).
/// Возврат отсортирован по score убыв. (tie-break по пути), с проставленным rank.
pub fn compute_hotspots(
    churn: &[FileChurn],
    complexity: &[FileComplexityRow],
    cfg: &HotspotConfig,
) -> Vec<Hotspot> {
    // Churn живых файлов в выбранном окне.
    let churn_map: HashMap<&str, u64> = churn
        .iter()
        .filter(|c| c.is_alive)
        .map(|c| (c.path.as_str(), window_value(c, cfg.churn_window)))
        .collect();

    // Универсум ранжирования: живые файлы с cyclomatic complexity.
    let files: Vec<(&str, u64, f64)> = complexity
        .iter()
        .filter(|r| r.method == ComplexityMethod::Cyclomatic)
        .map(|r| {
            let churn = churn_map.get(r.path.as_str()).copied().unwrap_or(0);
            (r.path.as_str(), churn, r.value)
        })
        .collect();

    if files.is_empty() {
        return Vec::new();
    }

    let churn_pct = percentiles(files.iter().map(|(p, ch, _)| (*p, *ch as f64)));
    let cx_pct = percentiles(files.iter().map(|(p, _, cx)| (*p, *cx)));

    let mut hotspots: Vec<Hotspot> = files
        .iter()
        .map(|(path, churn, cx)| {
            let cp = churn_pct[*path];
            let xp = cx_pct[*path];
            Hotspot {
                path: (*path).to_string(),
                churn: *churn,
                complexity: *cx,
                churn_pct: cp,
                complexity_pct: xp,
                score: cp * xp,
                rank: 0,
            }
        })
        .collect();

    // Убыв. по score; стабильный tie-break по пути (детерминизм).
    hotspots.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    for (i, h) in hotspots.iter_mut().enumerate() {
        h.rank = i as u32 + 1;
    }
    hotspots
}

fn window_value(c: &FileChurn, window: ChurnWindow) -> u64 {
    match window {
        ChurnWindow::Total => c.churn_total,
        ChurnWindow::Recent => c.churn_recent,
        ChurnWindow::Mid => c.churn_mid,
        ChurnWindow::Long => c.churn_long,
    }
}

/// Ранг-перцентиль в [0,1] для каждого пути: 0 у минимума, 1 у максимума.
///
/// Сортировка по (значение, путь) — детерминированный tie-break. При одном файле
/// перцентиль 1.0.
fn percentiles<'a>(items: impl Iterator<Item = (&'a str, f64)>) -> HashMap<&'a str, f64> {
    let mut v: Vec<(&str, f64)> = items.collect();
    v.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    let n = v.len();
    let mut map = HashMap::with_capacity(n);
    for (i, (path, _)) in v.iter().enumerate() {
        let pct = if n <= 1 {
            1.0
        } else {
            i as f64 / (n - 1) as f64
        };
        map.insert(*path, pct);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use chronograph_lang::SupportedLanguage;

    fn churn(path: &str, total: u64, alive: bool) -> FileChurn {
        FileChurn {
            path: path.to_string(),
            churn_total: total,
            churn_recent: total,
            churn_mid: total,
            churn_long: total,
            lines_added: 0,
            lines_deleted: 0,
            is_alive: alive,
        }
    }

    fn cx(path: &str, value: f64, method: ComplexityMethod) -> FileComplexityRow {
        FileComplexityRow {
            path: path.to_string(),
            blob_sha: "b".to_string(),
            value,
            per_loc: 0.0,
            loc: 10,
            method,
            language: match method {
                ComplexityMethod::Cyclomatic => Some(SupportedLanguage::Rust),
                ComplexityMethod::Indentation => None,
            },
        }
    }

    #[test]
    fn top_hotspot_is_high_in_both() {
        // hot.rs: высокий churn И высокая complexity → должен быть #1.
        let churns = vec![
            churn("hot.rs", 100, true),
            churn("churny.rs", 90, true),
            churn("complex.rs", 5, true),
            churn("calm.rs", 3, true),
        ];
        let cxs = vec![
            cx("hot.rs", 40.0, ComplexityMethod::Cyclomatic),
            cx("churny.rs", 2.0, ComplexityMethod::Cyclomatic),
            cx("complex.rs", 45.0, ComplexityMethod::Cyclomatic),
            cx("calm.rs", 1.0, ComplexityMethod::Cyclomatic),
        ];
        let hs = compute_hotspots(&churns, &cxs, &HotspotConfig::default());
        assert_eq!(hs[0].path, "hot.rs");
        assert_eq!(hs[0].rank, 1);
        // calm.rs (низкий по обоим) — последний.
        assert_eq!(hs.last().unwrap().path, "calm.rs");
    }

    #[test]
    fn score_is_in_unit_range_and_product() {
        let churns = vec![
            churn("a", 10, true),
            churn("b", 20, true),
            churn("c", 30, true),
        ];
        let cxs = vec![
            cx("a", 1.0, ComplexityMethod::Cyclomatic),
            cx("b", 2.0, ComplexityMethod::Cyclomatic),
            cx("c", 3.0, ComplexityMethod::Cyclomatic),
        ];
        let hs = compute_hotspots(&churns, &cxs, &HotspotConfig::default());
        for h in &hs {
            assert!((0.0..=1.0).contains(&h.churn_pct));
            assert!((0.0..=1.0).contains(&h.complexity_pct));
            assert!((0.0..=1.0).contains(&h.score));
            // score — ровно произведение перцентилей.
            assert!((h.score - h.churn_pct * h.complexity_pct).abs() < 1e-12);
        }
    }

    #[test]
    fn fallback_files_are_excluded() {
        // LICENSE (indentation, огромный value) и высокий churn — НЕ в hotspots.
        let churns = vec![churn("code.rs", 10, true), churn("LICENSE", 500, true)];
        let cxs = vec![
            cx("code.rs", 5.0, ComplexityMethod::Cyclomatic),
            cx("LICENSE", 999.0, ComplexityMethod::Indentation),
        ];
        let hs = compute_hotspots(&churns, &cxs, &HotspotConfig::default());
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].path, "code.rs");
        assert!(hs.iter().all(|h| h.path != "LICENSE"));
    }

    #[test]
    fn dead_files_have_zero_churn_contribution() {
        // Мёртвый по churn файл не должен получать churn-сигнал.
        let churns = vec![churn("alive.rs", 50, true), churn("dead.rs", 99, false)];
        let cxs = vec![
            cx("alive.rs", 10.0, ComplexityMethod::Cyclomatic),
            cx("dead.rs", 10.0, ComplexityMethod::Cyclomatic),
        ];
        let hs = compute_hotspots(&churns, &cxs, &HotspotConfig::default());
        let dead = hs.iter().find(|h| h.path == "dead.rs").unwrap();
        assert_eq!(dead.churn, 0, "мёртвый churn не учитывается");
    }

    #[test]
    fn deterministic_across_runs() {
        let churns = vec![
            churn("a", 10, true),
            churn("b", 10, true),
            churn("c", 30, true),
        ];
        let cxs = vec![
            cx("a", 5.0, ComplexityMethod::Cyclomatic),
            cx("b", 5.0, ComplexityMethod::Cyclomatic),
            cx("c", 5.0, ComplexityMethod::Cyclomatic),
        ];
        let a = compute_hotspots(&churns, &cxs, &HotspotConfig::default());
        let b = compute_hotspots(&churns, &cxs, &HotspotConfig::default());
        assert_eq!(a, b);
    }

    #[test]
    fn empty_when_no_cyclomatic_files() {
        let churns = vec![churn("readme.md", 100, true)];
        let cxs = vec![cx("readme.md", 200.0, ComplexityMethod::Indentation)];
        let hs = compute_hotspots(&churns, &cxs, &HotspotConfig::default());
        assert!(hs.is_empty());
    }
}
