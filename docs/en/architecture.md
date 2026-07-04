# Architecture

[← Docs index](README.md) · [Русский](../ru/architecture.md)

Chronograph is a Cargo workspace with strict crate boundaries. The boundaries are the point: they keep the analysis layer reusable and independent of any particular git backend.

## Crates

```
chronograph-core     data model, config, orchestration, shared traits
chronograph-git      gix wrapper: history walk, diff, renames, blob reads
chronograph-lang     tree-sitter, per-language complexity
chronograph-metrics  churn, coupling, knowledge, code age, hotspot — one module each
chronograph-store    DuckDB: schema, migrations, read/write
chronograph-report   self-contained HTML + JSON export
chronograph-cli      the `chronograph` binary
```

### Dependency rules

- **`chronograph-metrics` does not depend on `chronograph-git`.** It works only against data in the store. The git layer *fills* the store; the metrics layer *reads* it. Heavy aggregations run inside DuckDB.
- **Nothing depends on `chronograph-cli`.** The CLI is the top consumer; the pipeline is reusable as a library.
- **`chronograph-core` pulls no heavy dependencies** (no gix / tree‑sitter / duckdb) — only the data model and traits.

Metrics reach content they *do* need (file bytes for complexity, blame) through **traits** implemented by `chronograph-git`, so they never depend on gix directly:

- `CommitSource` — history walk (implemented by `GitSource`).
- `Store` — persistence (implemented by `DuckStore`).
- `BlobReader` — read a git blob by `blob_sha` (implemented by `GitSource`).

## Data flow

```
 ┌──────────────┐   run_analysis(source, store, config)
 │ chronograph- │   • gix walks new commits (incremental via head_sha)
 │ git (gix)    │   • extracts sha/parents/author/time
 │              │   • tree-diff vs first parent, rename detection
 └──────┬───────┘   • per-line added/deleted, blob_sha
        │  writes
        ▼
 ┌──────────────┐   DuckDB cache at <repo>/.chronograph/cache.duckdb
 │ chronograph- │   authors, commits, file_changes (+ materialized
 │ store (DuckDB)│  file_metrics, coupling, knowledge, file_age)
 └──────┬───────┘
        │  reads
        ▼
 ┌──────────────┐   compute_churn / compute_complexity / compute_coupling
 │ chronograph- │   compute_knowledge / compute_age / compute_hotspots
 │ metrics      │   materialize() writes results back into the store
 └──────┬───────┘
        │
        ▼
   CLI tables · report.html · chronograph.json
```

CLI commands like `hotspots`/`coupling` compute **on the fly** for freshness; `report`/`export` first **materialize** the analytic tables. Both call the same `compute_*` functions, so there is no divergence — one source of truth.

## DuckDB schema

The store persists these tables (a stable subset is the specified schema; `old_path`, `churn_365d`, `blob_sha`, and `file_age` are agreed extensions):

```sql
authors(author_id, canonical_name, canonical_email)
commits(sha, author_id, committed_at, files_changed, is_mechanical)
file_changes(sha, path, old_path, added, deleted, change_type, blob_sha)

-- materialized analytics
file_metrics(path, churn_total, churn_30d, churn_90d, churn_365d,
             complexity, complexity_per_loc, hotspot_rank, is_alive)
coupling(path_a, path_b, support, coupling_ratio, explained_by_imports)
knowledge(path, author_id, ownership_ratio)
module_bus_factor(module, bus_factor, top_owner_ratio)
file_age(path, lines, newest_age_days, median_age_days, p90_age_days, oldest_age_days)

analysis_meta(engine_version, config_hash, analyzed_at, head_sha)
```

DuckDB is chosen for **columnar analytics locally, with no server** — the co‑occurrence self‑join and windowed churn counts run in SQL.

## Incrementality

`analysis_meta.head_sha` stores the last analyzed head. On a re‑run, the old head is passed to the revision walk as a *hidden* tip, so only new commits are traversed. Idempotent inserts (`INSERT OR IGNORE` keyed on `commits.sha`) protect against double‑processing and history edits.

## Determinism

Same repo + same config → **byte‑identical** output. Guaranteed by:

- **UTC everywhere.** Time is `i64` unix‑seconds in the core; no session timezone.
- **Fixed traversal order.** The revision walk is deterministic; `author_id` is assigned by first appearance in that order.
- **Deterministic aggregation.** Rank‑percentiles and sorts tie‑break by path; no reliance on `HashMap` iteration order in output.
- **Stable serialization.** JSON keys sorted, floats formatted with fixed precision; SVG numbers `{:.2}`, colors integer `rgb()`.
- **Provenance in every artifact.** `engine_version`, `config_hash`, `head_sha` are written into each report/export. The wall‑clock `analyzed_at` is kept out of byte‑identity‑breaking positions.

A reproducibility test runs the pipeline twice and asserts identical output; the HTML report has its own byte‑identity test.

## Error handling

- Library crates define their own error types via **`thiserror`** and avoid `unwrap()`/`expect()` outside proven‑impossible invariants.
- The CLI uses **`anyhow`** with `.context(...)` for human‑readable messages.
