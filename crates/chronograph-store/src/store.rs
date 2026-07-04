//! Реализация [`Store`] поверх DuckDB.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chronograph_core::error::BoxError;
use chronograph_core::model::{AnalysisMeta, Commit};
use chronograph_core::{Error, Result, Store};
use duckdb::types::{TimeUnit, Value};
use duckdb::{params, Connection};

/// DDL схемы из `chronograph-tz.md`, раздел 7.
///
/// `commits.sha` — PRIMARY KEY: даёт идемпотентность через `INSERT OR IGNORE`.
/// Аналитические таблицы создаются пустыми (skeleton) — наполнение на Этапах 1+.
/// `file_changes` имеет nullable `old_path` (отступление от буквы ТЗ §7, явно
/// согласовано) — заполняется для rename/copy, чтобы история переименованных
/// файлов не фрагментировалась в churn/hotspot на Этапе 1 (принцип 2.5 ТЗ).
/// См. запись «пересмотрено» в CONTEXT.md.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS authors (
    author_id      INTEGER PRIMARY KEY,
    canonical_name TEXT,
    canonical_email TEXT
);

CREATE TABLE IF NOT EXISTS commits (
    sha           TEXT PRIMARY KEY,
    author_id     INTEGER,
    committed_at  TIMESTAMP,
    files_changed INTEGER,
    is_mechanical BOOLEAN
);

CREATE TABLE IF NOT EXISTS file_changes (
    sha         TEXT,
    path        TEXT,
    old_path    TEXT,        -- прежний путь для rename/copy (change_type R/C); иначе NULL
    added       INTEGER,
    deleted     INTEGER,
    change_type TEXT,
    blob_sha    TEXT         -- oid git-блоба этого состояния (для complexity по коммиту)
);

-- материализованные метрики (skeleton, Этап 1+)
CREATE TABLE IF NOT EXISTS file_metrics (
    path            TEXT,
    churn_total     INTEGER,
    churn_30d       INTEGER,
    churn_90d       INTEGER,
    churn_365d      INTEGER,     -- расширение §7: окно 365д из ТЗ 3.1 (явно согласовано)
    complexity      REAL,
    complexity_per_loc REAL,
    hotspot_rank    INTEGER,
    is_alive        BOOLEAN
);

CREATE TABLE IF NOT EXISTS coupling (
    path_a              TEXT,
    path_b              TEXT,
    support             INTEGER,
    coupling_ratio      REAL,
    explained_by_imports BOOLEAN
);

CREATE TABLE IF NOT EXISTS knowledge (
    path            TEXT,
    author_id       INTEGER,
    ownership_ratio REAL
);

CREATE TABLE IF NOT EXISTS module_bus_factor (
    module          TEXT,
    bus_factor      INTEGER,
    top_owner_ratio REAL
);

-- code age / stability (§3.6): распределение возраста строк по файлу перцентилями.
-- Расширение §7 (в исходной схеме нет) — согласовано явно, как old_path/churn_365d/
-- blob_sha. Возраст в днях от anchor=max(committed_at). «% старше X дней» НЕ храним
-- (§3.6 не задаёт порог); перцентили — параметр-свободное описание распределения.
CREATE TABLE IF NOT EXISTS file_age (
    path            TEXT,
    lines           INTEGER,
    newest_age_days INTEGER,
    median_age_days INTEGER,
    p90_age_days    INTEGER,
    oldest_age_days INTEGER
);

CREATE TABLE IF NOT EXISTS analysis_meta (
    engine_version TEXT,
    config_hash    TEXT,
    analyzed_at    TIMESTAMP,
    head_sha       TEXT
);
"#;

/// Порог промежуточного сброса буферов записи (строк `file_changes`).
///
/// Ограничивает пиковую память на гигантских репо; на обычных — один сброс в flush.
/// Перф-настройка, не семантика: порядок строк сохраняется при любом значении.
const SPILL_THRESHOLD: usize = 50_000;

