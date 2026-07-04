# CLI reference

[← Docs index](README.md) · [Русский](../ru/cli.md)

The `chronograph` binary exposes seven subcommands. Every analysis command takes a repository path (default: current directory) and shares a set of common flags.

```
chronograph <COMMAND> [PATH] [OPTIONS]

Commands:
  analyze    Full/incremental history analysis; builds the .chronograph/cache.duckdb cache
  hotspots   Top hotspots (churn × complexity) to the terminal
  coupling   Top change-coupling pairs (files that change together) to the terminal
  knowledge  Knowledge-concentration risk (bus factor) per file, to the terminal
  age        Line-age distribution (code age / stability) per file, to the terminal
  report     Self-contained HTML report (Overview + Hotspots + Coupling + Knowledge)
  export     Deterministic JSON export of metrics + event stream (for the Web UI / pipelines)
```

Every command that reads a repository runs an **incremental analyze** first, so you can call `hotspots`/`coupling`/`report`/`export` directly — the cache is brought up to `HEAD` automatically.

## Common flags

| Flag | Applies to | Meaning |
|---|---|---|
| `[PATH]` | all | Path to the git repository. Default: `.` |
| `--db <FILE>` | all | Cache location. Default: `<repo>/.chronograph/cache.duckdb` |
| `--exclude <GLOB>` | all | Glob of paths to exclude (vendored/generated). Repeatable. |
| `--top <N>` | hotspots, coupling, knowledge, age | How many rows to print. Default: `20` |
| `--show-names` | knowledge, export | Reveal real author names instead of `Author #N`. Default: **off** |
| `--blame-budget <N>` | knowledge, age, report, export | Per‑file blame budget (revisions × added lines). `0` = unlimited. Default: `10000000` |

---

## `analyze`

Builds or updates the cache. Rarely needed directly (other commands do it for you), but useful to warm the cache or force a full rebuild.

```
chronograph analyze [PATH] [OPTIONS]

Options:
      --db <FILE>                 Cache file (default <repo>/.chronograph/cache.duckdb)
      --exclude <GLOB>            Exclude paths (repeatable)
      --no-incremental            Force a full re-scan instead of incremental
      --mechanical-max-files <N>  Mark commits touching > N files as "mechanical"
```

**Example:**

```bash
chronograph analyze .
# Processed new commits: 925. HEAD: 7d3a…
# Cache: ./.chronograph/cache.duckdb
```

On a second run with no new commits:

```
Cache is up to date (HEAD 7d3a…); no new commits.
```

See [Configuration](configuration.md) for `--no-incremental` and `--mechanical-max-files`.

---

## `hotspots`

Ranks files by `churn × complexity`. Area of highest maintenance risk: changed often **and** structurally complex.

```
chronograph hotspots [PATH] [--top N] [--db FILE] [--exclude GLOB]
```

**Example output** (columns: rank, path, churn, cyclomatic complexity, churn percentile, complexity percentile, score):

```
  #  path                                          churn    cx  churn%    cx%  score
  1  build.rs                                        106    24    0.98   0.99  0.970
  2  src/error.rs                                    199    10    1.00   0.82  0.820
  3  src/context.rs                                   61     9    0.79   0.79  0.624
  4  src/fmt.rs                                       34    15    0.55   0.93  0.512
  7  src/lib.rs                                      301     2    1.00   0.20  0.200
```

Note `src/lib.rs`: huge churn but trivial complexity → ranks low. That is the point — churn alone is not risk.

