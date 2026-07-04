# Metrics explained

[← Docs index](README.md) · [Русский](../ru/metrics.md)

Every Chronograph signal has a **transparent, documented definition**. There is no opaque "health score". This page gives the exact formula and the rationale — *why* each metric correlates with a real problem — for every signal.

A cross‑cutting rule: **all time windows are measured from `anchor = max(committed_at)`** (the repository's last activity), never from wall‑clock "now". This guarantees the same repository always yields the same numbers ([determinism](architecture.md#determinism)).

## Contents

- [Churn](#churn)
- [Complexity](#complexity)
- [Hotspots](#hotspots)
- [Change coupling](#change-coupling)
- [Knowledge / bus factor](#knowledge--bus-factor)
- [Code age](#code-age)
- [Mechanical commits](#mechanical-commits)

---

## Churn

**What it measures:** how often and how much a file changes.

**Definition:** for each file, churn is the **number of commits that touched it**, computed over the whole history and over three rolling windows:

| Field | Window |
|---|---|
| `churn_total` | entire history |
| `churn_30d` | last 30 days before the anchor |
| `churn_90d` | last 90 days |
| `churn_365d` | last 365 days |

Windows are configurable (defaults 30/90/365 from the specification). Chronograph also tracks total added/deleted lines.

**Rename continuity:** churn follows renames. When `b.txt` is renamed to `c.txt`, their histories are glued into a single canonical file rather than fragmented into two — otherwise a renamed file would look artificially "young" and low‑churn. This is why renames are stored (`old_path`) during ingestion.

**Why it correlates with problems:** frequently changed code is where bugs concentrate and where the cost of misunderstanding is paid repeatedly. Churn alone is not risk (a constantly‑touched but trivial file is fine) — it becomes risk when combined with complexity ([hotspots](#hotspots)).

By default, [mechanical commits](#mechanical-commits) are excluded from churn.

---

## Complexity

**What it measures:** the structural intricacy of a file's current content.

**Definition — cyclomatic complexity** (for supported languages): `1 + number of branch nodes` in the AST, where branch nodes are `if`/`elif`, loops, `match`/`switch`/`select` arms, `except`/`catch`, and the ternary operator. The AST is produced by **tree‑sitter**, not regexes.

Supported languages for cyclomatic complexity:

- **Rust**, **Python**, **Go**, **JavaScript/TypeScript** (incl. TSX).

**Fallback:** for other languages, or when parsing fails, Chronograph uses an **indentation‑based** heuristic (sum of nesting depths over non‑empty lines, using a relative indentation stack so it's robust to tabs vs. spaces). This is a coarse but non‑zero signal.

`complexity_per_loc` normalizes complexity by lines of code.

**Content source:** complexity is computed from the **git blob** of the file's current state (addressed by `blob_sha`), not from the working directory. This makes it deterministic and historically exact, and lets Chronograph cache results by `(blob_sha, language)` — the same content is never parsed twice.

**Known limitations (documented, honest):**

- Logical `&&`/`||` are **not** counted in v1 (grammars don't expose them as distinct nodes).
- Macro bodies (e.g. Rust `macro_rules!`) are token trees, not control flow, so branches inside macros aren't counted.

---

## Hotspots

**What it measures:** the intersection of churn and complexity — the code you change constantly and understand least.

**Formula:**

```
score = churn_percentile × complexity_percentile
```

where each percentile is a **rank‑percentile in [0, 1]**: a file's position in the sorted order divided by `N − 1` (tie‑broken by path for determinism). The score is therefore also in `[0, 1]` and is exactly the product of two decomposable components — you can always see *why* a file ranks where it does (its churn, its complexity, and each percentile).

**Only ranked:** live files that have a **cyclomatic** complexity (the four supported languages). Files that fall back to the indentation heuristic (docs, configs, snapshots, unsupported languages) are **excluded** from the hotspot ranking.

> **Why exclude fallback files?** The indentation heuristic and cyclomatic complexity are different scales. On a full run, a large YAML or LICENSE file can score a huge indentation "complexity" and, combined with release‑noise churn, produce false hotspots. Excluding fallback files keeps the ranking meaningful (real code). A future improvement is an *allowlist* of programming‑language extensions so real code in currently‑unsupported languages (C++, Ruby, Java…) can be ranked too, while non‑code stays out.

**No magic number:** the score is never rescaled to 0–100. It's always the visible product of two visible percentiles.

---

## Change coupling

**What it measures:** which files tend to change **in the same commit** — a proxy for hidden logical/architectural dependency.

**Algorithm:** co‑occurrence over commits (a self‑join of the change table on commit SHA, taking pairs where `a.path < b.path`). Only pairs that actually co‑occurred are considered — cost is `O(Σ kᵢ²)` over commit sizes, **not** `O(files²)`.

**Formulas:**

```
support(A, B)       = number of commits that touched both A and B
coupling_ratio(A,B) = support(A, B) / min(commits(A), commits(B))
```

Because `support ≤ min(commits)`, the ratio is always in `(0, 1]`. A ratio near 1.0 means "almost every time one changed, so did the other".

**Symmetry:** `coupling(A, B) == coupling(B, A)` by construction (the pair is canonical `a < b`; support is a symmetric intersection; `min` is symmetric). This invariant is enforced by a property test.

**Thresholds:** pairs need `support ≥ min_support` (default **5**) to qualify — this filters coincidental one‑off co‑changes. Mechanical/giant commits are excluded by default (they both distort the signal and inflate the `O(kᵢ²)` cost).

**Not code‑only:** unlike hotspots, coupling is meaningful for configs and test snapshots too (e.g. a `*.rs` ↔ `*.stderr` UI‑test pair is a legitimate coupling).

**Why it correlates with problems:** two files that always change together but live in different modules signal a dependency your architecture doesn't make explicit — the kind of coupling that makes changes ripple unexpectedly.

---

## Knowledge / bus factor

**What it measures:** how concentrated the knowledge of a file is — the *risk* that it lives in too few heads.

**Definition:** from `git blame`, Chronograph computes each author's **ownership** (share of surviving lines) per file, then:

```
bus_factor        = the minimum number of top owners whose combined
                    ownership share is STRICTLY greater than the threshold
                    (default threshold = 0.5, i.e. "> 50% of the knowledge")
top_owner_ratio   = the single largest owner's share
```

A **bus factor of 1** means one person owns more than half the file — the classic concentration risk.

**Presented as risk, never as credit or blame.** Output is sorted by risk (lowest bus factor, then highest top‑owner share). **Authors are anonymized by default** (`Author #N`); real names require the explicit `--show-names` flag. This is a product requirement — surfacing "who wrote the most" would undermine adoption and isn't the point.

**Blame budget:** blaming a pathologically large, frequently‑rewritten file (think a multi‑megabyte generated `CHANGELOG` with hundreds of revisions) is indivisible and can take tens of minutes. Such files are skipped when their cost (`revisions × added lines`) exceeds the [blame budget](configuration.md#blame-budget) (default 10,000,000), and they are reported explicitly — never silently dropped.

---

## Code age

**What it measures:** the age of the surviving lines in a file — a stability signal complementary to churn.

**Definition:** from `git blame`, each surviving line gets an age in **days from the anchor** (`max(committed_at)`). Per file, Chronograph reports the distribution as percentiles:

| Field | Meaning |
|---|---|
| `newest_age_days` | youngest surviving line |
| `median_age_days` | median line age |
| `p90_age_days` | 90th‑percentile age |
| `oldest_age_days` | oldest surviving line |

Percentiles describe the distribution without needing an arbitrary "% older than X days" threshold.

**Interpretation:**

- **Small median age** → a zone of constant rewriting (often overlaps with hotspots).
- **Large median age** → stable, mature code that has settled.

---

## Mechanical commits

Some commits are "mechanical": bulk renames, formatting sweeps, vendored‑dependency drops, or release bumps that touch a huge number of files at once. Left in, they distort churn and coupling.

Chronograph flags a commit as mechanical when it touches more than **N** files, where **N** is set via `--mechanical-max-files`. **By default the heuristic is off** (no threshold), because a good default depends on the repository and the specification doesn't fix one — Chronograph won't invent a magic number. When enabled, churn and coupling exclude mechanical commits by default.

See [Configuration](configuration.md#mechanical-commits).
