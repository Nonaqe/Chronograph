// Вкладка Knowledge: карта концентрации знаний (принцип 2.4 — риск, не оценка
// людей). Treemap: площадь = churn (операционный вес: активно меняющееся с
// bus factor 1 — живой риск; решение пользователя), цвет = bus factor
// (1 = тревожный красный). Авторы показываются РОВНО как в данных — движок
// анонимизирует экспорт по умолчанию («Author #N»), UI ничего не раскрывает.

import { useMemo, useState } from "react";
import type { ChronographExport, KnowledgeEntry } from "../types";
import FileTreemap, { type TreemapFile } from "./FileTreemap";

const LOW: [number, number, number] = [255, 245, 200];
const HIGH: [number, number, number] = [176, 0, 0];

/** bus factor → цвет: 1 = глубокий красный, 4+ = спокойный бледный. */
function bfColor(v: number): string {
  const t = 1 - Math.min(1, Math.max(0, (v - 1) / 3));
  const c = LOW.map((lo, i) => Math.round(lo + (HIGH[i] - lo) * t));
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}

type SortKey = "risk" | "path" | "bf" | "ratio" | "churn";

const ROWS_CAP = 300;

export default function KnowledgeView({ data }: { data: ChronographExport }) {
  const churnBy = useMemo(() => {
    const m = new Map<string, number>();
    for (const f of data.files) m.set(f.path, f.churn_total ?? 0);
    return m;
  }, [data]);

  // Экспорт уже отсортирован по риску (bus_factor ↑, top_owner_ratio ↓, путь).
  const riskIndex = useMemo(() => {
    const m = new Map<string, number>();
    data.knowledge.forEach((k, i) => m.set(k.path, i));
    return m;
  }, [data]);

  const files = useMemo<TreemapFile[]>(
    () =>
      data.knowledge.map((k) => {
        const churn = churnBy.get(k.path) ?? 0;
        return {
          path: k.path,
          area: churn,
          // Непрерывная цветовая метрика: bus factor, сглаженный долей
          // топ-владельца. На соло-репо (clap: 594/601 файлов с bf=1) чистый
          // bf красит ВСЁ в один красный — информации ноль; добавка
          // (1 − top_owner_ratio) различает «100% один человек» (темнее) и
          // «51% + второй рядом» (светлее). Монотонно по риску; сырые числа —
          // в тултипе и таблице.
          color: k.bus_factor + (1 - k.top_owner_ratio),
          tip: [
            `bus factor ${k.bus_factor} · top owner ${escapeHtml(k.top_owner)} (${Math.round(
              k.top_owner_ratio * 100,
            )}%)`,
            `churn ${churn}`,
          ],
        };
      }),
    [data, churnBy],
  );

  const [query, setQuery] = useState("");
  const [bf1Only, setBf1Only] = useState(false);
  const [sort, setSort] = useState<{ key: SortKey; asc: boolean }>({
    key: "risk",
    asc: true,
  });

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = data.knowledge.filter(
      (k) =>
        (!bf1Only || k.bus_factor === 1) &&
        (q === "" || k.path.toLowerCase().includes(q)),
    );
    const dir = sort.asc ? 1 : -1;
    const val = (k: KnowledgeEntry): number | string => {
      switch (sort.key) {
        case "risk":
          return riskIndex.get(k.path) ?? 0;
        case "path":
          return k.path;
        case "bf":
          return k.bus_factor;
        case "ratio":
          return k.top_owner_ratio;
        case "churn":
          return churnBy.get(k.path) ?? 0;
      }
    };
    return [...filtered].sort((a, b) => {
      const va = val(a);
      const vb = val(b);
      if (va < vb) return -dir;
      if (va > vb) return dir;
      return a.path < b.path ? -1 : 1;
    });
  }, [data, query, bf1Only, sort, riskIndex, churnBy]);

  const shown = rows.slice(0, ROWS_CAP);

  const th = (key: SortKey, label: string, num = false) => (
    <th
      className={(num ? "num " : "") + (sort.key === key ? "active" : "")}
      onClick={() =>
        setSort((s) => ({ key, asc: s.key === key ? !s.asc : key === "risk" || key === "path" }))
      }
    >
      {label}
      {sort.key === key ? (sort.asc ? " ↑" : " ↓") : ""}
    </th>
  );

  if (data.knowledge.length === 0) {
    return (
      <div className="panel" style={{ padding: 40, textAlign: "center", color: "var(--muted)" }}>
        No knowledge data in the export (blame skipped or stale cache).
      </div>
    );
  }

  return (
    <div>
      <FileTreemap
        files={files}
        colorOf={bfColor}
        areaLabel="churn"
        colorLabel="bus factor"
      />
      <div className="graph-legend">
        <span>
          area — churn (√ scale, small ones get a minimum) · color — bus factor,
          smoothed by top-owner share:{" "}
          {[1, 2, 3].map((b) => (
            <span key={b} className="bf" style={{ background: bfColor(b), color: b === 1 ? "#fff" : "#22201c", marginLeft: 6 }}>
              {b}
            </span>
          ))}
          <span className="bf" style={{ background: bfColor(4), color: "#22201c", marginLeft: 6 }}>
            4+
          </span>
        </span>
        <span style={{ marginLeft: "auto" }}>
          {data.knowledge.length.toLocaleString("en-US")} files · bus factor = 1
          for {data.knowledge.filter((k) => k.bus_factor === 1).length.toLocaleString("en-US")}
        </span>
      </div>

      <div className="panel tbl-wrap">
        <div className="tbl-head">
          <input
            type="text"
            placeholder="filter by path…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <label>
            <input
              type="checkbox"
              checked={bf1Only}
              onChange={(e) => setBf1Only(e.target.checked)}
            />
            bus factor = 1 only
          </label>
          <span style={{ marginLeft: "auto" }}>
            {rows.length.toLocaleString("en-US")} files
            {rows.length > ROWS_CAP && ` · showing first ${ROWS_CAP} — narrow the filter`}
          </span>
        </div>
        <table className="risk">
          <thead>
            <tr>
              {th("risk", "risk")}
              {th("path", "file")}
              {th("bf", "bus factor", true)}
              {th("ratio", "top-owner share", true)}
              <th>top owner</th>
              {th("churn", "churn", true)}
            </tr>
          </thead>
          <tbody>
            {shown.map((k, i) => (
              <tr key={k.path}>
                <td className="num">{(riskIndex.get(k.path) ?? i) + 1}</td>
                <td className="path">{k.path}</td>
                <td className="num">
                  <span
                    className="bf"
                    style={{
                      background: bfColor(k.bus_factor),
                      color: k.bus_factor === 1 ? "#fff" : "#22201c",
                    }}
                  >
                    {k.bus_factor}
                  </span>
                </td>
                <td className="num">{Math.round(k.top_owner_ratio * 100)}%</td>
                <td>{k.top_owner}</td>
                <td className="num">{(churnBy.get(k.path) ?? 0).toLocaleString("en-US")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
