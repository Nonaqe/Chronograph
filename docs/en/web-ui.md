# Web UI

[← Docs index](README.md) · [Русский](../ru/web-ui.md)

The web UI is an optional interactive client built with **React + TypeScript + Vite**, with visualizations hand‑built on **D3**. All dependencies are bundled — zero CDN. It reads a `chronograph.json` produced by [`chronograph export`](cli.md#export).

## Running it

```bash
cd web
npm install
npm run dev        # dev server at http://localhost:5173
npm run build      # static bundle in web/dist/
```

## Loading data

Produce an export, then feed it to the UI:

```bash
chronograph export /path/to/repo --out chronograph.json
```

- **Drag & drop** the `chronograph.json` file onto the drop zone (or click **Choose file**).
- When served over HTTP you can auto‑load via `?src=<url>` — e.g. `http://localhost:5173/?src=/data/ripgrep.json` (place the file under `web/public/data/`, which is git‑ignored).

<div align="center">
<img src="../assets/landing.png" alt="Landing / drop zone" width="80%">
<br><em>The landing screen: drop a <code>chronograph.json</code> to begin.</em>
</div>

Once loaded, a header shows the head SHA, commit/file/author counts, and an **"authors anonymized"** badge when names are hidden.

## The six tabs

### 1. Change coupling

A D3 force graph of files that change together. Node size = churn, dark outline = bus factor 1, edge width = support, edge brightness = coupling strength. Nodes are colored by top‑level directory and grouped into module bundles. A rich filter panel ("risk map") lets you threshold by ratio and support, show the top‑N pairs, switch module levels, and toggle overlays (hotspot top‑100, bus factor = 1, changed this year, code only, cross‑module only).

<div align="center">
<img src="../assets/coupling.png" alt="Change coupling force graph" width="90%">
</div>

### 2. Hotspots

A zoomable treemap (d3‑hierarchy). Area = complexity (√‑scaled so small files stay visible), color = churn. Click a directory to drill in; a breadcrumb tracks your depth; hover for per‑cell detail.

<div align="center">
<img src="../assets/hotspots.png" alt="Hotspots treemap" width="90%">
<br><em>Drilled into <code>crates/</code>: <code>core/</code> is the reddest (highest churn).</em>
</div>

### 3. Knowledge

A treemap where area = churn and color = bus factor (smoothed by top‑owner share), plus a sortable risk table (bus factor, top‑owner share, top owner, churn). Filter by path, or show only bus‑factor‑1 files.

<div align="center">
<img src="../assets/knowledge.png" alt="Knowledge map" width="90%">
</div>

### 4. Code age

A histogram of files bucketed by median line age (`<1 mo`, `1–3 mo`, `3–12 mo`, `1–2 yr`, `2–5 yr`, `5 yr+`) plus an "age map" treemap. Click a bucket to see its file list.

<div align="center">
<img src="../assets/age.png" alt="Code age" width="90%">
</div>

### 5. Timeline

A scrubber over the whole history with a live‑files growth chart. Drag the handle to any moment and the treemap below shows the file tree **as it was** at that commit.

<div align="center">
<img src="../assets/timeline.png" alt="Timeline" width="90%">
</div>

### 6. Repository growth

The showcase feature (and, by design, the *last* one built): a Gource‑style animation of "growing roots". The file/directory tree unfolds radially over time — branches thicken with the number of files in a subtree, file "buds" bloom and flash when a commit touches them. Colors encode top‑level directories; **authors are not shown** (principle: files, not people). Play, pause, reset, adjust speed, or scrub.

<div align="center">
<img src="../assets/growth.png" alt="Repository growth animation" width="90%">
</div>

## Principles

- **Determinism does not apply to the UI.** The byte‑identity guarantee is about the `chronograph.json` artifact; the live rendering uses physics and time, so it is not reproducible pixel‑for‑pixel (by design).
- **Controls are presentation filters, not metric thresholds.** Sliders for ratio/support/top‑N change what's *shown*; the underlying numbers are already computed by the engine.
- **Anonymization is honored.** The engine anonymizes authors by default; the UI simply shows what's in the file.
