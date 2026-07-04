// Переиспользуемый zoomable treemap по файлам (Hotspots / Knowledge / Code age).
//
// Показывает ОДИН уровень за раз: прямоугольники — дети текущей директории
// (файлы и поддиректории-агрегаты). Клик по директории — вглубь, хлебные
// крошки — назад. Это решает обе проблемы плоского treemap из статичного
// отчёта: тултип доступен на прямоугольнике любого размера, а мелочь
// «раскрывается» спуском на уровень ниже.
//
// Тайлинг — squarified (тот же алгоритм Брулса, что в server-side отчёте),
// вход отсортирован по площади — визуальный язык совпадает с report.html.
// Живой рендер: правило байт-идентичности не действует (CONTEXT.md).

import { useMemo, useRef, useState } from "react";
import { hierarchy, treemap, treemapSquarify, type HierarchyRectangularNode } from "d3-hierarchy";

export interface TreemapFile {
  path: string;
  /** Метрика площади (>0; файлы с 0 получают минимальную видимую площадь). */
  area: number;
  /** Непрерывное значение цветовой метрики. */
  color: number;
  /** Дополнительные строки тултипа (уже отформатированы вкладкой). */
  tip: string[];
}

interface Props {
  files: TreemapFile[];
  /** Готовая цветовая шкала значения → css-цвет. */
  colorOf: (v: number) => string;
  areaLabel: string;
  colorLabel: string;
  /**
   * Трансформация площади ДЛЯ РАСКЛАДКИ (default √): гасит скошенность метрик
   * (один файл-гигант не схлопывает остальных в невидимые полоски). Тултипы и
   * агрегаты директорий всегда показывают СЫРЫЕ значения.
   */
  areaTransform?: (v: number) => number;
}

/** Доля панели, меньше которой плитка не бывает (видимость мелочи). */
const MIN_TILE_SHARE = 0.004;

/** Узел собственного дерева путей с агрегатами для директорий. */
interface TNode {
  name: string;
  path: string;
  children: Map<string, TNode>;
  file?: TreemapFile;
  /** Σ сырой площади-метрики (для показа в тултипах). */
  sumRaw: number;
  /** Σ трансформированной площади (для раскладки). */
  sumLayout: number;
  /** Σ цвет×сырая-площадь (взвешенное среднее для директорий). */
  sumColorRaw: number;
  fileCount: number;
}

const W = 1160;
const H = 560;

function makeNode(name: string, path: string): TNode {
  return {
    name,
    path,
    children: new Map(),
    sumRaw: 0,
    sumLayout: 0,
    sumColorRaw: 0,
    fileCount: 0,
  };
}

function buildTree(files: TreemapFile[], transform: (v: number) => number): TNode {
  const root = makeNode("", "");
  for (const f of files) {
    const raw = Math.max(f.area, 0);
    const layout = Math.max(transform(raw), 1e-6);
    const segs = f.path.split("/");
    let node = root;
    node.sumRaw += raw;
    node.sumLayout += layout;
    node.sumColorRaw += f.color * raw;
    node.fileCount += 1;
    for (let i = 0; i < segs.length; i++) {
      const isLeaf = i === segs.length - 1;
      const key = segs[i];
      let child = node.children.get(key);
      if (!child) {
        child = makeNode(key, segs.slice(0, i + 1).join("/"));
        node.children.set(key, child);
      }
      child.sumRaw += raw;
      child.sumLayout += layout;
      child.sumColorRaw += f.color * raw;
      child.fileCount += 1;
      if (isLeaf) child.file = f;
      node = child;
    }
  }
  return root;
}

/** Найти узел по пути ('' — корень); сгибаем цепочки одиночных директорий. */
function nodeAt(root: TNode, path: string): TNode {
  if (path === "") return root;
  let node = root;
  for (const seg of path.split("/")) {
    const child = node.children.get(seg);
    if (!child) return root;
    node = child;
  }
  return node;
}

type CellDatum = { n?: TNode; children?: { n: TNode }[] };
type Cell = HierarchyRectangularNode<CellDatum>;

