# Installation

[← Docs index](README.md) · [Русский](../ru/installation.md)

Chronograph is a Rust workspace. There is no published binary release channel for local use yet, so the supported path is **building from source**. (The GitHub Action uses a prebuilt binary attached to a GitHub Release — see [GitHub Action](github-action.md).)

## Requirements

- **Rust** (stable) with `cargo` — install via [rustup](https://rustup.rs/).
- A **C/C++ toolchain**, required to compile the bundled DuckDB from source:
  - Linux: `gcc`/`clang` + `make`.
  - macOS: Xcode command‑line tools (`xcode-select --install`).
  - Windows: the MSVC build tools (Visual Studio Build Tools).
- **git** is *not* required at runtime — Chronograph reads repositories directly through `gix` (gitoxide). Git is only used by the test fixtures.
- For the web UI: **Node.js** (18+) and npm.

## Build

```bash
git clone https://github.com/Nonaqe/Chronograph.git
cd Chronograph
cargo build --release
```

The binary is produced at:

```
target/release/chronograph        # (chronograph.exe on Windows)
```

> **First build is slow.** Chronograph statically links a bundled DuckDB (compiled from source), tree‑sitter grammars, and gix. The first `cargo build` can take several minutes; subsequent builds reuse the cache. `cargo clean` throws that cache away — avoid it unless necessary.

Add the binary to your `PATH`, or invoke it by full path (`./target/release/chronograph`).

## Verify

```bash
chronograph --version
chronograph --help
```

You should see the list of subcommands: `analyze`, `hotspots`, `coupling`, `knowledge`, `age`, `report`, `export`.

## First run

Point Chronograph at any git repository:

```bash
chronograph hotspots /path/to/some/repo
```

On the first run this:

1. Opens the repository with `gix`.
2. Walks the full history, extracting commits, diffs and renames.
3. Writes a cache to `<repo>/.chronograph/cache.duckdb`.
4. Computes and prints the result.

On later runs, only **new commits** are processed (the last `head_sha` is remembered) — so repeated commands are fast.

## The analysis cache

Chronograph stores its analysis in a local DuckDB file:

```
<repo>/.chronograph/cache.duckdb
```

- It is safe to delete — it is rebuilt on the next run.
- You can relocate it with `--db <FILE>` on any command.
- Add `.chronograph/` to your `.gitignore` (Chronograph never commits it for you).

If you ever change the engine version and see schema errors, delete the cache and re‑run.

## Performance note

Always use the **release** binary for real repositories. Debug builds are 5–20× slower on the CPU‑bound paths (complexity parsing, blame). See [Configuration → blame budget](configuration.md#blame-budget) for controlling cost on pathologically large files.

## Running the web UI

```bash
cd web
npm install
npm run dev        # dev server at http://localhost:5173
npm run build      # static bundle in web/dist/
```

The UI needs a `chronograph.json` produced by [`chronograph export`](cli.md#export). See the [Web UI guide](web-ui.md).