/// Хранилище сырых данных истории на DuckDB.
///
/// Внутри одного прогона все записи идут в одной транзакции (производительность +
/// атомарность): транзакция открывается лениво на первой записи и фиксируется в
/// [`Store::flush`].
///
/// Запись — БАТЧЕМ через `duckdb::Appender` (row-wise INSERT — худший паттерн для
/// колоночного DuckDB; профилирование показало ~69% времени analyze). Строки
/// буферизуются в порядке поступления и сбрасываются пачкой — порядок и контент
/// детерминированы, как при построчной записи. Идемпотентность (бывший
/// `INSERT OR IGNORE`) обеспечивается in-memory набором известных sha, загружаемым
/// при открытии (как кэш авторов).
pub struct DuckStore {
    conn: Connection,
    /// Нормализованный email → author_id (кэш для дедупликации авторов).
    authors: HashMap<String, i64>,
    /// Следующий свободный author_id (детерминирован порядком первого появления).
    next_author_id: i64,
    /// Открыта ли транзакция прогона.
    in_txn: bool,
    /// SHA уже записанных коммитов (идемпотентность повторной записи).
    known_shas: HashSet<String>,
    /// Буфер строк `commits`: (sha, author_id, committed_at_unix, files, is_mech).
    pending_commits: Vec<(String, i64, i64, i64, bool)>,
    /// Буфер строк `file_changes` (колонки в порядке схемы).
    #[allow(clippy::type_complexity)]
    pending_changes: Vec<(String, String, Option<String>, i64, i64, String, String)>,
}

impl DuckStore {
    /// Открыть/создать кэш по файловому пути и применить схему.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).map_err(se)?;
        Self::from_conn(conn)
    }

    /// In-memory хранилище (используется в тестах).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(se)?;
        Self::from_conn(conn)
    }

    /// Read-доступ к соединению для аналитического слоя (`chronograph-metrics`).
    ///
    /// Метрики читают плоские таблицы и материализуют результаты через DuckDB
    /// (колоночная аналитика — раздел 5 ТЗ). Граница соблюдена: метрики ходят в
    /// данные только через стор и не знают про gix.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA).map_err(se)?;

        // Загрузить уже известных авторов (для инкрементального прогона).
        let mut authors = HashMap::new();
        let mut max_id = 0_i64;
        {
            let mut stmt = conn
                .prepare("SELECT author_id, canonical_email FROM authors")
                .map_err(se)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(se)?;
            for row in rows {
                let (id, email) = row.map_err(se)?;
                max_id = max_id.max(id);
                authors.insert(email, id);
            }
        }

        // Загрузить sha уже записанных коммитов (идемпотентность батч-записи).
        let mut known_shas = HashSet::new();
        {
            let mut stmt = conn.prepare("SELECT sha FROM commits").map_err(se)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(se)?;
            for row in rows {
                known_shas.insert(row.map_err(se)?);
            }
        }

        Ok(DuckStore {
            conn,
            authors,
            next_author_id: max_id + 1,
            in_txn: false,
            known_shas,
            pending_commits: Vec::new(),
            pending_changes: Vec::new(),
        })
    }

    /// Лениво открыть транзакцию прогона.
    fn ensure_txn(&mut self) -> Result<()> {
        if !self.in_txn {
            self.conn.execute_batch("BEGIN TRANSACTION").map_err(se)?;
            self.in_txn = true;
        }
        Ok(())
    }

    /// Получить (или создать) `author_id` для автора, нормализуя email.
    ///
    /// Нормализация Этапа 0 — по email (trim + lowercase). `.mailmap` — Этап 4.
    fn author_id(&mut self, name: &str, email: &str) -> Result<i64> {
        let key = email.trim().to_lowercase();
        if let Some(&id) = self.authors.get(&key) {
            return Ok(id);
        }
        let id = self.next_author_id;
        self.next_author_id += 1;
        self.conn
            .prepare_cached(
                "INSERT INTO authors (author_id, canonical_name, canonical_email) VALUES (?, ?, ?)",
            )
            .map_err(se)?
            .execute(params![id, name, key])
            .map_err(se)?;
        self.authors.insert(key, id);
        Ok(id)
    }
}

/// Перевести unix-секунды (UTC) в наивный TIMESTAMP DuckDB (без session-tz).
fn to_timestamp(unix_seconds: i64) -> Value {
    Value::Timestamp(
        TimeUnit::Microsecond,
        unix_seconds.saturating_mul(1_000_000),
    )
}

