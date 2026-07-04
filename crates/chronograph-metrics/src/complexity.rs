//! Complexity живых файлов на HEAD — из git-объектов, с кэшем по blob_sha.
//!
//! **Что делаем:** для каждого живого (не удалённого на HEAD) файла берём его
//! канонический путь и SHA git-блоба текущего состояния, достаём байты через
//! [`BlobReader`] (внутри — gix; metrics его не видит) и считаем сложность через
//! `chronograph-lang`.
//!
//! **Почему из git-объекта, а не с диска:** контент-адресность даёт детерминизм и
//! историчность — complexity считается по файлу ровно таким, каким он был на
//! коммите, независимо от состояния рабочего дерева (см. решение в CONTEXT.md).
//!
//! **Кэш:** complexity — чистая функция (контент блоба, язык), поэтому кэшируется
//! по ключу `(blob_sha, lang)` в таблице `complexity_cache`. Неизменные файлы (тот
//! же oid) на повторном прогоне не пересчитываются — инкрементальность.

use chronograph_core::{BlobReader, Result};
use chronograph_lang::{file_complexity, ComplexityMethod, SupportedLanguage};
use chronograph_store::DuckStore;
use duckdb::{params, Connection};

use crate::store_err as se;

/// Complexity одного живого файла на HEAD.
#[derive(Debug, Clone, PartialEq)]
pub struct FileComplexityRow {
    /// Канонический путь файла на HEAD.
    pub path: String,
    /// SHA git-блоба текущего состояния.
    pub blob_sha: String,
    /// Значение сложности (cyclomatic или indentation).
    pub value: f64,
    /// Сложность на строку.
    pub per_loc: f64,
    /// Непустых строк.
    pub loc: u32,
    /// Метод подсчёта.
    pub method: ComplexityMethod,
    /// Язык (если считалось по AST; иначе `None`).
    pub language: Option<SupportedLanguage>,
}

/// Посчитать complexity всех живых файлов на HEAD, используя `reader` для контента.
///
/// Результаты кэшируются по `(blob_sha, lang)`; повторный вызов на неизменных
/// файлах не читает блобы и не парсит заново. Порядок детерминирован (по пути).
pub fn compute_complexity<R: BlobReader>(
    store: &DuckStore,
    reader: &R,
) -> Result<Vec<FileComplexityRow>> {
    let conn = store.conn();
    ensure_cache_table(conn)?;

    let canonical = crate::paths::build_canonical_map(conn)?;
    crate::paths::materialize_path_map(conn, &canonical)?;

    let head_blobs = query_head_blobs(conn)?;

    let mut out = Vec::with_capacity(head_blobs.len());
    for (path, blob_sha) in head_blobs {
        let language = SupportedLanguage::detect_by_path(&path);
        let lang_tag = language_tag(language);

        if let Some((value, per_loc, loc, method)) = cache_get(conn, &blob_sha, lang_tag)? {
            out.push(FileComplexityRow {
                path,
                blob_sha,
                value,
                per_loc,
                loc,
                method,
                // При cyclomatic язык однозначно = язык ключа; при indentation — None.
                language: match method {
                    ComplexityMethod::Cyclomatic => language,
                    ComplexityMethod::Indentation => None,
                },
            });
            continue;
        }

        let bytes = reader.read_blob(&blob_sha)?;
        let fc = file_complexity(&path, &bytes);
        cache_put(
            conn, &blob_sha, lang_tag, fc.value, fc.per_loc, fc.loc, fc.method,
        )?;
        out.push(FileComplexityRow {
            path,
            blob_sha,
            value: fc.value,
            per_loc: fc.per_loc,
            loc: fc.loc,
            method: fc.method,
            language: fc.language,
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Живые файлы на HEAD: `(канонический путь, blob_sha текущего состояния)`.
///
/// Механический фильтр НЕ применяется — текущее состояние файла определяется
/// фактически последним изменением, каким бы ни был коммит.
fn query_head_blobs(conn: &Connection) -> Result<Vec<(String, String)>> {
    // Тай-брейк ПОЛНЫЙ (как в churn/paths): при склейке имён несколько строк одного
    // коммита мапятся на один canonical — (ts, sha) равны; не-'D' выигрывает, далее
    // change_type и сырой путь (иначе blob_sha текущего состояния недетерминирован).
    let sql = "WITH mapped AS (
                   SELECT pm.canonical AS path, fc.path AS raw_path,
                          fc.change_type, fc.blob_sha,
                          CAST(epoch(c.committed_at) AS BIGINT) AS ts, fc.sha AS sha
                   FROM file_changes fc
                   JOIN commits c ON fc.sha = c.sha
                   JOIN path_map pm ON fc.path = pm.path
               ),
               ranked AS (
                   SELECT path, change_type, blob_sha,
                          row_number() OVER (PARTITION BY path
                              ORDER BY ts DESC, sha DESC,
                                       CASE WHEN change_type = 'D' THEN 1 ELSE 0 END,
                                       change_type, raw_path) AS rn
                   FROM mapped
               )
               SELECT path, blob_sha FROM ranked
               WHERE rn = 1 AND change_type <> 'D'
               ORDER BY path";
    let mut stmt = conn.prepare(sql).map_err(se)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(se)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(se)?);
    }
    Ok(out)
}

fn ensure_cache_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS complexity_cache (
             blob_sha TEXT,
             lang     TEXT,
             value    DOUBLE,
             per_loc  DOUBLE,
             loc      INTEGER,
             method   TEXT,
             PRIMARY KEY (blob_sha, lang)
         );",
    )
    .map_err(se)?;
    Ok(())
}

