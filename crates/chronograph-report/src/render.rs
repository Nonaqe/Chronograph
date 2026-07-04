//! Рендер self-contained HTML. Детерминирован: фикс. форматирование чисел,
//! отсортированный вход, server-side SVG, без wall-clock времени.

use crate::data::{HotspotEntry, ReportData};
use crate::treemap::squarify;

const CSS: &str = include_str!("../assets/report.css");
const TREEMAP_W: f64 = 940.0;
const TREEMAP_H: f64 = 460.0;

/// Отрендерить `report.html` как строку.
pub fn render_html(data: &ReportData) -> String {
    let mut s = String::with_capacity(16 * 1024);
    s.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    s.push_str("<meta charset=\"utf-8\">\n");
    s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    s.push_str("<title>Chronograph report</title>\n<style>\n");
    s.push_str(CSS);
    s.push_str("\n</style>\n</head>\n<body>\n<div class=\"wrap\">\n");

    render_header(&mut s, data);
    render_overview(&mut s, data);
    render_hotspots(&mut s, data);
    render_coupling(&mut s, data);
    render_knowledge(&mut s, data);
    render_blame_skips(&mut s, data);
    render_age(&mut s, data);

    s.push_str("<footer>Chronograph ");
    s.push_str(&esc(&data.overview.engine_version));
    s.push_str(" · self-contained report · no external requests</footer>\n");
    s.push_str("</div>\n</body>\n</html>\n");
    s
}

fn render_header(s: &mut String, data: &ReportData) {
    s.push_str("<header>\n<h1>Chronograph report</h1>\n<div class=\"sub\">HEAD <code>");
    s.push_str(&esc(&data.overview.head_sha));
    s.push_str("</code></div>\n</header>\n");
}

fn render_overview(s: &mut String, data: &ReportData) {
    let o = &data.overview;
    s.push_str("<section>\n<h2>Overview</h2>\n<div class=\"cards\">\n");
    card(s, "commits", &o.total_commits.to_string());
    card(s, "files", &o.total_files.to_string());
    card(s, "hotspots", &o.hotspot_files.to_string());
    card(s, "coupled pairs", &o.coupling_pairs.to_string());
    card(s, "bus factor = 1", &o.bus_factor_one.to_string());
    card(s, "blame skipped", &o.blame_skipped.to_string());
    s.push_str("</div>\n<div class=\"meta\">engine <code>");
    s.push_str(&esc(&o.engine_version));
    s.push_str("</code> · config <code>");
    s.push_str(&esc(&o.config_hash));
    s.push_str("</code></div>\n</section>\n");
}

fn card(s: &mut String, key: &str, value: &str) {
    s.push_str("<div class=\"card\"><div class=\"k\">");
    s.push_str(&esc(key));
    s.push_str("</div><div class=\"v\">");
    s.push_str(&esc(value));
    s.push_str("</div></div>\n");
}