impl Store for DuckStore {
    fn last_head(&self) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT head_sha FROM analysis_meta LIMIT 1")
            .map_err(se)?;
        let mut rows = stmt.query([]).map_err(se)?;
        match rows.next().map_err(se)? {
            Some(row) => Ok(Some(row.get::<_, String>(0).map_err(se)?)),
            None => Ok(None),
        }
    }

    fn write_commit(&mut self, commit: &Commit, is_mechanical: bool) -> Result<()> {
        self.ensure_txn()?;

        // Идемпотентность: sha уже записан (этим или прошлым прогоном) → пропускаем
        // вместе с file_changes. Семантика прежнего INSERT OR IGNORE, но in-memory —
        // батч-запись через Appender конфликтов не разрешает.
        if self.known_shas.contains(&commit.sha) {
            return Ok(());
        }
        let author_id = self.author_id(&commit.author.name, &commit.author.email)?;
        self.known_shas.insert(commit.sha.clone());

        self.pending_commits.push((
            commit.sha.clone(),
            author_id,
            commit.committed_at,
            commit.files_changed() as i64,
            is_mechanical,
        ));
        for fc in &commit.file_changes {
            self.pending_changes.push((
                commit.sha.clone(),
                fc.path.clone(),
                fc.old_path.clone(),
                fc.added as i64,
                fc.deleted as i64,
                fc.change_type.code().to_string(),
                fc.blob_sha.clone(),
            ));
        }

        // Промежуточный сброс — ограничение пиковой памяти на гигантских репо.
        if self.pending_changes.len() >= SPILL_THRESHOLD {
            spill(
                &self.conn,
                &mut self.pending_commits,
                &mut self.pending_changes,
            )?;
        }
        Ok(())
    }

    fn write_meta(&mut self, meta: &AnalysisMeta) -> Result<()> {
        self.ensure_txn()?;
        // Храним одну строку «последнего прогона» — точку инкрементального
        // продолжения. Перезаписываем целиком.
        self.conn
            .execute("DELETE FROM analysis_meta", [])
            .map_err(se)?;
        self.conn
            .execute(
                "INSERT INTO analysis_meta (engine_version, config_hash, analyzed_at, head_sha) \
                 VALUES (?, ?, ?, ?)",
                params![
                    meta.engine_version,
                    meta.config_hash,
                    to_timestamp(meta.analyzed_at),
                    meta.head_sha
                ],
            )
            .map_err(se)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        spill(
            &self.conn,
            &mut self.pending_commits,
            &mut self.pending_changes,
        )?;
        if self.in_txn {
            self.conn.execute_batch("COMMIT").map_err(se)?;
            self.in_txn = false;
        }
        Ok(())
    }
}

/// Сбросить буферы записи в DuckDB батчем через `Appender`.
///
/// Строки аппендятся в порядке буфера (= порядок вызовов `write_commit`) — контент
/// таблиц идентичен построчной записи. Порядок append-а на контент строк не влияет
/// (author_id уже назначен в момент `write_commit`), но сохраняем его для
/// предсказуемости.
#[allow(clippy::type_complexity)]
fn spill(
    conn: &Connection,
    commits: &mut Vec<(String, i64, i64, i64, bool)>,
    changes: &mut Vec<(String, String, Option<String>, i64, i64, String, String)>,
) -> Result<()> {
    if !commits.is_empty() {
        let mut app = conn.appender("commits").map_err(se)?;
        for (sha, author_id, committed_at, files, mech) in commits.drain(..) {
            app.append_row(params![
                sha,
                author_id,
                to_timestamp(committed_at),
                files,
                mech
            ])
            .map_err(se)?;
        }
        app.flush().map_err(se)?;
    }
    if !changes.is_empty() {
        let mut app = conn.appender("file_changes").map_err(se)?;
        for (sha, path, old_path, added, deleted, ct, blob) in changes.drain(..) {
            app.append_row(params![sha, path, old_path, added, deleted, ct, blob])
                .map_err(se)?;
        }
        app.flush().map_err(se)?;
    }
    Ok(())
}

