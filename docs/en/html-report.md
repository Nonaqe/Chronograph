# HTML report

[← Docs index](README.md) · [Русский](../ru/html-report.md)

`chronograph report` renders a single **self‑contained** `report.html`: all CSS is inlined, all data is embedded as JSON, and charts are server‑side SVG generated in Rust. There are **zero external requests** — no CDN, no fonts, no scripts fetched at view time. You can email the file, commit it, or host it anywhere.

```bash
chronograph report /path/to/repo --out report.html
```

## What's inside

| Section | Content |
|---|---|
| **Overview** | Repository metadata: engine version, config hash, `head_sha`, commit/file/author counts. |
| **Hotspots** | A **squarified treemap** where area = complexity and color = churn (pale → deep red). |
| **Coupling** | A table of the top change‑coupling pairs (support and ratio). |
| **Knowledge** | Per‑file bus factor and top‑owner share, with authors anonymized by default. |

The Hotspots treemap is drawn as SVG directly in Rust (squarified / Bruls layout). Input is sorted by `(complexity desc, path asc)` before layout, so equal‑area cells lay out reproducibly.

## Determinism

Two runs on the same commit produce a **byte‑identical** `report.html`. This is guaranteed by:

- Deterministic JSON serialization (sorted, fixed float formatting).
- Server‑side SVG (no browser layout engine involved).
- Only deterministic metadata in the file — `engine_version`, `config_hash`, `head_sha`. The wall‑clock `analyzed_at` is **not** embedded in a way that would break byte‑identity.

A reproducibility test asserts this byte‑for‑byte.

## Options

| Flag | Meaning |
|---|---|
| `--out <FILE>` | Output path (default `report.html`). |
| `--db <FILE>` | Cache location. |
| `--exclude <GLOB>` | Exclude paths (repeatable). |
| `--blame-budget <N>` | Per‑file blame budget for the knowledge section (default 10,000,000; `0` = unlimited). |

## Report vs. web UI

The HTML report is **static and self‑contained** — ideal for CI artifacts and sharing. For **interactive** exploration (force graph, zoomable treemaps, timeline scrubbing, the growth animation), use the [Web UI](web-ui.md) with a `chronograph export` JSON. The report favors portability; the web UI favors interactivity.