fn render_hotspots(s: &mut String, data: &ReportData) {
    s.push_str("<section>\n<h2>Hotspots — churn × complexity</h2>\n");
    if data.hotspots.is_empty() {
        s.push_str("<p class=\"meta\">Нет файлов с поддержанной complexity.</p>\n</section>\n");
        return;
    }
    // Порядок раскладки: площадь (complexity) убыв., tie-break по пути.
    let mut order: Vec<usize> = (0..data.hotspots.len()).collect();
    order.sort_by(|&a, &b| {
        let (ha, hb) = (&data.hotspots[a], &data.hotspots[b]);
        hb.complexity
            .partial_cmp(&ha.complexity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| ha.path.cmp(&hb.path))
    });
    let areas: Vec<f64> = order.iter().map(|&i| data.hotspots[i].complexity).collect();
    let rects = squarify(&areas, TREEMAP_W, TREEMAP_H);

    let (min_c, max_c) = churn_range(&data.hotspots);

    s.push_str("<div class=\"treemap\">\n<svg viewBox=\"0 0 ");
    s.push_str(&n2(TREEMAP_W));
    s.push(' ');
    s.push_str(&n2(TREEMAP_H));
    s.push_str("\" xmlns=\"http://www.w3.org/2000/svg\">\n");

    for r in &rects {
        let e = &data.hotspots[order[r.index]];
        let t = norm(e.churn, min_c, max_c);
        let (cr, cg, cb) = churn_color(t);
        s.push_str("<g>\n<title>");
        s.push_str(&esc(&format!(
            "{} · churn {} · cx {} · #{}",
            e.path, e.churn, e.complexity as i64, e.rank
        )));
        s.push_str("</title>\n<rect x=\"");
        s.push_str(&n2(r.x));
        s.push_str("\" y=\"");
        s.push_str(&n2(r.y));
        s.push_str("\" width=\"");
        s.push_str(&n2(r.w));
        s.push_str("\" height=\"");
        s.push_str(&n2(r.h));
        s.push_str("\" fill=\"rgb(");
        s.push_str(&format!("{cr},{cg},{cb}"));
        s.push_str(")\" stroke=\"#fbfaf7\" stroke-width=\"1\"/>\n");
        // Подпись, если прямоугольник достаточно крупный.
        if r.w > 46.0 && r.h > 20.0 {
            let text_fill = if t > 0.55 { "#ffffff" } else { "#22201c" };
            let max_chars = ((r.w - 8.0) / 6.5) as usize;
            let label = truncate(basename(&e.path), max_chars.max(1));
            s.push_str("<text x=\"");
            s.push_str(&n2(r.x + 5.0));
            s.push_str("\" y=\"");
            s.push_str(&n2(r.y + 15.0));
            s.push_str("\" font-family=\"ui-monospace, Menlo, Consolas, monospace\" font-size=\"11\" fill=\"");
            s.push_str(text_fill);
            s.push_str("\">");
            s.push_str(&esc(&label));
            s.push_str("</text>\n");
        }
        s.push_str("</g>\n");
    }
    s.push_str("</svg>\n</div>\n");
    s.push_str("<div class=\"legend\">площадь = complexity · цвет = churn (низкий <span class=\"bar\"></span> высокий)</div>\n");

    // Компактная таблица топ-hotspots.
    s.push_str("<table>\n<thead><tr><th class=\"num\">#</th><th>file</th><th class=\"num\">churn</th><th class=\"num\">complexity</th></tr></thead>\n<tbody>\n");
    for e in data.hotspots.iter().take(15) {
        s.push_str("<tr><td class=\"num\">");
        s.push_str(&e.rank.to_string());
        s.push_str("</td><td class=\"path\">");
        s.push_str(&esc(&e.path));
        s.push_str("</td><td class=\"num\">");
        s.push_str(&e.churn.to_string());
        s.push_str("</td><td class=\"num\">");
        s.push_str(&(e.complexity as i64).to_string());
        s.push_str("</td></tr>\n");
    }
    s.push_str("</tbody>\n</table>\n</section>\n");
}

fn render_coupling(s: &mut String, data: &ReportData) {
    s.push_str("<section>\n<h2>Change coupling — файлы, меняющиеся вместе</h2>\n");
    if data.couplings.is_empty() {
        s.push_str("<p class=\"meta\">Нет пар выше порога support.</p>\n</section>\n");
        return;
    }
    s.push_str("<table>\n<thead><tr><th class=\"num\">support</th><th class=\"num\">ratio</th><th>file A</th><th>file B</th></tr></thead>\n<tbody>\n");
    for c in data.couplings.iter().take(25) {
        s.push_str("<tr><td class=\"num\">");
        s.push_str(&c.support.to_string());
        s.push_str("</td><td class=\"num\">");
        s.push_str(&n2(c.ratio));
        s.push_str("</td><td class=\"path\">");
        s.push_str(&esc(&c.path_a));
        s.push_str("</td><td class=\"path\">");
        s.push_str(&esc(&c.path_b));
        s.push_str("</td></tr>\n");
    }
    s.push_str("</tbody>\n</table>\n</section>\n");
}

fn render_knowledge(s: &mut String, data: &ReportData) {
    s.push_str("<section>\n<h2>Knowledge &amp; bus factor — риск концентрации знаний</h2>\n");
    if data.knowledge.is_empty() {
        s.push_str("<p class=\"meta\">Нет данных о владении.</p>\n</section>\n");
        return;
    }
    s.push_str("<p class=\"meta\">Файл с bus factor = 1 — риск: знания о нём сосредоточены в одном человеке. Авторы анонимизированы.</p>\n");
    s.push_str("<table>\n<thead><tr><th class=\"num\">bus factor</th><th class=\"num\">top owner</th><th>owner</th><th>file</th></tr></thead>\n<tbody>\n");
    // Вход уже отсортирован по риску (bus_factor↑, top_owner_ratio↓, путь).
    for k in data.knowledge.iter().take(25) {
        s.push_str("<tr><td class=\"num\">");
        s.push_str(&k.bus_factor.to_string());
        s.push_str("</td><td class=\"num\">");
        s.push_str(&pct(k.top_owner_ratio));
        s.push_str("</td><td>");
        s.push_str(&esc(&k.top_owner));
        s.push_str("</td><td class=\"path\">");
        s.push_str(&esc(&k.module));
        s.push_str("</td></tr>\n");
    }
    s.push_str("</tbody>\n</table>\n</section>\n");
}

