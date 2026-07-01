//! Change coupling — killer feature: файлы, меняющиеся вместе.
//!
//! **Что считаем:** для пары файлов (A, B):
//! - `support(A,B)` = число коммитов, где менялись оба;
//! - `coupling_ratio(A,B) = support / min(commits(A), commits(B))` — доля
//!   совместных изменений относительно реже меняющегося из пары.
//!
//! **Как (co-occurrence, НЕ декартов квадрат):** для каждого коммита берём его
//! множество изменённых канонических файлов и считаем пары ВНУТРИ коммита
//! (self-join `file_changes` по `sha`, `a.path < b.path` — одна симметричная пара).
//! В рейтинг попадают только реально со-встречавшиеся пары. Сложность O(Σ kᵢ²) по
//! коммитам, а не O(файлов²).
//!
//! **Зачем:** высокая связность + отсутствие явной зависимости в коде = скрытый
//! архитектурный долг. Самый «вау»-инсайт (ТЗ 3.4).
//!
//! **Фильтры (ТЗ 3.4):** минимальный `support` (шум); исключение «гигантских»/
//! механических коммитов (и искажают сигнал, и раздувают O(kᵢ²)).
//!
//! **Симметрия:** пара хранится канонически (`path_a < path_b`); `support` —
//! симметричное пересечение, `min(commits)` — симметричный минимум ⇒
//! `coupling(A,B) == coupling(B,A)` по построению (CLAUDE.md).

use chronograph_core::Result;
use chronograph_store::DuckStore;

use crate::store_err as se;

/// Конфигурация change coupling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CouplingConfig {
    /// Минимальный support (совместных коммитов) для попадания пары в рейтинг.
    ///
    /// Дефолт 5 — из примера ТЗ 3.1/3.4 («например, ≥ 5»); НЕ спец-значение,
    /// конфигурируемо (CLAUDE.md запрещает зашивать пороги).
    pub min_support: u32,
    /// Исключать «механические»/гигантские коммиты (ТЗ 3.4 требует это явно).
    pub exclude_mechanical: bool,
}

impl Default for CouplingConfig {
    fn default() -> Self {
        CouplingConfig {
            min_support: 5,
            exclude_mechanical: true,
        }
    }
}

/// Связность одной пары файлов (канонически `path_a < path_b`).
#[derive(Debug, Clone, PartialEq)]
pub struct Coupling {
    /// Первый путь пары (лексикографически меньший).
    pub path_a: String,
    /// Второй путь пары.
    pub path_b: String,
    /// Число коммитов, где менялись оба.
    pub support: u64,
    /// `support / min(commits(A), commits(B))`, (0,1].
    pub coupling_ratio: f64,
}

/// Посчитать change coupling по всем со-встречавшимся парам файлов.
///
/// Возврат отсортирован по `coupling_ratio` убыв., затем `support` убыв., затем
/// путям — детерминированно.
pub fn compute_coupling(store: &DuckStore, cfg: &CouplingConfig) -> Result<Vec<Coupling>> {
    let conn = store.conn();

    let canonical = crate::paths::build_canonical_map(conn)?;
    crate::paths::materialize_path_map(conn, &canonical)?;

    let mech = if cfg.exclude_mechanical {
        "NOT c.is_mechanical"
    } else {
        "TRUE"
    };

    // mapped: уникальные (канонический путь, коммит), без механических.
    // pairs: пары внутри коммита (co-occurrence, a.path < b.path).
    // coupling_ratio = support / min(commits(a), commits(b)).
    let sql = format!(
        "WITH mapped AS (
             SELECT DISTINCT pm.canonical AS path, fc.sha AS sha
             FROM file_changes fc
             JOIN commits c ON fc.sha = c.sha
             JOIN path_map pm ON fc.path = pm.path
             WHERE {mech}
         ),
         commit_counts AS (
             SELECT path, count(*) AS n FROM mapped GROUP BY path
         ),
         pairs AS (
             SELECT a.path AS pa, b.path AS pb, count(*) AS support
             FROM mapped a
             JOIN mapped b ON a.sha = b.sha AND a.path < b.path
             GROUP BY a.path, b.path
             HAVING count(*) >= {min_support}
         )
         SELECT p.pa, p.pb, p.support,
                CAST(p.support AS DOUBLE) / least(ca.n, cb.n) AS coupling_ratio
         FROM pairs p
         JOIN commit_counts ca ON p.pa = ca.path
         JOIN commit_counts cb ON p.pb = cb.path
         ORDER BY coupling_ratio DESC, p.support DESC, p.pa, p.pb",
        mech = mech,
        min_support = cfg.min_support
    );

    let mut stmt = conn.prepare(&sql).map_err(se)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Coupling {
                path_a: row.get(0)?,
                path_b: row.get(1)?,
                support: row.get::<_, i64>(2)? as u64,
                coupling_ratio: row.get(3)?,
            })
        })
        .map_err(se)?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(se)?);
    }
    Ok(out)
}