> Only **live files with cyclomatic complexity** (Rust/Python/Go/JS/TS) are ranked. Files that fall back to the indentation heuristic (docs, configs, unsupported languages) are excluded from the hotspot ranking. See [Metrics → Hotspots](metrics.md#hotspots).

---

## `coupling`

Finds files that **change together**. Reveals hidden architectural dependencies imports don't show.

```
chronograph coupling [PATH] [--top N] [--min-support N] [--db FILE] [--exclude GLOB]
```

- `--min-support <N>` — minimum number of shared commits for a pair to qualify. Default: **5**.

**Example output** (columns: support, ratio, file A, file B):

```
 supp  ratio  file_a                             file_b
   23   0.92  src/error.rs                       src/wrapper.rs
   31   0.79  src/backtrace.rs                   src/error.rs
   20   0.80  src/context.rs                     src/wrapper.rs
   12   0.75  build.rs                           build/probe.rs
```

`error.rs ↔ wrapper.rs` at ratio 0.92 is the "hidden debt" insight: they are architecturally entangled even though nothing imports one from the other. See [Metrics → Change coupling](metrics.md#change-coupling).

---

## `knowledge`

Computes **knowledge concentration** (bus factor) per file from `git blame`. Sorted by **risk** (lowest bus factor and highest top‑owner share first).

```
chronograph knowledge [PATH] [--top N] [--show-names] [--blame-budget N] [--db FILE] [--exclude GLOB]
```

**Authors are anonymized by default** (`Author #N`) — this is a product requirement, not a preference (principle: metrics are about risk, not blame). Use `--show-names` to reveal real names.

**Example output:**

```
files: 118; bus_factor = 1 (concentration risk): 63; blame skipped: 0
 bf  top%  top owner                 file
  1  100%  Author #1                 src/format.rs
  1   96%  Author #1                 src/backtrace.rs
  1   88%  Author #2                 build.rs
  2   61%  Author #1                 src/error.rs
```

A **bus factor of 1** means a single author owns more than half of a file's surviving lines — if they leave, that knowledge leaves with them.

Files too expensive to blame (see [`--blame-budget`](configuration.md#blame-budget)) are skipped and reported explicitly, never silently dropped.

---

## `age`

Shows the **age distribution of surviving lines** per file, from `git blame`. Age is measured in days from `anchor = max(committed_at)` (deterministic, not wall‑clock).

```
chronograph age [PATH] [--top N] [--blame-budget N] [--db FILE] [--exclude GLOB]
```

**Example output** (sorted youngest‑median first — the most actively rewritten code on top):

```
files: 118; median of median-age: 612 d.; blame skipped: 0
 newest  median     p90  oldest  file
      0       4      31      95  src/error.rs
      2      18      66     140  src/context.rs
     12     420     900    1400  src/lib.rs
```

- A **small median** → a zone of constant rewriting.
- A **large median** → stable, old code.

---

## `report`

Generates a single **self‑contained HTML report** (no external requests): Overview + Hotspots treemap + Coupling + Knowledge.

```
chronograph report [PATH] [--out FILE] [--blame-budget N] [--db FILE] [--exclude GLOB]
```

- `--out <FILE>` — output path. Default: `report.html`.

```bash
chronograph report . --out report.html
# Report written: report.html
```

Two runs on the same commit produce a **byte‑identical** file. See [HTML report](html-report.md).

---

## `export`

Produces the **deterministic `chronograph.json`** consumed by the [Web UI](web-ui.md) — metrics plus the full per‑commit event stream.

```
chronograph export [PATH] [--out FILE] [--format json] [--show-names] [--blame-budget N] [--db FILE] [--exclude GLOB]
```

- `--out <FILE>` — output path. Default: `chronograph.json`.
- `--format <json>` — only `json` today (`parquet` is a planned follow‑up).
- `--show-names` — include real author names (default: anonymized).

```bash
chronograph export . --out chronograph.json
# Export written: chronograph.json
```

The document is byte‑identical across runs on the same commit. See the full [JSON export schema](json-export.md).

---

## Exit codes & errors

Chronograph uses human‑readable, contextual error messages (via `anyhow`). A non‑zero exit code means the run failed (e.g., the path is not a git repository, or the cache could not be written). Blame failures on individual files are **not** fatal — they are counted and reported.
