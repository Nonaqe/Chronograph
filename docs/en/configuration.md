# Configuration

[← Docs index](README.md) · [Русский](../ru/configuration.md)

Chronograph favors **explicit, configurable thresholds** over hard‑coded constants. This page collects every knob and its default.

## Exclusion globs — `--exclude`

Exclude vendored or generated paths from the analysis. Repeatable.

```bash
chronograph hotspots . \
  --exclude "vendor/**" \
  --exclude "**/*.min.js" \
  --exclude "third_party/**"
```

Excluded paths never enter the analysis, so they can't skew churn, coupling, hotspots, or knowledge.

## Churn windows

Churn is computed over three rolling windows with defaults **30 / 90 / 365 days**, taken directly from the specification. Windows are measured from `anchor = max(committed_at)` (the last activity in the history), **not** wall‑clock now — this is what keeps output reproducible. The defaults are configurable in the library (`ChurnConfig`); the CLI uses the specified defaults.

## Coupling support — `--min-support`

```bash
chronograph coupling . --min-support 5     # default
```

A pair of files must share at least `min_support` commits to appear. The default of **5** filters coincidental one‑off co‑changes. Lower it to surface more (noisier) pairs; raise it for only the strongest couplings.

## Mechanical commits

Flag: `--mechanical-max-files`

```bash
chronograph analyze . --mechanical-max-files 50
```

A commit touching more than **N** files is flagged as *mechanical* (bulk rename, format sweep, vendored drop, release bump). Mechanical commits distort churn and coupling, so both **exclude them by default** once the heuristic is enabled.

> **The default is off** (no threshold). A good value depends on the repository, and the specification doesn't fix one — Chronograph will not invent a magic number. Enable it explicitly when a repository has bulk commits polluting the signal. The flag is set at `analyze` time (it's a property of ingestion).

## Blame budget

Flag: `--blame-budget`

```bash
chronograph knowledge . --blame-budget 10000000   # default
chronograph age .       --blame-budget 0          # unlimited
```

Blame powers the **knowledge** and **code‑age** metrics. Blaming one pathologically large, frequently‑rewritten file (e.g. a multi‑megabyte generated `CHANGELOG` with hundreds of revisions) is indivisible and can take tens of minutes; a cache doesn't help because such a file is invalidated by almost every commit.

The budget caps per‑file cost as:

```
cost = revisions × total_added_lines
```

Files whose cost exceeds the budget are **skipped and reported** (in the `blame_skips` list / the CLI's skip summary / the report), never silently dropped.

- **Default: 10,000,000.** Chosen from data: normal code has `cost < 1M`; pathological generated giants exceed 30M. The default cuts the gap with a ~10× margin on both sides. On a repo like ripgrep it excludes nothing (max ≈ 1M).
- **`0` = unlimited** — blame everything, however slow.

`--blame-budget` applies to `knowledge`, `age`, `report`, and `export`.

## Author names — `--show-names`

```bash
chronograph knowledge . --show-names
chronograph export . --show-names --out chronograph.json
```

By default the knowledge metric and the JSON export **anonymize** authors as `Author #N`. `--show-names` reveals real names. Default‑off is a product requirement: the knowledge map is a *concentration risk*, not a credit/blame ledger.

## Cache location — `--db`

```bash
chronograph report . --db /tmp/chronograph-cache.duckdb
```

Relocate the DuckDB cache. Default: `<repo>/.chronograph/cache.duckdb`.

## Full rebuild — `--no-incremental`

```bash
chronograph analyze . --no-incremental
```

Force a complete re‑scan instead of processing only new commits. Useful after upgrading the engine or if you suspect a stale cache. (Deleting `.chronograph/` achieves the same thing.)

## Defaults at a glance

| Setting | Default | Flag |
|---|---|---|
| Cache path | `<repo>/.chronograph/cache.duckdb` | `--db` |
| Churn windows | 30 / 90 / 365 days | *(library)* |
| Coupling min support | 5 | `--min-support` |
| Mechanical threshold | off | `--mechanical-max-files` |
| Blame budget | 10,000,000 | `--blame-budget` |
| Author names | anonymized | `--show-names` |
| Top‑N rows | 20 | `--top` |
| Incremental | on | `--no-incremental` |
