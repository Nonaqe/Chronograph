# GitHub Action

[← Docs index](README.md) · [Русский](../ru/github-action.md)

The GitHub Action is Chronograph's primary distribution channel: add it to a workflow and every run produces a self‑contained HTML report as an artifact — no Rust toolchain, no build time.

It is a **composite action** that downloads a precompiled binary from a GitHub Release, **verifies its SHA‑256**, runs `chronograph report`, and (optionally) uploads the HTML.

## Minimal usage

```yaml
name: chronograph
on: [push]
jobs:
  report:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0            # REQUIRED: full history is needed
      - uses: Nonaqe/Chronograph/action@v0.1.0
        with:
          path: .
          output: chronograph-report.html
```

> **`fetch-depth: 0` is mandatory.** By default `actions/checkout` makes a shallow clone with a single commit; Chronograph needs the full history to compute anything meaningful.

The report is uploaded as an artifact named `chronograph-report`. Download it from **Actions → the run → Artifacts**.

## Inputs

| Input | Default | Description |
|---|---|---|
| `path` | `.` | Path to the git repository to analyze. |
| `output` | `chronograph-report.html` | Where to write the HTML report. |
| `version` | *(pinned to the Action's release)* | Explicitly pick the `chronograph` binary version. |
| `upload-artifact` | `true` | Whether to upload the report as an artifact. |

## Outputs

| Output | Description |
|---|---|
| `report` | Path to the generated HTML report. |

## Versioning

Pin the Action by tag (`@v0.1.0`) — it downloads **exactly that version** of the binary. The binary only changes when you change the pin, so a moving upstream never silently breaks your CI. `latest` is deliberately not used.

## Platforms

At launch, only **Linux x64** runners are supported. On macOS/Windows runners the Action fails with a clear, explicit error (and a non‑zero exit) rather than a silent misbehavior. Additional platforms are planned.

## How it works (and why)

Building the full engine (gix + bundled DuckDB + tree‑sitter grammars) in CI would take minutes on every run. Instead:

1. A separate **release workflow** cross‑compiles the Linux x64 binary on a `v*` tag, tarballs it, generates a `.sha256`, and publishes both to a GitHub Release.
2. The Action downloads that tarball + checksum, **verifies the SHA‑256 before executing** (standard hygiene for an action other people's pipelines trust), extracts, and runs `chronograph report`.

Because the download is an anonymous `curl` of a release asset, the **upstream repository must be public** — private release assets are not served to anonymous requests. This is a requirement of the distribution model, not an accident.

## Publishing to GitHub Pages (optional)

Turn the report into a browsable page:

```yaml
      - uses: actions/upload-pages-artifact@v3
        with:
          path: chronograph-report.html
      - uses: actions/deploy-pages@v4
```

## Security note

Third‑party actions in these examples are pinned by major tag for readability. In production, pin them by full commit SHA.
