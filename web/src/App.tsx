import { useCallback, useEffect, useRef, useState } from "react";
import type { ChronographExport } from "./types";
import { fetchExport, readFile } from "./load";
import CouplingGraph from "./views/CouplingGraph";
import HotspotsView from "./views/HotspotsView";
import KnowledgeView from "./views/KnowledgeView";
import AgeView from "./views/AgeView";
import TimelineView from "./views/TimelineView";
import GrowthView from "./views/GrowthView";

type Tab = "coupling" | "hotspots" | "knowledge" | "age" | "timeline" | "growth";

const TABS: { id: Tab; label: string; ready: boolean }[] = [
  { id: "coupling", label: "Change coupling", ready: true },
  { id: "hotspots", label: "Hotspots", ready: true },
  { id: "knowledge", label: "Knowledge", ready: true },
  { id: "age", label: "Code age", ready: true },
  { id: "timeline", label: "Timeline", ready: true },
  { id: "growth", label: "Repository growth", ready: true },
];

export default function App() {
  const [data, setData] = useState<ChronographExport | null>(null);
  const [tab, setTab] = useState<Tab>("coupling");
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);

  // ?src=<url> — автозагрузка при отдаче по http (dev-сервер, Pages).
  useEffect(() => {
    const src = new URLSearchParams(window.location.search).get("src");
    if (src) {
      fetchExport(src).then(setData, (e: Error) => setError(e.message));
    }
  }, []);

  const acceptFile = useCallback((file: File | undefined) => {
    if (!file) return;
    setError(null);
    readFile(file).then(setData, (e: Error) => setError(e.message));
  }, []);

  if (!data) {
    return (
      <div className="wrap">
        <header className="app">
          <h1>Chronograph</h1>
          <div className="sub">git repository evolution analytics</div>
        </header>
        <div
          className={`dropzone${dragOver ? " over" : ""}`}
          onDragOver={(e) => {
            e.preventDefault();
            setDragOver(true);
          }}
          onDragLeave={() => setDragOver(false)}
          onDrop={(e) => {
            e.preventDefault();
            setDragOver(false);
            acceptFile(e.dataTransfer.files[0]);
          }}
        >
          <h2>Drop chronograph.json here</h2>
          <p>
            The file is produced by <code>chronograph export</code> in the repository root.
          </p>
          <button className="pick" onClick={() => fileInput.current?.click()}>
            Choose file
          </button>
          <input
            ref={fileInput}
            type="file"
            accept=".json,application/json"
            hidden
            onChange={(e) => acceptFile(e.target.files?.[0] ?? undefined)}
          />
          {error && <div className="error">{error}</div>}
        </div>
      </div>
    );
  }

  const m = data.meta;
  return (
    <div className="wrap">
      <header className="app">
        <h1>Chronograph</h1>
        <div className="sub">
          <code>{m.head_sha.slice(0, 12)}</code> · {m.total_commits.toLocaleString("en-US")}{" "}
          commits · {data.files.length.toLocaleString("en-US")} files ·{" "}
          {m.total_authors.toLocaleString("en-US")} authors
          {m.anonymized && <span className="badge">authors anonymized</span>}
        </div>
      </header>

      <nav className="tabs">
        {TABS.map((t) => (
          <button
            key={t.id}
            className={tab === t.id ? "active" : ""}
            disabled={!t.ready}
            title={t.ready ? undefined : "coming soon"}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </nav>

      {tab === "coupling" && <CouplingGraph data={data} />}
      {tab === "hotspots" && <HotspotsView data={data} />}
      {tab === "knowledge" && <KnowledgeView data={data} />}
      {tab === "age" && <AgeView data={data} />}
      {tab === "timeline" && <TimelineView data={data} />}
      {tab === "growth" && <GrowthView data={data} />}
    </div>
  );
}
