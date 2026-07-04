// Вкладка Hotspots: интерактивный treemap (площадь = complexity, цвет = churn —
// та же семантика, что в server-side отчёте; принцип 2.6: раскрываемо до
// составляющих в тултипе). Участвуют только живые файлы с cyclomatic
// complexity — как в hotspot-ранжировании движка (решение Этапа 1).

import { useMemo } from "react";
import type { ChronographExport } from "../types";
import FileTreemap, { type TreemapFile } from "./FileTreemap";

// Палитра churn — как легенда отчёта: бледно-жёлтый → глубокий красный.
const LOW: [number, number, number] = [255, 245, 200];
const HIGH: [number, number, number] = [176, 0, 0];

export default function HotspotsView({ data }: { data: ChronographExport }) {
  const { files, maxChurn } = useMemo(() => {
    const files: TreemapFile[] = [];
    let maxChurn = 1;
    for (const f of data.files) {
      // Только участники hotspot-рейтинга = живые файлы с CYCLOMATIC complexity.
      // Фильтр по `complexity != null` был бы неверен: в колонке есть и
      // indentation-fallback (Cargo.lock/yaml/md), смешивать шкалы нельзя —
      // ровно проблема LICENSE/ci.yml, решённая движком на Этапе 1.
      if (f.hotspot_rank == null || f.complexity == null) continue;
      const churn = f.churn_total ?? 0;
      maxChurn = Math.max(maxChurn, churn);
      files.push({
        path: f.path,
        area: f.complexity,
        color: churn,
        tip: [
          `churn ${churn} · complexity ${f.complexity.toFixed(1)}` +
            (f.complexity_per_loc != null
              ? ` · per line ${f.complexity_per_loc.toFixed(3)}`
              : ""),
          `hotspot #${f.hotspot_rank}`,
          `churn windows: 30d ${f.churn_30d ?? 0} · 90d ${f.churn_90d ?? 0} · 365d ${f.churn_365d ?? 0}`,
        ],
      });
    }
    return { files, maxChurn };
  }, [data]);

  // √-шкала: churn сильно скошен (единичные файлы с churn в тысячи при
  // типичных десятках) — линейная шкала красит всё в бледное. Указано в легенде.
  const colorOf = (v: number) => {
    const t = Math.sqrt(Math.max(0, Math.min(1, v / maxChurn)));
    const c = LOW.map((lo, i) => Math.round(lo + (HIGH[i] - lo) * t));
    return `rgb(${c[0]},${c[1]},${c[2]})`;
  };

  if (files.length === 0) {
    return (
      <div className="panel" style={{ padding: 40, textAlign: "center", color: "var(--muted)" }}>
        No live files with cyclomatic complexity — hotspots are computed only for
        supported languages (Rust/Python/Go/JS/TS).
      </div>
    );
  }

  return (
    <div>
      <FileTreemap
        files={files}
        colorOf={colorOf}
        areaLabel="complexity"
        colorLabel="churn"
      />
      <div className="graph-legend">
        <span>
          area — complexity (√ scale, small ones get a guaranteed minimum) ·
          color — churn (√):{" "}
          <span className="grad" />
          {" "}0 → {maxChurn.toLocaleString("en-US")}
        </span>
        <span style={{ marginLeft: "auto" }}>
          {files.length.toLocaleString("en-US")} files · click a directory to
          drill in · hover for details
        </span>
      </div>
    </div>
  );
}