/// Обернуть ошибку duckdb в ошибку хранилища ядра.
fn se<E: Into<BoxError>>(e: E) -> Error {
    Error::store(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chronograph_core::model::{Author, ChangeType, FileChange};

    fn commit(sha: &str, email: &str, files: &[(&str, ChangeType, u64, u64)]) -> Commit {
        Commit {
            sha: sha.to_string(),
            parent_shas: vec![],
            author: Author {
                name: "Tester".into(),
                email: email.into(),
            },
            committed_at: 1_700_000_000,
            file_changes: files
                .iter()
                .map(|(p, ct, a, d)| FileChange {
                    path: (*p).to_string(),
                    old_path: None,
                    added: *a,
                    deleted: *d,
                    change_type: *ct,
                    blob_sha: format!("blob-{p}"),
                })
                .collect(),
        }
    }

    fn count(store: &DuckStore, table: &str) -> i64 {
        store
            .conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn writes_commit_and_file_changes() {
        let mut store = DuckStore::open_in_memory().unwrap();
        store
            .write_commit(
                &commit("abc", "a@x.io", &[("f.rs", ChangeType::Added, 3, 0)]),
                false,
            )
            .unwrap();
        store.flush().unwrap();

        assert_eq!(count(&store, "commits"), 1);
        assert_eq!(count(&store, "file_changes"), 1);
        assert_eq!(count(&store, "authors"), 1);
    }

    #[test]
    fn persists_old_path_for_rename() {
        let mut store = DuckStore::open_in_memory().unwrap();
        let c = Commit {
            sha: "ren".into(),
            parent_shas: vec![],
            author: Author {
                name: "T".into(),
                email: "a@x.io".into(),
            },
            committed_at: 1_700_000_000,
            file_changes: vec![
                FileChange {
                    path: "new.rs".into(),
                    old_path: Some("old.rs".into()),
                    added: 0,
                    deleted: 0,
                    change_type: ChangeType::Renamed,
                    blob_sha: "blob-new".into(),
                },
                FileChange {
                    path: "plain.rs".into(),
                    old_path: None,
                    added: 1,
                    deleted: 0,
                    change_type: ChangeType::Added,
                    blob_sha: "blob-plain".into(),
                },
            ],
        };
        store.write_commit(&c, false).unwrap();
        store.flush().unwrap();

        let renamed_old: Option<String> = store
            .conn
            .query_row(
                "SELECT old_path FROM file_changes WHERE change_type = 'R'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(renamed_old.as_deref(), Some("old.rs"));

        // Для не-rename old_path остаётся NULL.
        let added_old: Option<String> = store
            .conn
            .query_row(
                "SELECT old_path FROM file_changes WHERE change_type = 'A'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(added_old, None);
    }

    #[test]
    fn duplicate_commit_is_idempotent() {
        let mut store = DuckStore::open_in_memory().unwrap();
        let c = commit("dup", "a@x.io", &[("f.rs", ChangeType::Modified, 1, 1)]);
        store.write_commit(&c, false).unwrap();
        // Повторная запись того же sha не должна задвоить file_changes.
        store.write_commit(&c, false).unwrap();
        store.flush().unwrap();

        assert_eq!(count(&store, "commits"), 1);
        assert_eq!(count(&store, "file_changes"), 1);
    }

    #[test]
    fn authors_are_deduplicated_by_email() {
        let mut store = DuckStore::open_in_memory().unwrap();
        store
            .write_commit(
                &commit("c1", "Same@X.io", &[("a", ChangeType::Added, 1, 0)]),
                false,
            )
            .unwrap();
        // Тот же email в другом регистре → тот же author_id.
        store
            .write_commit(
                &commit("c2", "same@x.io", &[("b", ChangeType::Added, 1, 0)]),
                false,
            )
            .unwrap();
        store.flush().unwrap();

        assert_eq!(count(&store, "authors"), 1);
    }

    #[test]
    fn last_head_roundtrips() {
        let mut store = DuckStore::open_in_memory().unwrap();
        assert_eq!(store.last_head().unwrap(), None);

        store
            .write_meta(&AnalysisMeta {
                engine_version: "0.0.0".into(),
                config_hash: "deadbeef".into(),
                analyzed_at: 1_700_000_000,
                head_sha: "headsha".into(),
            })
            .unwrap();
        store.flush().unwrap();

        assert_eq!(store.last_head().unwrap(), Some("headsha".to_string()));
    }

    #[test]
    fn store_is_deterministic_for_same_input() {
        // Один и тот же вход в два независимых стора → идентичный дамп таблиц.
        let input = vec![
            commit("c1", "a@x.io", &[("a.rs", ChangeType::Added, 2, 0)]),
            commit("c2", "b@x.io", &[("a.rs", ChangeType::Modified, 1, 1)]),
        ];
        let dump = |commits: &[Commit]| {
            let mut store = DuckStore::open_in_memory().unwrap();
            for c in commits {
                store.write_commit(c, false).unwrap();
            }
            store.flush().unwrap();
            dump_tables(&store)
        };
        assert_eq!(dump(&input), dump(&input));
    }

    /// Детерминированный текстовый дамп сырых таблиц (для сравнения прогонов).
    ///
    /// Число колонок задаётся явно: в duckdb `Statement::column_count()` доступен
    /// только после выполнения, а индексный `row.get(i)` работает по факту запроса.
    fn dump_tables(store: &DuckStore) -> String {
        let mut out = String::new();
        for (table, order, ncols) in [
            ("authors", "author_id", 3usize),
            ("commits", "sha", 5),
            ("file_changes", "sha, path, change_type", 7),
            ("analysis_meta", "head_sha", 4),
        ] {
            out.push_str(&format!("== {table} ==\n"));
            let sql = format!("SELECT * FROM {table} ORDER BY {order}");
            let mut stmt = store.conn.prepare(&sql).unwrap();
            let rows = stmt
                .query_map([], |row| {
                    let mut line = String::new();
                    for i in 0..ncols {
                        let v: Value = row.get(i)?;
                        line.push_str(&format!("{v:?}|"));
                    }
                    Ok(line)
                })
                .unwrap();
            for r in rows {
                out.push_str(&r.unwrap());
                out.push('\n');
            }
        }
        out
    }
}
