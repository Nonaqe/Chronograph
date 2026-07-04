//! Команда `chronograph knowledge` — риск концентрации знаний (bus factor).
//!
//! Как `hotspots`/`coupling`: инкрементально доводит кэш до HEAD, считает knowledge
//! (blame → ownership → bus factor), печатает таблицу, отсортированную ПО РИСКУ
//! (низкий bus_factor + высокий top_owner_ratio наверху). Без графики.
//!
//! Авторы АНОНИМИЗИРОВАНЫ по умолчанию (Author #N, принцип 2.4 CLAUDE.md — риск, не
//! заслуга/вина). Реальные имена — только по явному флагу `--show-names`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use chronograph_core::{run_analysis, Config};
use chronograph_git::GitSource;
use chronograph_metrics::{
    compute_knowledge, FileKnowledge, KnowledgeConfig, SkipReason, DEFAULT_BLAME_BUDGET,
};
use chronograph_store::DuckStore;
use clap::Args;

const CACHE_REL_PATH: &str = ".chronograph/cache.duckdb";

#[derive(Args)]
pub struct KnowledgeArgs {
    /// Путь к git-репозиторию (по умолчанию — текущая директория).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Путь к файлу кэша DuckDB (по умолчанию `<repo>/.chronograph/cache.duckdb`).
    #[arg(long = "db", value_name = "FILE")]
    pub db: Option<PathBuf>,

    /// Сколько строк показать.
    #[arg(long = "top", value_name = "N", default_value_t = 20)]
    pub top: usize,

    /// Показать реальные имена авторов вместо анонимных Author #N.
    ///
    /// По умолчанию ВЫКЛ (принцип 2.4: knowledge — риск концентрации, не заслуга).
    #[arg(long = "show-names", default_value_t = false)]
    pub show_names: bool,

    /// Glob-паттерны исключаемых путей (vendored/generated). Можно повторять.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Бюджет blame на файл (строко-ревизии: ревизии × добавленные строки).
    /// Файлы дороже — пропускаются с явной пометкой. 0 — безлимит.
    #[arg(long = "blame-budget", value_name = "N", default_value_t = DEFAULT_BLAME_BUDGET)]
    pub blame_budget: u64,
}

pub fn run(args: KnowledgeArgs) -> anyhow::Result<()> {
    let cfg = Config {
        repo_path: args.path.clone(),
        since: None,
        exclude: args.exclude.clone(),
        incremental: true,
        mechanical_commit_max_files: None,
    };
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| args.path.join(CACHE_REL_PATH));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("не удалось создать каталог кэша {}", parent.display()))?;
    }

    let source = GitSource::open(&cfg)
        .with_context(|| format!("открытие репозитория {}", cfg.repo_path.display()))?;
    let mut store = DuckStore::open(&db_path)
        .with_context(|| format!("открытие кэша {}", db_path.display()))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    run_analysis(&source, &mut store, &cfg, now).context("анализ истории")?;

    let knowledge_cfg = KnowledgeConfig {
        blame_budget: args.blame_budget,
        ..Default::default()
    };
    let knowledge = compute_knowledge(&store, &source, &knowledge_cfg).context("knowledge")?;

    // Ярлыки авторов: анонимные по умолчанию, реальные имена по флагу.
    let labels = if args.show_names {
        author_names(&store).context("чтение имён авторов")?
    } else {
        anon_labels(&knowledge.files)
    };

    print!(
        "{}",
        render(&knowledge.files, &labels, args.top, knowledge.blame_skipped)
    );
    // Причины пропусков — явно, не молча (полный список — в HTML-отчёте).
    if !knowledge.skips.is_empty() {
        println!("пропущено из blame (первые 10):");
        for s in knowledge.skips.iter().take(10) {
            match &s.reason {
                SkipReason::Failed => {
                    println!("  {} — сбой blame (баг gix-blame)", s.path);
                }
                SkipReason::OverBudget { cost, budget } => {
                    println!(
                        "  {} — дороже бюджета: {cost} строко-ревизий > {budget} (--blame-budget)",
                        s.path
                    );
                }
            }
        }
    }
    Ok(())
}