fn render_blame_skips(s: &mut String, data: &ReportData) {
    if data.blame_skips.is_empty() {
        return; // нечего объяснять — секция не показывается
    }
    s.push_str("<section>\n<h2>Файлы вне knowledge/age — почему</h2>\n");
    s.push_str(
        "<p class=\"meta\">Эти файлы исключены из построчного анализа владения/возраста. \
         Причина указана для каждого: либо файл непомерно дорог для blame — чаще всего \
         это генерируемые гиганты (lock-файлы, changelog'и, i18n-бандлы), но бывает и \
         живой код-долгожитель; порог настраивается флагом --blame-budget — либо на нём \
         падает gix-blame (известный баг апстрима). Остальные метрики \
         (churn/coupling/hotspots) по этим файлам посчитаны полностью.</p>\n",
    );
    s.push_str("<table>\n<thead><tr><th>file</th><th>причина</th></tr></thead>\n<tbody>\n");
    for e in data.blame_skips.iter().take(50) {
        s.push_str("<tr><td class=\"path\">");
        s.push_str(&esc(&e.path));
        s.push_str("</td><td>");
        s.push_str(&esc(&e.reason));
        s.push_str("</td></tr>\n");
    }
    s.push_str("</tbody>\n</table>\n</section>\n");
}

/// Бакеты возраста (границы в днях, верхняя эксклюзивна). Это ПРЕЗЕНТАЦИОННЫЙ выбор
/// (как раскладка/цвета treemap), фиксирован ради детерминизма — НЕ порог метрики
/// (метрика хранит перцентили, §3.6 порога не задаёт).
const AGE_BUCKETS: &[(&str, i64)] = &[
    ("<1мес", 30),
    ("1-3мес", 90),
    ("3-12мес", 365),
    ("1-2г", 730),
    ("2-5л", 1825),
    ("5л+", i64::MAX),
];
const AGE_W: f64 = 940.0;
const AGE_H: f64 = 260.0;