#[allow(clippy::type_complexity)]
fn cache_get(
    conn: &Connection,
    blob_sha: &str,
    lang: &str,
) -> Result<Option<(f64, f64, u32, ComplexityMethod)>> {
    let mut stmt = conn
        .prepare(
            "SELECT value, per_loc, loc, method FROM complexity_cache \
             WHERE blob_sha = ? AND lang = ?",
        )
        .map_err(se)?;
    let mut rows = stmt.query(params![blob_sha, lang]).map_err(se)?;
    match rows.next().map_err(se)? {
        Some(row) => {
            let value: f64 = row.get(0).map_err(se)?;
            let per_loc: f64 = row.get(1).map_err(se)?;
            let loc: i64 = row.get(2).map_err(se)?;
            let method = method_from_tag(&row.get::<_, String>(3).map_err(se)?);
            Ok(Some((value, per_loc, loc as u32, method)))
        }
        None => Ok(None),
    }
}

fn cache_put(
    conn: &Connection,
    blob_sha: &str,
    lang: &str,
    value: f64,
    per_loc: f64,
    loc: u32,
    method: ComplexityMethod,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO complexity_cache (blob_sha, lang, value, per_loc, loc, method) \
         VALUES (?, ?, ?, ?, ?, ?)",
        params![
            blob_sha,
            lang,
            value,
            per_loc,
            loc as i64,
            method_tag(method)
        ],
    )
    .map_err(se)?;
    Ok(())
}

/// Тег языка для ключа кэша (по расширению; `none` — fallback-язык).
fn language_tag(lang: Option<SupportedLanguage>) -> &'static str {
    match lang {
        Some(SupportedLanguage::Rust) => "rust",
        Some(SupportedLanguage::Python) => "python",
        Some(SupportedLanguage::Go) => "go",
        Some(SupportedLanguage::JavaScript) => "js",
        Some(SupportedLanguage::TypeScript) => "ts",
        Some(SupportedLanguage::Tsx) => "tsx",
        None => "none",
    }
}

fn method_tag(m: ComplexityMethod) -> &'static str {
    match m {
        ComplexityMethod::Cyclomatic => "cyclomatic",
        ComplexityMethod::Indentation => "indentation",
    }
}

fn method_from_tag(t: &str) -> ComplexityMethod {
    match t {
        "cyclomatic" => ComplexityMethod::Cyclomatic,
        _ => ComplexityMethod::Indentation,
    }
}
