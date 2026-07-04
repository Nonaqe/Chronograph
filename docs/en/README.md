# Chronograph documentation

[← Project README](../../README.md) · [Русский](../ru/README.md)

Welcome to the Chronograph documentation. Chronograph is a git‑repository evolution analytics engine: it reads history and produces transparent, deterministic signals about where a codebase is risky.

## Contents

1. **[Installation](installation.md)** — build from source, requirements, the binary, the cache.
2. **[CLI reference](cli.md)** — every command (`analyze`, `hotspots`, `coupling`, `knowledge`, `age`, `report`, `export`) and every flag, with example output.
3. **[Metrics explained](metrics.md)** — precise definitions and formulas for churn, complexity, hotspots, change coupling, knowledge / bus factor, and code age.
4. **[GitHub Action](github-action.md)** — run Chronograph in CI, inputs, workflow examples, GitHub Pages publishing.
5. **[HTML report](html-report.md)** — what the self‑contained `report.html` contains and how it's built.
6. **[Web UI](web-ui.md)** — the six interactive tabs, how to run it and load data, with screenshots.
7. **[JSON export](json-export.md)** — the full `chronograph.json` schema (version 1).
8. **[Architecture](architecture.md)** — crates, dependency boundaries, data flow, the DuckDB schema, and how determinism is guaranteed.
9. **[Configuration](configuration.md)** — churn windows, exclusion globs, the mechanical‑commit threshold, the blame budget, and coupling support.
10. **[FAQ & troubleshooting](faq.md)** — common problems, privacy/anonymization, and the project's anti‑goals.

## The 30‑second mental model

```
        git history
             │
       ┌─────▼─────┐   gix (gitoxide) walks commits, diffs, renames
       │  ingest   │   → stored in a local DuckDB cache (.chronograph/)
       └─────┬─────┘
             │
       ┌─────▼─────┐   churn · complexity (tree-sitter) · coupling
       │  metrics  │   knowledge (blame) · code age · hotspots
       └─────┬─────┘
             │
   ┌─────────┼──────────┐
   ▼         ▼          ▼
 CLI      report.html  chronograph.json ──► web UI
 tables   (Action)     (deterministic export)
```

Everything is incremental: the last analyzed `head_sha` is stored, so re‑running only processes new commits.