fn render_age(s: &mut String, data: &ReportData) {
    s.push_str("<section>\n<h2>Code age — распределение возраста строк</h2>\n");
    if data.age_medians.is_empty() {
        s.push_str("<p class=\"meta\">Нет данных о возрасте.</p>\n</section>\n");
        return;
    }

    // Файлы по median-возрасту в фикс. бакеты (свежие слева, стабильные справа).
    let mut counts = vec![0u64; AGE_BUCKETS.len()];
    for &m in &data.age_medians {
        let idx = AGE_BUCKETS
            .iter()
            .position(|(_, hi)| m < *hi)
            .unwrap_or(AGE_BUCKETS.len() - 1);
        counts[idx] += 1;
    }
    let max = counts.iter().copied().max().unwrap_or(1).max(1);

    s.push_str("<p class=\"meta\">Файлов по median-возрасту строк: свежепереписанные слева, стабильный старый код справа.</p>\n");
    s.push_str("<div class=\"treemap\">\n<svg viewBox=\"0 0 ");
    s.push_str(&n2(AGE_W));
    s.push(' ');
    s.push_str(&n2(AGE_H));
    s.push_str("\" xmlns=\"http://www.w3.org/2000/svg\">\n");

    let n = AGE_BUCKETS.len() as f64;
    let slot = AGE_W / n;
    let bar_w = slot * 0.7;
    let pad = (slot - bar_w) / 2.0;
    let base_y = AGE_H - 34.0; // место под подписи бакетов
    let chart_h = base_y - 18.0; // место над столбцом под число

    for (i, ((label, _), &count)) in AGE_BUCKETS.iter().zip(&counts).enumerate() {
        let fi = i as f64;
        let x = fi * slot + pad;
        let h = chart_h * (count as f64 / max as f64);
        let y = base_y - h;
        // Цвет: свежие (слева) — насыщенно, старые (справа) — бледно (fresh vs древнее).
        let t = if n > 1.0 {
            (n - 1.0 - fi) / (n - 1.0)
        } else {
            1.0
        };
        let (cr, cg, cb) = churn_color(t);

        s.push_str("<rect x=\"");
        s.push_str(&n2(x));
        s.push_str("\" y=\"");
        s.push_str(&n2(y));
        s.push_str("\" width=\"");
        s.push_str(&n2(bar_w));
        s.push_str("\" height=\"");
        s.push_str(&n2(h));
        s.push_str("\" fill=\"rgb(");
        s.push_str(&format!("{cr},{cg},{cb}"));
        s.push_str(")\" stroke=\"#fbfaf7\" stroke-width=\"1\"/>\n");

        // Число файлов над столбцом.
        s.push_str("<text x=\"");
        s.push_str(&n2(x + bar_w / 2.0));
        s.push_str("\" y=\"");
        s.push_str(&n2(y - 5.0));
        s.push_str("\" text-anchor=\"middle\" font-family=\"ui-monospace, Menlo, Consolas, monospace\" font-size=\"12\" fill=\"#22201c\">");
        s.push_str(&count.to_string());
        s.push_str("</text>\n");

        // Подпись бакета под осью.
        s.push_str("<text x=\"");
        s.push_str(&n2(x + bar_w / 2.0));
        s.push_str("\" y=\"");
        s.push_str(&n2(base_y + 18.0));
        s.push_str("\" text-anchor=\"middle\" font-family=\"ui-monospace, Menlo, Consolas, monospace\" font-size=\"12\" fill=\"#22201c\">");
        s.push_str(&esc(label));
        s.push_str("</text>\n");
    }

    s.push_str("</svg>\n</div>\n</section>\n");
}

// --- детерминированные хелперы форматирования ---

/// Доля как целые проценты с фиксированным форматом (детерминизм).
fn pct(x: f64) -> String {
    format!("{}%", (x * 100.0).round() as i64)
}

/// Число с фиксированной точностью (2 знака) — не дефолтный float Display.
fn n2(x: f64) -> String {
    format!("{x:.2}")
}

fn churn_range(hs: &[HotspotEntry]) -> (u64, u64) {
    let mut min = u64::MAX;
    let mut max = 0u64;
    for e in hs {
        min = min.min(e.churn);
        max = max.max(e.churn);
    }
    (min, max)
}

fn norm(v: u64, min: u64, max: u64) -> f64 {
    if max <= min {
        0.5
    } else {
        (v - min) as f64 / (max - min) as f64
    }
}

/// Цвет churn: интерполяция pale (255,245,200) → deep red (176,0,0).
fn churn_color(t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let r = (255.0 + t * (176.0 - 255.0)).round() as u8;
    let g = (245.0 + t * (0.0 - 245.0)).round() as u8;
    let b = (200.0 + t * (0.0 - 200.0)).round() as u8;
    (r, g, b)
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max <= 1 {
        "…".to_string()
    } else {
        let head: String = s.chars().take(max - 1).collect();
        format!("{head}…")
    }
}

