# FAQ & troubleshooting

[← Docs index](README.md) · [Русский](../ru/faq.md)

## General

### Does Chronograph score developers?

No — and it deliberately can't. Metrics are about **files and modules**, not people. The one people‑adjacent signal, the knowledge map, is presented only as a **concentration risk** (bus factor), with authors **anonymized by default** (`Author #N`). This is a product requirement, not a preference.

### Which languages are supported?

For **cyclomatic complexity** (and therefore hotspot ranking): **Rust, Python, Go, JavaScript/TypeScript** (incl. TSX). Other languages get an indentation‑based complexity fallback but are excluded from hotspot ranking. Churn, coupling, knowledge, and code age work for any file.

### Does it call the `git` binary?

No. Chronograph reads repositories directly through **gix (gitoxide)**. The `git` CLI is only used to build test fixtures.

### Is my data sent anywhere?

No. Everything runs locally. The HTML report and JSON export are self‑contained files with zero external requests. (The GitHub Action downloads a *binary* from a public release; it never uploads your code.)

## Determinism & reproducibility

### Why do two runs produce identical output?

By design — same repo + config → **byte‑identical** results. Timestamps are UTC, traversal order is fixed, aggregations tie‑break deterministically, and serialization is stable. See [Architecture → Determinism](architecture.md#determinism).

### The web UI animation looks different each time — is that a bug?

No. Byte‑identity applies to the **`chronograph.json` artifact**, not to the live UI. The growth/timeline rendering uses physics and time and is intentionally not pixel‑reproducible.

## Troubleshooting

### "not a git repository" / open errors

Point Chronograph at the repository root (the directory containing `.git`), and make sure it's a real git repo. In CI, remember `fetch-depth: 0` so the full history is present.

### The GitHub Action fails to download the binary (404)

The Action downloads a release asset via anonymous `curl`, so the **upstream repository must be public** and a matching release must exist for the pinned `version`/tag. On non‑Linux‑x64 runners the Action fails intentionally with a clear message.

### The first build takes forever

Chronograph compiles a **bundled DuckDB** from source on the first build (plus tree‑sitter grammars and gix). This is a one‑time cost; later builds are incremental. Avoid `cargo clean`, which discards that cache.

### `cargo test --workspace` occasionally fails on Windows with an rlib error

This is a known cargo build race when many heavy test binaries link in parallel (each statically links duckdb/arrow/gix). It is **not** a code failure. Workarounds: build serially (`cargo test -j 1`), or run tests per‑crate. It typically converges within a retry.

### Some files are missing from the knowledge / age tables

They were likely **skipped by the blame budget** (very large, frequently‑rewritten files). Check the skip summary / `blame_skips` list, and raise or disable the budget with `--blame-budget` (see [Configuration](configuration.md#blame-budget)).

### Schema / cache errors after upgrading

Delete the cache (`<repo>/.chronograph/`) and re‑run, or use `--no-incremental` to force a full rebuild.

## Anti‑goals — what Chronograph will not become

- **Individual developer productivity scoring.**
- **Manager DORA metrics** (that's the LinearB/Swarmia market).
- **A real‑time linter** (that's SonarQube).
- **"Every language at once"** — the launch set is JS/TS, Python, Go, Rust.
- **ML defect prediction** (a separate problem needing a bug dataset).

## Performance tips

- Always use the **release** binary; debug is 5–20× slower on CPU‑bound paths.
- Use `--exclude` to drop vendored/generated trees.
- Enable `--mechanical-max-files` on repos with bulk commits.
- Tune `--blame-budget` if knowledge/age runs are slow on giant files.
- Re‑runs are incremental — only new commits are processed.
