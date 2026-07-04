// Вкладка Timeline: scrubber по времени. График роста репозитория (живых
// файлов) + активность коммитов; перетаскиваемый ползунок; на позиции t —
// снимок дерева файлов (FileTreemap, площадь=churn, цвет=свежесть касания) +
// сводка коммита. Делит движок воспроизведения (timeline.ts) с вкладкой Рост.
// Живой рендер — вне правила байт-идентичности (CONTEXT.md).

import { useMemo, useRef, useState } from "react";
import type { ChronographExport } from "../types";
import { Timeline, fmtDate } from "../timeline";
import FileTreemap, { type TreemapFile } from "./FileTreemap";

const W = 1160;
const CHART_H = 200;
const PAD = { l: 6, r: 6, t: 14, b: 24 };

const LOW: [number, number, number] = [255, 245, 200];
const HIGH: [number, number, number] = [176, 0, 0];

export default function TimelineView({ data }: { data: ChronographExport }) {
  const tl = useMemo(() => new Timeline(data), [data]);
  const [idx, setIdx] = useState(() => tl.events.length - 1);
  const svgRef = useRef<SVGSVGElement>(null);

  const n = tl.events.length;
  if (n === 0) {
    return (
      <div className="panel" style={{ padding: 40, textAlign: "center", color: "var(--muted)" }}>
        No event stream in the export.
      </div>
    );
  }

  const maxAlive = Math.max(1, ...tl.points.map((p) => p.aliveFiles));
  const maxTouched = Math.max(1, ...tl.points.map((p) => p.touched));
  const innerW = W - PAD.l - PAD.r;
  const innerH = CHART_H - PAD.t - PAD.b;
  const xOf = (i: number) => PAD.l + (n <= 1 ? 0 : (i / (n - 1)) * innerW);

  // Площадь роста (число живых файлов) — накопительная форма истории.
  const areaPath = useMemo(() => {
    const pts = tl.points.map(
      (p, i) => `${xOf(i).toFixed(1)},${(PAD.t + innerH - (p.aliveFiles / maxAlive) * innerH).toFixed(1)}`,
    );
    return `M ${PAD.l},${PAD.t + innerH} L ${pts.join(" L ")} L ${PAD.l + innerW},${PAD.t + innerH} Z`;
  }, [tl, maxAlive, innerH, innerW]);

  const frame = useMemo(() => tl.frameAt(idx), [tl, idx]);
  const cur = tl.points[Math.max(0, idx)];

  // Снимок дерева: площадь = накопленный churn, цвет = свежесть касания
  // (насколько недавно относительно текущего момента idx).
  const files = useMemo<TreemapFile[]>(() => {
    const out: TreemapFile[] = [];
    for (const f of frame.files.values()) {
      const recency = idx <= 0 ? 1 : 1 - (idx - f.lastTouch) / (idx + 1);
      out.push({
        path: f.path,
        area: f.churn,
        color: Math.max(0, Math.min(1, recency)),
        tip: [
          `churn so far ${f.churn}`,
          `last touched: ${fmtDate(tl.events[f.lastTouch].ts)}`,
        ],
      });
    }
    return out;
  }, [frame, idx, tl]);

  const colorOf = (v: number) => {
    const c = LOW.map((lo, i) => Math.round(lo + (HIGH[i] - lo) * v));
    return `rgb(${c[0]},${c[1]},${c[2]})`;
  };

  const seekFromClientX = (clientX: number) => {
    const svg = svgRef.current;
    if (!svg) return;
    const rect = svg.getBoundingClientRect();
    const rel = (clientX - rect.left) / rect.width; // 0..1 в координатах viewBox
    const px = rel * W;
    const t = (px - PAD.l) / innerW;
    setIdx(Math.max(0, Math.min(n - 1, Math.round(t * (n - 1)))));
  };

  const dragging = useRef(false);

  return (
    <div>
      <div className="panel controls" style={{ alignItems: "baseline" }}>
        <label style={{ flex: 1 }}>
          <input
            type="range"
            min={0}
            max={n - 1}
            value={idx}
            style={{ width: "100%" }}
            onChange={(e) => setIdx(Number(e.target.value))}
          />
        </label>
        <span className="stats" style={{ marginLeft: 0 }}>
          {fmtDate(cur.ts)} · commit {idx + 1} of {n} · {frame.files.size.toLocaleString("en-US")}{" "}
          live files
        </span>
      </div>

      <div className="panel treemap-live">
        <svg
          ref={svgRef}
          viewBox={`0 0 ${W} ${CHART_H}`}
          role="img"
          style={{ cursor: "ew-resize" }}
          onMouseDown={(e) => {
            dragging.current = true;
            seekFromClientX(e.clientX);
          }}
          onMouseMove={(e) => dragging.current && seekFromClientX(e.clientX)}
          onMouseUp={() => (dragging.current = false)}
          onMouseLeave={() => (dragging.current = false)}
        >
          {/* активность коммитов — тонкие штрихи снизу */}
          <g>
            {tl.points.map((p, i) =>
              i % Math.ceil(n / 600) === 0 ? (
                <line
                  key={i}
                  x1={xOf(i)}
                  x2={xOf(i)}
                  y1={PAD.t + innerH}
                  y2={PAD.t + innerH - (p.touched / maxTouched) * (innerH * 0.5)}
                  stroke="#d8cfc0"
                  strokeWidth={0.8}
                />
              ) : null,
            )}
          </g>
          <path d={areaPath} fill="#e7d6c6" stroke="#a04a2a" strokeWidth={1} />
          {/* ползунок */}
          <line
            x1={xOf(idx)}
            x2={xOf(idx)}
            y1={PAD.t - 6}
            y2={PAD.t + innerH}
            stroke="#7a1f1f"
            strokeWidth={2}
          />
          <circle cx={xOf(idx)} cy={PAD.t - 6} r={4} fill="#7a1f1f" />
          {/* подписи оси времени */}
          <text x={PAD.l} y={CHART_H - 6} fontSize={11} fill="#6b6459">
            {fmtDate(tl.minTs)}
          </text>
          <text x={W - PAD.r} y={CHART_H - 6} fontSize={11} fill="#6b6459" textAnchor="end">
            {fmtDate(tl.maxTs)}
          </text>
          <text
            x={W / 2}
            y={CHART_H - 6}
            fontSize={11}
            fill="#6b6459"
            textAnchor="middle"
          >
            growth: live files up to {maxAlive.toLocaleString("en-US")}
          </text>
        </svg>
      </div>
      <div className="graph-legend">
        <span>
          drag the handle or scrub across the chart · the tree snapshot below is
          at the selected moment
        </span>
        <span style={{ marginLeft: "auto" }}>
          commit {cur.sha.slice(0, 10)} · {cur.author} · touched {cur.touched}{" "}
          files
        </span>
      </div>

      <h3 className="section-h">Tree at the selected moment</h3>
      {files.length === 0 ? (
        <div className="panel" style={{ padding: 40, textAlign: "center", color: "var(--muted)" }}>
          No live files at this moment yet.
        </div>
      ) : (
        <FileTreemap
          files={files}
          colorOf={colorOf}
          areaLabel="churn"
          colorLabel="freshness"
        />
      )}
      <div className="graph-legend">
        <span>
          area — churn so far (√) · color — touch freshness:{" "}
          <span className="grad" /> long ago → just now
        </span>
      </div>
    </div>
  );
}