/// Анонимные ярлыки `author_id → "Author #N"` (плотный индекс по возр. author_id).
fn anon_labels(knowledge: &[FileKnowledge]) -> HashMap<i64, String> {
    let mut ids: Vec<i64> = knowledge
        .iter()
        .flat_map(|fk| fk.owners.iter().map(|o| o.author_id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .enumerate()
        .map(|(i, id)| (id, format!("Author #{}", i + 1)))
        .collect()
}

/// Реальные имена `author_id → canonical_name` из стора.
fn author_names(store: &DuckStore) -> anyhow::Result<HashMap<i64, String>> {
    let mut stmt = store
        .conn()
        .prepare("SELECT author_id, canonical_name FROM authors")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let mut map = HashMap::new();
    for r in rows {
        let (id, name) = r?;
        map.insert(id, name);
    }
    Ok(map)
}

/// Отрендерить таблицу риска (чистая функция — тестируется без БД).
///
/// Сортировка ПО РИСКУ: bus_factor возр. (1 = риск наверху), затем top_owner_ratio
/// убыв., затем путь — детерминированно.
fn render(
    knowledge: &[FileKnowledge],
    labels: &HashMap<i64, String>,
    top: usize,
    blame_skipped: usize,
) -> String {
    let mut out = String::new();
    if knowledge.is_empty() {
        out.push_str("Нет данных о владении (пустой репозиторий?).\n");
        if blame_skipped > 0 {
            out.push_str(&format!(
                "пропущено {blame_skipped} файлов из-за ошибки blame.\n"
            ));
        }
        return out;
    }

    let mut ranked: Vec<&FileKnowledge> = knowledge.iter().collect();
    ranked.sort_by(|a, b| {
        a.bus_factor
            .cmp(&b.bus_factor)
            .then(
                b.top_owner_ratio
                    .partial_cmp(&a.top_owner_ratio)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then_with(|| a.path.cmp(&b.path))
    });

    let bus1 = knowledge.iter().filter(|k| k.bus_factor == 1).count();
    out.push_str(&format!(
        "файлов: {}; bus_factor = 1 (риск концентрации): {}; пропущено blame: {}\n",
        knowledge.len(),
        bus1,
        blame_skipped
    ));
    out.push_str(&format!(
        "{:>3} {:>5}  {:<24} {}\n",
        "bf", "top%", "top owner", "file"
    ));
    for k in ranked.iter().take(top) {
        let owner_id = k.owners.first().map(|o| o.author_id).unwrap_or(-1);
        let owner = labels
            .get(&owner_id)
            .cloned()
            .unwrap_or_else(|| "Author #?".into());
        out.push_str(&format!(
            "{:>3} {:>4}% {:<24} {}\n",
            k.bus_factor,
            (k.top_owner_ratio * 100.0).round() as i64,
            truncate(&owner, 24),
            k.path,
        ));
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let tail: String = s.chars().skip(s.chars().count() - (max - 1)).collect();
        format!("…{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chronograph_metrics::AuthorOwnership;

    fn fk(path: &str, bf: u32, top: f64, top_author: i64) -> FileKnowledge {
        FileKnowledge {
            path: path.to_string(),
            total_lines: 100,
            owners: vec![AuthorOwnership {
                author_id: top_author,
                lines: (top * 100.0) as u32,
                ownership_ratio: top,
            }],
            bus_factor: bf,
            top_owner_ratio: top,
        }
    }

    #[test]
    fn render_sorts_by_risk_bf_then_ratio() {
        // bf=1/90% и bf=1/60% и bf=2/… → порядок: (1,90), (1,60), (2,..).
        let k = vec![
            fk("shared.rs", 2, 0.4, 7),
            fk("mid.rs", 1, 0.6, 8),
            fk("risky.rs", 1, 0.9, 9),
        ];
        let labels = anon_labels(&k);
        let out = render(&k, &labels, 10, 0);
        let p_risky = out.find("risky.rs").unwrap();
        let p_mid = out.find("mid.rs").unwrap();
        let p_shared = out.find("shared.rs").unwrap();
        assert!(p_risky < p_mid, "bf=1/90% выше bf=1/60%");
        assert!(p_mid < p_shared, "bf=1 выше bf=2");
    }

    #[test]
    fn render_anonymizes_by_default() {
        let k = vec![fk("a.rs", 1, 1.0, 42)];
        let labels = anon_labels(&k);
        let out = render(&k, &labels, 10, 0);
        assert!(out.contains("Author #1"), "анонимный ярлык");
        assert!(!out.contains("42"), "author_id не светится");
    }

    #[test]
    fn render_show_names_uses_labels() {
        // Явные имена (эмулируем --show-names передачей карты имён).
        let k = vec![fk("a.rs", 1, 1.0, 5)];
        let mut labels = HashMap::new();
        labels.insert(5, "Real Name".to_string());
        let out = render(&k, &labels, 10, 0);
        assert!(out.contains("Real Name"));
    }

    #[test]
    fn render_shows_blame_skipped_count() {
        // Счётчик пропущенных виден в шапке.
        let k = vec![fk("a.rs", 1, 1.0, 1)];
        let labels = anon_labels(&k);
        let out = render(&k, &labels, 10, 7);
        assert!(out.contains("пропущено blame: 7"), "счётчик в шапке");
    }

    #[test]
    fn render_empty_is_graceful() {
        let out = render(&[], &HashMap::new(), 10, 0);
        assert!(out.contains("Нет данных"));
    }
}
