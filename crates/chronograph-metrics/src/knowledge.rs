//! Knowledge map / bus factor — концентрация знаний по файлам (§3.5 ТЗ).
//!
//! **Что считаем:** для каждого живого файла — распределение авторства (доля
//! `ownership_ratio` каждого автора) и **bus factor** = минимальное число
//! топ-владельцев, чья суммарная доля строго превышает порог (дефолт 50%).
//!
//! **Как:** построчный blame HEAD-версии файла (через трейт
//! [`chronograph_core::BlameSource`], реализуемый gix в `chronograph-git` — здесь
//! gix не фигурирует). Каждая строка атрибутируется коммиту, последним её
//! изменившему; коммит → `author_id` (уже канонический после mailmap-ingestion,
//! таблица `commits`). Доли суммируются по автору.
//!
//! **Зачем:** файл с bus factor = 1 — операционный риск: знания о нём
//! сосредоточены в одном человеке (ушёл — встал модуль). Подаётся как РИСК
//! концентрации, не как заслуга/вина (принцип 2.4 CLAUDE.md); имена авторов здесь
//! не фигурируют — только `author_id`, анонимизация/отображение — на слое вывода.
//!
//! **Детерминизм:** blame-опции фиксированы (Histogram, весь файл); агрегация не
//! зависит от порядка участков; владельцы сортируются по (доля убыв., author_id
//! возр.) — тай-брейк детерминирован. Два прогона → идентичный результат.
//!
//! **Модуль = файл (v1, требует подтверждения):** bus factor считается на уровне
//! файла (`FileKnowledge.path`). ТЗ §3.5 говорит «файлам/модулям», но явного
//! определения «модуль = директория/пакет» нет; агрегация до директории — отдельный
//! шаг (как складывать доли/владельцев), вынесена на потом. См. CONTEXT.md.
//!
//! **Известные ограничения v1 (задокументированы):** (1) blame без
//! rename-following (`rewrites: None`) — ownership считается по текущей идентичности
//! файла; canonical-резолв живых файлов при этом over-approximates (может назвать
//! живым путь, которого на HEAD уже нет из-за не отслеженного rename) — такие файлы
//! blame честно отдаёт пустыми и они выпадают из результата, а не ломают прогон;
//! (2) «механические» коммиты (массовый реформат) НЕ исключаются из blame —
//! бот-форматтер может раздуть чью-то долю; оба — follow-up Этапа 4.

use std::collections::HashMap;

use chronograph_core::{BlameSource, FileBlame, Result};
use chronograph_store::DuckStore;

use crate::config::KnowledgeConfig;
use crate::store_err as se;

/// `author_id`, присваиваемый строкам, чей blame-коммит не найден в сторе.
///
/// При полном обходе истории такого быть не должно; сентинел нужен для честности
/// (доли всё равно суммируются в 1.0) и устойчивости, а не как норма.
const UNKNOWN_AUTHOR: i64 = -1;

/// Доля одного автора в файле.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorOwnership {
    /// Нормализованный автор (mailmap-канонический). `-1` — неизвестный коммит.
    pub author_id: i64,
    /// Сколько строк файла последним тронул этот автор.
    pub lines: u32,
    /// Доля автора в файле: `lines / total_lines`, (0, 1].
    pub ownership_ratio: f64,
}

/// Распределение знаний по одному файлу + его bus factor.
#[derive(Debug, Clone, PartialEq)]
pub struct FileKnowledge {
    /// Канонический путь файла (модуль v1).
    pub path: String,
    /// Всего атрибутированных blame-строк.
    pub total_lines: u32,
    /// Владельцы, отсортированы по доле убыв., затем `author_id` возр.
    pub owners: Vec<AuthorOwnership>,
    /// Минимум авторов, покрывающих > порога знаний. ≥ 1 для непустого файла.
    pub bus_factor: u32,
    /// Доля крупнейшего владельца (первый в `owners`).
    pub top_owner_ratio: f64,
}

/// Итог knowledge: распределения по файлам + счётчик пропущенных из-за паники blame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KnowledgeReport {
    /// По строке на файл (детерминированный порядок по пути).
    pub files: Vec<FileKnowledge>,
    /// Сколько живых файлов пропущено из blame (сбой или превышение бюджета).
    /// Показывается явно (CLI/HTML), не теряется молча.
    pub blame_skipped: usize,
    /// Пропущенные файлы ПОИМЁННО с причинами (для отчёта/CLI).
    pub skips: Vec<crate::blame_cache::BlameSkip>,
}

/// Посчитать knowledge/bus factor по всем живым файлам репозитория.
///
/// Порядок результата детерминирован (по пути). Файлы без blame-строк (пустые/
/// исчезнувшие) в результат не попадают. Файлы, на которых blame упал, считаются в
/// [`KnowledgeReport::blame_skipped`].
pub fn compute_knowledge(
    store: &DuckStore,
    blamer: &impl BlameSource,
    cfg: &KnowledgeConfig,
) -> Result<KnowledgeReport> {
    let conn = store.conn();

    // HEAD последнего прогона — коммит, на который считаем blame.
    let head: Option<String> = conn
        .query_row("SELECT head_sha FROM analysis_meta LIMIT 1", [], |r| {
            r.get(0)
        })
        .ok();
    let Some(head) = head else {
        return Ok(KnowledgeReport::default()); // анализ ещё не прогонялся
    };

    // Карта коммит → автор (канонический author_id после mailmap-ingestion).
    let commit_author = load_commit_authors(store)?;

    // Живые канонические файлы на HEAD (последнее изменение ≠ удаление).
    let files = crate::paths::living_files_meta(conn)?;

    // Blame через инкрементальный кэш + largest-first + бюджет (см. blame_cache):
    // повторный прогон переблеймливает только изменившиеся файлы.
    let (blamed, skips) =
        crate::blame_cache::cached_blame_many(store, blamer, &files, &head, cfg.blame_budget)?;
    let paths: Vec<String> = files.into_iter().map(|f| f.path).collect();

    let mut report = from_blame(&commit_author, &paths, &blamed, cfg);
    report.skips = skips;
    Ok(report)
}