/// Экранирование HTML/XML-текста.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{CouplingEntry, KnowledgeEntry, Overview};

    fn sample() -> ReportData {
        ReportData {
            overview: Overview {
                head_sha: "abcdef123456".into(),
                engine_version: "0.0.0".into(),
                config_hash: "deadbeef".into(),
                total_commits: 100,
                total_files: 10,
                hotspot_files: 2,
                coupling_pairs: 1,
                bus_factor_one: 1,
                blame_skipped: 3,
            },
            hotspots: vec![
                HotspotEntry {
                    rank: 1,
                    path: "src/a.rs".into(),
                    churn: 50,
                    complexity: 20.0,
                },
                HotspotEntry {
                    rank: 2,
                    path: "src/b.rs".into(),
                    churn: 10,
                    complexity: 5.0,
                },
            ],
            couplings: vec![CouplingEntry {
                path_a: "src/a.rs".into(),
                path_b: "src/b.rs".into(),
                support: 8,
                ratio: 0.8,
            }],
            knowledge: vec![
                KnowledgeEntry {
                    module: "src/a.rs".into(),
                    bus_factor: 1,
                    top_owner_ratio: 1.0,
                    top_owner: "Author #1".into(),
                },
                KnowledgeEntry {
                    module: "src/b.rs".into(),
                    bus_factor: 2,
                    top_owner_ratio: 0.5,
                    top_owner: "Author #2".into(),
                },
            ],
            // Возрасты в разных бакетах: 5 дн (<1мес), 200 дн (3-12мес), 1500 дн (2-5л).
            age_medians: vec![5, 200, 1500],
            blame_skips: vec![crate::data::BlameSkipEntry {
                path: "CHANGELOG.md".into(),
                reason: "слишком дорог для blame: стоимость 37000000 строко-ревизий > бюджета 10000000 (файл с очень большой историей изменений; поднять: --blame-budget)".into(),
            }],
        }
    }

    #[test]
    fn render_is_deterministic() {
        let d = sample();
        assert_eq!(render_html(&d), render_html(&d));
    }

    #[test]
    fn render_has_no_external_requests() {
        // Self-contained: никаких внешних ресурсов. (xmlns SVG-namespace — не запрос.)
        let html = render_html(&sample());
        assert!(!html.contains("<script"), "нет JS в v1");
        assert!(!html.contains("<link"), "нет внешних стилей");
        assert!(!html.contains("src=\"http"), "нет внешних src");
        assert!(!html.contains("href=\"http"), "нет внешних href");
        assert!(!html.to_lowercase().contains("cdn"), "нет CDN");
    }

    #[test]
    fn render_contains_data() {
        let html = render_html(&sample());
        assert!(html.contains("src/a.rs"));
        assert!(html.contains("<svg"));
        assert!(html.contains("Change coupling"));
        assert!(html.contains("abcdef123456"));
        // Счётчик пропущенных blame — карточка в Overview (значение из sample = 3).
        assert!(html.contains("blame skipped"));
        assert!(html.contains(">3</div>"));
    }

    #[test]
    fn blame_skips_section_shows_reasons() {
        let html = render_html(&sample());
        // Секция причин есть, файл назван, причина с числами показана.
        assert!(html.contains("Файлы вне knowledge/age"));
        assert!(html.contains("CHANGELOG.md"));
        assert!(html.contains("слишком дорог для blame"));
        assert!(html.contains("--blame-budget"), "как поднять — подсказано");

        // Без пропусков секция не рендерится вовсе.
        let mut clean = sample();
        clean.blame_skips.clear();
        let html2 = render_html(&clean);
        assert!(!html2.contains("Файлы вне knowledge/age"));
    }

    #[test]
    fn age_section_renders_histogram() {
        let html = render_html(&sample());
        assert!(html.contains("Code age"), "секция возраста есть");
        // Бакеты как подписи осей.
        assert!(html.contains("&lt;1мес") || html.contains("<1мес"));
        assert!(html.contains("2-5л"));
        // Ещё один <svg> (гистограмма) помимо treemap.
        assert!(
            html.matches("<svg").count() >= 2,
            "treemap + гистограмма возраста"
        );
    }

    #[test]
    fn knowledge_section_is_anonymized_and_risk_first() {
        let html = render_html(&sample());
        // Секция есть, анонимные ярлыки — не имена.
        assert!(html.contains("bus factor"));
        assert!(html.contains("Author #1"));
        assert!(html.contains("Author #2"));
        // Внутри секции Knowledge: риск (bf=1, a.rs) выше bf=2 (b.rs).
        let sect = &html[html.find("Knowledge").expect("секция есть")..];
        let pos_a = sect.find("src/a.rs").expect("a.rs в секции");
        let pos_b = sect.find("src/b.rs").expect("b.rs в секции");
        assert!(pos_a < pos_b, "риск (bf=1) выше по секции");
        // Никаких реальных имён не утекает через эту секцию.
        assert!(!html.contains("David Tolnay"));
    }

    #[test]
    fn escaping_prevents_injection() {
        let mut d = sample();
        d.hotspots[0].path = "a<b>&\"'.rs".into();
        let html = render_html(&d);
        assert!(html.contains("a&lt;b&gt;&amp;"));
        assert!(!html.contains("a<b>"));
    }

    #[test]
    fn float_formatting_is_fixed_precision() {
        assert_eq!(n2(0.5), "0.50");
        assert_eq!(n2(12.0), "12.00");
    }
}
