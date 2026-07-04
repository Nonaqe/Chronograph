# JSON export schema

[← Docs index](README.md) · [Русский](../ru/json-export.md)

`chronograph export` writes a single deterministic `chronograph.json`. It is the contract between the engine and the [Web UI](web-ui.md), but it's self‑contained for any pipeline. This page documents **schema version 1** (`meta.schema_version === 1`).

```bash
chronograph export /path/to/repo --out chronograph.json
```

The document is **byte‑identical** across runs on the same commit (sorted keys, fixed float formatting).

## Top‑level shape

```jsonc
{
  "meta":        { ... },   // repository + run metadata
  "files":       [ ... ],   // per-file metrics (hotspots, churn, complexity)
  "coupling":    [ ... ],   // change-coupling pairs
  "knowledge":   [ ... ],   // bus factor per file
  "file_age":    [ ... ],   // line-age distribution per file
  "blame_skips": [ ... ],   // files skipped during blame, with reasons
  "events":      [ ... ]    // full per-commit event stream
}
```

## `meta`

```jsonc
{
  "schema_version": 1,
  "engine_version": "0.0.0",
  "config_hash": "f283ba95ccc573f7",   // hash of the analysis config
  "head_sha": "7634900254ca…",          // commit the export reflects
  "anchor_ts": 1622505600,              // unix seconds UTC; age days are measured from here
  "total_commits": 7,
  "total_authors": 1,
  "anonymized": true                    // true unless --show-names was used
}
```

## `files[]` — file metrics

Nullable fields are `null` when not applicable (e.g. no complexity for an unsupported language).

```jsonc
{
  "path": "a.rs",
  "churn_total": 6,
  "churn_30d": 6,
  "churn_90d": 6,
  "churn_365d": 6,
  "complexity": 3.0,
  "complexity_per_loc": 3.0,
  "hotspot_rank": 1,        // 1 = hottest; null if not ranked
  "is_alive": true          // false if the file was deleted
}
```

## `coupling[]` — change coupling

```jsonc
{ "a": "a.rs", "b": "c.rs", "support": 6, "ratio": 1.0 }
```

`a < b` canonically; `ratio = support / min(commits(a), commits(b))` ∈ (0, 1]. See [Metrics → Change coupling](metrics.md#change-coupling).

## `knowledge[]` — bus factor

```jsonc
{ "path": "a.rs", "bus_factor": 1, "top_owner_ratio": 1.0, "top_owner": "Author #1" }
```

`top_owner` is `Author #N` unless `--show-names` was passed.

## `file_age[]` — line age

```jsonc
{
  "path": "a.rs",
  "lines": 1,
  "newest_age_days": 0,
  "median_age_days": 0,
  "p90_age_days": 0,
  "oldest_age_days": 0
}
```

Ages are in days from `meta.anchor_ts`.

## `blame_skips[]` — skipped files

Files too expensive to blame (or where blame failed) — reported, never silently dropped.

```jsonc
{ "path": "CHANGELOG.md", "reason": "over_budget", "cost": 37000000, "budget": 10000000 }
```

`reason` is `over_budget` (with `cost`/`budget`) or `failed` (`cost`/`budget` null).

## `events[]` — per‑commit event stream

The full history, in deterministic order. This drives the Timeline and Repository‑growth views.

```jsonc
{
  "sha": "2790fc39…",
  "ts": 1622505600,           // unix seconds UTC
  "author": "Author #1",      // anonymized unless --show-names
  "mechanical": false,        // was this a mechanical commit?
  "changes": [
    {
      "path": "a.rs",
      "type": "M",            // A add · M modify · D delete · R rename · C copy
      "old_path": null,       // previous path for R/C, else null
      "added": 1,
      "deleted": 1
    }
  ]
}
```

## Consuming it

The TypeScript types in `web/src/types.ts` mirror this schema exactly (`meta.schema_version === 1`). If the Rust export changes, those types change in lockstep — treat `schema_version` as your compatibility gate.

> `parquet` output (`--format parquet`) is a planned follow‑up; the flag is reserved so the CLI won't need to change.