export default function FileTreemap({
  files,
  colorOf,
  areaLabel,
  colorLabel,
  areaTransform = Math.sqrt,
}: Props) {
  const [path, setPath] = useState("");
  const tipRef = useRef<HTMLDivElement>(null);

  const root = useMemo(() => buildTree(files, areaTransform), [files, areaTransform]);
  const current = nodeAt(root, path);

  const cells: Cell[] = useMemo(() => {
    const kids = [...current.children.values()];
    // Пол площади: ни одна плитка текущего уровня не меньше MIN_TILE_SHARE
    // панели — мелочь остаётся видимой и hover-доступной рядом с гигантом.
    const total = kids.reduce((s, k) => s + k.sumLayout, 0);
    const floor = total * MIN_TILE_SHARE;
    const shallow = hierarchy<CellDatum>({
      children: kids.map((n) => ({ n })),
    })
      .sum((d) => (d.n ? Math.max(d.n.sumLayout, floor) : 0))
      .sort(
        (a, b) =>
          (b.value ?? 0) - (a.value ?? 0) ||
          (a.data.n && b.data.n ? (a.data.n.path < b.data.n.path ? -1 : 1) : 0),
      );
    const laid = treemap<CellDatum>()
      .tile(treemapSquarify)
      .size([W, H])
      .paddingInner(3)
      .paddingOuter(2)(shallow);
    return laid.leaves().filter((c) => c.data.n);
  }, [current]);

  const crumbs = path === "" ? [] : path.split("/");

  const showTip = (e: React.MouseEvent, lines: string[]) => {
    const el = tipRef.current;
    if (!el) return;
    el.style.display = "block";
    el.innerHTML = lines.join("");
    moveTip(e);
  };
  const moveTip = (e: React.MouseEvent) => {
    const el = tipRef.current;
    if (!el) return;
    el.style.left = `${e.clientX + 14}px`;
    el.style.top = `${e.clientY + 12}px`;
  };
  const hideTip = () => {
    const el = tipRef.current;
    if (el) el.style.display = "none";
  };

  const fmt = (v: number) =>
    Math.abs(v) >= 100 ? Math.round(v).toLocaleString("en-US") : v.toFixed(1);

  return (
    <div>
      <div className="crumbs">
        <button className={path === "" ? "here" : ""} onClick={() => setPath("")}>
          whole project
        </button>
        {crumbs.map((seg, i) => {
          const p = crumbs.slice(0, i + 1).join("/");
          return (
            <span key={p}>
              {" / "}
              <button className={p === path ? "here" : ""} onClick={() => setPath(p)}>
                {seg}
              </button>
            </span>
          );
        })}
        <span className="dim">
          {" · "}
          {current.fileCount.toLocaleString("en-US")} files
        </span>
      </div>

      <div className="panel treemap-live">
        <svg viewBox={`0 0 ${W} ${H}`} role="img">
          {cells.map((c) => {
            const n = c.data.n!;
            const w = c.x1 - c.x0;
            const h = c.y1 - c.y0;
            const isDir = !n.file;
            const avgColor = n.sumRaw > 0 ? n.sumColorRaw / n.sumRaw : 0;
            const fill = colorOf(n.file ? n.file.color : avgColor);
            const label = isDir ? `${n.name}/` : n.name;
            const showLabel = w > 58 && h > 17;
            // Подпись читаемой на любом фоне: тёмная на светлом, светлая на тёмном.
            const rgb = fill.match(/\d+/g)?.map(Number) ?? [255, 255, 255];
            const lum = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2];
            const labelFill = lum < 140 ? "#fdfcfa" : "#22201c";
            const maxChars = Math.max(3, Math.floor(w / 6.8));
            const text =
              label.length > maxChars ? `${label.slice(0, maxChars - 1)}…` : label;
            const tipLines = n.file
              ? [
                  `<div class="path">${escapeHtml(n.file.path)}</div>`,
                  ...n.file.tip.map((t) => `<div class="dim">${t}</div>`),
                ]
              : [
                  `<div class="path">${escapeHtml(n.path)}/</div>`,
                  `<div class="dim">${n.fileCount} files · ${areaLabel} Σ ${fmt(
                    n.sumRaw,
                  )} · ${colorLabel} avg ${fmt(avgColor)}</div>`,
                  `<div class="dim">click — drill in</div>`,
                ];
            return (
              <g key={n.path}>
                <rect
                  x={c.x0}
                  y={c.y0}
                  width={w}
                  height={h}
                  fill={fill}
                  stroke={isDir ? "#a99e8d" : "#e6e1d8"}
                  strokeWidth={isDir ? 1.4 : 0.7}
                  rx={2}
                  style={{ cursor: isDir ? "pointer" : "default" }}
                  onClick={isDir ? () => { hideTip(); setPath(n.path); } : undefined}
                  onMouseEnter={(e) => showTip(e, tipLines)}
                  onMouseMove={moveTip}
                  onMouseLeave={hideTip}
                />
                {showLabel && (
                  <text
                    x={c.x0 + 5}
                    y={c.y0 + 13}
                    fontSize={11}
                    fontFamily='ui-monospace, "SF Mono", Menlo, Consolas, monospace'
                    fontWeight={isDir ? 600 : 400}
                    fill={labelFill}
                    pointerEvents="none"
                  >
                    {text}
                  </text>
                )}
              </g>
            );
          })}
        </svg>
      </div>

      <div ref={tipRef} className="tooltip" style={{ display: "none" }} />
    </div>
  );
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