/// Агрегировать knowledge/bus factor из ГОТОВОГО blame (без БД/gix).
///
/// Выделено, чтобы материализация блеймила ОДИН раз и питала knowledge+age из одного
/// прохода (blame — самая дорогая операция). `files[i]` соответствует `blamed[i]`.
pub(crate) fn from_blame(
    commit_author: &HashMap<String, i64>,
    files: &[String],
    blamed: &[FileBlame],
    cfg: &KnowledgeConfig,
) -> KnowledgeReport {
    let mut out = Vec::with_capacity(files.len());
    let mut blame_skipped = 0usize;
    for (path, fb) in files.iter().zip(blamed) {
        // Файл, на котором blame упал (паника gix-blame), — считаем и пропускаем.
        let hunks = match fb {
            FileBlame::Blamed(hunks) => hunks,
            FileBlame::Failed => {
                blame_skipped += 1;
                continue;
            }
        };

        // Суммируем строки по автору.
        let mut lines_by_author: HashMap<i64, u32> = HashMap::new();
        let mut total: u32 = 0;
        for h in hunks {
            let author = commit_author
                .get(&h.commit_sha)
                .copied()
                .unwrap_or(UNKNOWN_AUTHOR);
            *lines_by_author.entry(author).or_insert(0) += h.lines;
            total += h.lines;
        }
        if total == 0 {
            continue; // пустой файл — ownership не определён
        }

        // Владельцы, отсортированы детерминированно: доля убыв., author_id возр.
        let mut owners: Vec<AuthorOwnership> = lines_by_author
            .into_iter()
            .map(|(author_id, lines)| AuthorOwnership {
                author_id,
                lines,
                ownership_ratio: lines as f64 / total as f64,
            })
            .collect();
        owners.sort_by(|a, b| {
            b.lines
                .cmp(&a.lines)
                .then_with(|| a.author_id.cmp(&b.author_id))
        });

        let top_owner_ratio = owners[0].ownership_ratio;
        let bus_factor = compute_bus_factor(&owners, total, cfg.bus_factor_threshold);

        out.push(FileKnowledge {
            path: path.clone(),
            total_lines: total,
            owners,
            bus_factor,
            top_owner_ratio,
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    KnowledgeReport {
        files: out,
        blame_skipped,
        skips: Vec::new(), // заполняет вызывающий (cached_blame_many знает причины)
    }
}

/// bus factor = минимум топ-владельцев, чья суммарная доля СТРОГО > порога.
///
/// Считаем по строкам (целые), а не по накопленным f64-долям — устойчиво к
/// ошибкам округления: сравниваем `cumulative_lines > threshold * total`.
fn compute_bus_factor(owners: &[AuthorOwnership], total: u32, threshold: f64) -> u32 {
    let target = threshold * total as f64;
    let mut cumulative: u64 = 0;
    let mut count: u32 = 0;
    for o in owners {
        cumulative += o.lines as u64;
        count += 1;
        if cumulative as f64 > target {
            break;
        }
    }
    count.max(1)
}

/// Загрузить `commit.sha → author_id` из стора.
pub(crate) fn load_commit_authors(store: &DuckStore) -> Result<HashMap<String, i64>> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare("SELECT sha, author_id FROM commits")
        .map_err(se)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(se)?;
    let mut map = HashMap::new();
    for r in rows {
        let (sha, author) = r.map_err(se)?;
        map.insert(sha, author);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    // Агрегация ownership/bus_factor на настоящем gix blame покрыта интеграционными
    // тестами (`tests/knowledge.rs`); здесь — юниты чистой функции bus factor.
    use super::*;

    #[test]
    fn bus_factor_strict_threshold() {
        // 3 автора: 40% / 40% / 20% при пороге 0.5.
        // cumulative: 40 (не >50) → 80 (>50) → bus_factor = 2.
        let owners = vec![
            AuthorOwnership {
                author_id: 1,
                lines: 40,
                ownership_ratio: 0.4,
            },
            AuthorOwnership {
                author_id: 2,
                lines: 40,
                ownership_ratio: 0.4,
            },
            AuthorOwnership {
                author_id: 3,
                lines: 20,
                ownership_ratio: 0.2,
            },
        ];
        assert_eq!(compute_bus_factor(&owners, 100, 0.5), 2);
    }

    #[test]
    fn bus_factor_single_dominant_owner() {
        // 75% / 25%: первый уже > 50% → bus_factor = 1.
        let owners = vec![
            AuthorOwnership {
                author_id: 1,
                lines: 75,
                ownership_ratio: 0.75,
            },
            AuthorOwnership {
                author_id: 2,
                lines: 25,
                ownership_ratio: 0.25,
            },
        ];
        assert_eq!(compute_bus_factor(&owners, 100, 0.5), 1);
    }

    #[test]
    fn bus_factor_exactly_half_needs_next() {
        // Ровно 50% НЕ превышает порог (строгое >) → нужен следующий автор.
        let owners = vec![
            AuthorOwnership {
                author_id: 1,
                lines: 50,
                ownership_ratio: 0.5,
            },
            AuthorOwnership {
                author_id: 2,
                lines: 50,
                ownership_ratio: 0.5,
            },
        ];
        assert_eq!(compute_bus_factor(&owners, 100, 0.5), 2);
    }
}
