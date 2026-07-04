// Вкладка Code age (§3.6): распределение возраста строк по файлам.
//
// Гистограмма файлов по median-возрасту — те же бакеты, что в статичном отчёте
// (5b), но живая: hover — счётчик/доля, клик по бакету — таблица его файлов с
// полными перцентилями (спред newest…oldest ловит «смешанный возраст» вроде
// build.rs). Ниже — карта возраста: FileTreemap, площадь = churn, цвет =
// median-возраст (свежее = горячее). Бакеты — презентационный выбор рендера,
// метрика хранит перцентили (решение 5b).

import { useMemo, useState } from "react";
import type { ChronographExport, FileAgeEntry } from "../types";
import FileTreemap, { type TreemapFile } from "./FileTreemap";

const LOW: [number, number, number] = [255, 245, 200];
const HIGH: [number, number, number] = [176, 0, 0];

const BUCKETS = [
  { label: "<1 mo", max: 30 },
  { label: "1–3 mo", max: 90 },
  { label: "3–12 mo", max: 365 },
  { label: "1–2 yr", max: 730 },
  { label: "2–5 yr", max: 1825 },
  { label: "5 yr+", max: Infinity },
];

function bucketOf(days: number): number {
  return BUCKETS.findIndex((b) => days <= b.max);
}

function lerp(t: number): string {
  const c = LOW.map((lo, i) => Math.round(lo + (HIGH[i] - lo) * t));
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}

/** Свежее — горячее: бакет 0 — насыщенный, последний — бледный. */
function bucketColor(i: number): string {
  return lerp(1 - i / (BUCKETS.length - 1));
}

const W = 1160;
const HIST_H = 210;
const ROWS_CAP = 300;

export default function AgeView({ data }: { data: ChronographExport }) {
  const [selected, setSelected] = useState<number | null>(null);

  const churnBy = useMemo(() => {
    const m = new Map<string, number>();
    for (const f of data.files) m.set(f.path, f.churn_total ?? 0);
    return m;
  }, [data]);

  const buckets = useMemo(() => {
    const groups: FileAgeEntry[][] = BUCKETS.map(() => []);
    for (const f of data.file_age) groups[bucketOf(f.median_age_days)].push(f);
    for (const g of groups) {
      g.sort((a, b) => a.median_age_days - b.median_age_days || (a.path < b.path ? -1 : 1));
    }
    return groups;
  }, [data]);

  const total = data.file_age.length;
  const maxCount = Math.max(1, ...buckets.map((g) => g.length));

  const maxMedian = useMemo(
    () => Math.max(1, ...data.file_age.map((f) => f.median_age_days)),
    [data],
  );
  const treemapFiles = useMemo<TreemapFile[]>(
    () =>
      data.file_age.map((f) => ({
        path: f.path,
        area: churnBy.get(f.path) ?? 0,
        color: f.median_age_days,
        tip: [
          `line age (days): newest ${f.newest_age_days} · median ${f.median_age_days} · p90 ${f.p90_age_days} · oldest ${f.oldest_age_days}`,
          `lines ${f.lines} · churn ${churnBy.get(f.path) ?? 0}`,
        ],
      })),
    [data, churnBy],
  );
  // Свежее = горячее; √ растягивает свежую зону (там и есть сигнал «переписывается»).
  const ageColorOf = (v: number) =>
    lerp(1 - Math.sqrt(Math.max(0, Math.min(1, v / maxMedian))));

  if (total === 0) {
    return (
      <div className="panel" style={{ padding: 40, textAlign: "center", color: "var(--muted)" }}>
        No age data in the export (blame skipped or stale cache).
      </div>
    );
  }

  const barW = W / BUCKETS.length;
  const shownRows = selected != null ? buckets[selected].slice(0, ROWS_CAP) : [];

  return (
    <div>
      <div className="panel treemap-live">
        <svg viewBox={`0 0 ${W} ${HIST_H}`} role="img">
          {buckets.map((g, i) => {
            const h = (g.length / maxCount) * (HIST_H - 58);
            const x = i * barW + 14;
            const y = HIST_H - 34 - h;
            const isSel = selected === i;
            return (
              <g
                key={BUCKETS[i].label}
                style={{ cursor: "pointer" }}
                onClick={() => setSelected(isSel ? null : i)}
              >
                {/* кликабельная область на всю колонку */}
                <rect x={i * barW} y={0} width={barW} height={HIST_H} fill="transparent" />
                <rect
                  x={x}
                  y={y}
                  width={barW - 28}
                  height={Math.max(h, 2)}
                  rx={3}
                  fill={bucketColor(i)}
                  stroke={isSel ? "#7a1f1f" : "#e6e1d8"}
                  strokeWidth={isSel ? 2.5 : 0.7}
                />
                <text
                  x={i * barW + barW / 2}
                  y={y - 7}
                  textAnchor="middle"
                  fontSize={13}
                  fontWeight={600}
                  fill="#22201c"
                >
                  {g.length}
                </text>
                <text
                  x={i * barW + barW / 2}
                  y={HIST_H - 14}
                  textAnchor="middle"
                  fontSize={12}
                  fill={isSel ? "#7a1f1f" : "#6b6459"}
                  fontWeight={isSel ? 700 : 400}
                >
                  {BUCKETS[i].label} · {total > 0 ? Math.round((g.length / total) * 100) : 0}%
                </text>
              </g>
            );
          })}
        </svg>
      </div>
      <div className="graph-legend">
        <span>files by median line age · click a bucket for its file list</span>
        <span style={{ marginLeft: "auto" }}>
          {total.toLocaleString("en-US")} files
        </span>
      </div>

      {selected != null && (
        <div className="panel tbl-wrap">
          <div className="tbl-head">
            <span>
              {BUCKETS[selected].label}: {buckets[selected].length.toLocaleString("en-US")} files
              {buckets[selected].length > ROWS_CAP && ` · showing first ${ROWS_CAP}`}
            </span>
            <button className="linkish" onClick={() => setSelected(null)}>
              close
            </button>
          </div>
          <table className="risk">
            <thead>
              <tr>
                <th>file</th>
                <th className="num">lines</th>
                <th className="num">newest (d)</th>
                <th className="num">median</th>
                <th className="num">p90</th>
                <th className="num">oldest</th>
              </tr>
            </thead>
            <tbody>
              {shownRows.map((f) => (
                <tr key={f.path}>
                  <td className="path">{f.path}</td>
                  <td className="num">{f.lines.toLocaleString("en-US")}</td>
                  <td className="num">{f.newest_age_days.toLocaleString("en-US")}</td>
                  <td className="num">{f.median_age_days.toLocaleString("en-US")}</td>
                  <td className="num">{f.p90_age_days.toLocaleString("en-US")}</td>
                  <td className="num">{f.oldest_age_days.toLocaleString("en-US")}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <h3 className="section-h">Age map</h3>
      <FileTreemap
        files={treemapFiles}
        colorOf={ageColorOf}
        areaLabel="churn"
        colorLabel="median age (d)"
      />
      <div className="graph-legend">
        <span>
          area — churn (√ scale, small ones get a minimum) · color — median age
          (√): <span className="grad" /> fresh → old (0 →{" "}
          {maxMedian.toLocaleString("en-US")} d)
        </span>
        <span style={{ marginLeft: "auto" }}>
          hot spots = zones of active rewriting
        </span>
      </div>
    </div>
  );
}
