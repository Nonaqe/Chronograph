// Вкладка «Рост репозитория»: Gource-style анимация «прорастающих корней» на
// canvas (ТЗ §5, СТРОГО ПОСЛЕДНЯЯ фича — витрина проекта).
//
// Дерево ФАЙЛОВ И ДИРЕКТОРИЙ раскладывается радиально (d3-tree, файлы — ЛИСТЬЯ:
// каждый получает свой угловой сектор → разведены по кругу, не друг на друге).
// Ветви прорастают во времени: ребро parent→child «вытягивается», когда под
// узлом впервые появляется файл; толщина ветви ~ числу файлов в поддереве
// (толстый ствол → тонкие корешки). Файлы распускаются «почками» и вспыхивают
// при касании коммитом. Тёмный фон + мягкое свечение — эстетика живых корней.
//
// Живой рендер (физика/время/сиды) — правило байт-идентичности НЕ действует
// (CONTEXT.md). Данные — полный поток событий без семплирования; плеер
// инкрементальный (события применяются по мере хода времени) + canvas.

import { useEffect, useMemo, useRef, useState } from "react";
import { hierarchy, tree as d3tree, type HierarchyNode } from "d3-hierarchy";
import type { ChronographExport } from "../types";
import { Timeline, fmtDate } from "../timeline";

const W = 1200;
const H = 760;
const BG = "#14110d";

// Яркая палитра директорий — читаема на тёмном (не приглушённая «бумажная»).
const DIR_PALETTE = [
  "#e0563f", "#4d9fe0", "#6fce5a", "#e8c24a", "#b46fd6", "#3fd6c0",
  "#f0864a", "#9aa0c0", "#e05a9a", "#8fce3f", "#5aa8f0", "#e0a15a",
];

const GROW_DAYS = 26; // за сколько дней истории ветвь дорастает
const BLOOM_DAYS = 12; // за сколько дней «распускается» файл-почка
const FLASH_DAYS = 34; // спад вспышки касания

interface LNode {
  path: string;
  isFile: boolean;
  x: number;
  y: number;
  px: number; // позиция родителя
  py: number;
  bornTs: number;
  width: number; // толщина входящей ветви
  color: string; // цвет верхней директории
  wob: number; // детерминированный изгиб ветви
  hasParent: boolean;
}

function topDir(path: string): string {
  const i = path.indexOf("/");
  return i === -1 ? path : path.slice(0, i);
}

function hashWob(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return (((h >>> 0) % 1000) / 1000 - 0.5) * 2; // [-1,1]
}

interface Layout {
  nodes: LNode[];
  byPath: Map<string, LNode>;
}

/** Раскладка: файлы — листья, директории — узлы ветвления. */
function buildLayout(tl: Timeline): Layout {
  // Союз всех путей истории + время первого появления каждого.
  const firstTs = new Map<string, number>();
  for (const ev of tl.events) {
    for (const ch of ev.changes) {
      if (ch.type === "D") continue;
      if (!firstTs.has(ch.path)) firstTs.set(ch.path, ev.ts);
    }
  }

  interface RN {
    key: string;
    path: string;
    isFile: boolean;
    ts: number;
    children: Map<string, RN>;
  }
  const root: RN = { key: "", path: "", isFile: false, ts: tl.minTs, children: new Map() };
  for (const [path, ts] of firstTs) {
    const segs = path.split("/");
    let node = root;
    for (let i = 0; i < segs.length; i++) {
      const isFile = i === segs.length - 1;
      const key = segs[i];
      let ch = node.children.get(key);
      if (!ch) {
        ch = {
          key,
          path: segs.slice(0, i + 1).join("/"),
          isFile,
          ts: Infinity,
          children: new Map(),
        };
        node.children.set(key, ch);
      }
      if (isFile) ch.ts = ts;
      node = ch;
    }
  }

  interface HD {
    rn: RN;
    children?: HD[];
  }
  const toH = (n: RN): HD => ({
    rn: n,
    children: n.children.size ? [...n.children.values()].map(toH) : undefined,
  });

  const h: HierarchyNode<HD> = hierarchy<HD>(toH(root));
  h.sum((d) => (d.rn.isFile ? 1 : 0)); // value = число файлов в поддереве
  // bornTs = min ts листьев поддерева.
  h.eachAfter((d) => {
    if (d.data.rn.isFile) {
      d.data.rn.ts = d.data.rn.ts;
    } else {
      let m = Infinity;
      for (const c of d.children ?? []) m = Math.min(m, c.data.rn.ts);
      d.data.rn.ts = m === Infinity ? tl.minTs : m;
    }
  });

  // Нисходящее «корневое» дерево: семя сверху по центру, корни растут ВНИЗ.
  // tidy-раскладка (d3.tree) сама разводит листья по горизонтали без наложений
  // и отдаёт ширину поддереву по числу листьев (крупные модули — шире).
  // Неравномерность «как у настоящих корней» добавляем детерминированным
  // разбросом: угол ответвления и длина каждой ветви пляшут по хэшу пути.
  const MARGIN_X = 40;
  const TOP = 42;
  const innerW = W - MARGIN_X * 2;
  const innerH = H - TOP - 30;
  const maxDepth = Math.max(1, (h.height ?? 1));
  const rowH = innerH / maxDepth;
  d3tree<HD>()
    .size([innerW, innerH])
    .separation((a, b) => (a.parent === b.parent ? 1 : 1.7))(h);

  // Позиции с органическим разбросом; считаем один раз, детей связываем с
  // родителем по сохранённой (сдвинутой) позиции — ветви остаются непрерывными.
  const pos = new Map<string, { x: number; y: number }>();
  const seed = { x: W / 2, y: TOP };
  h.each((d) => {
    if (d.data.rn.path === "") {
      pos.set("", seed);
      return;
    }
    const jx = hashWob(d.data.rn.path) * rowH * 0.32; // боковой разброс ветви
    const jy = hashWob(d.data.rn.path + "#y") * rowH * 0.42; // разброс длины
    pos.set(d.data.rn.path, {
      x: MARGIN_X + (d.x ?? 0) + jx,
      y: TOP + (d.y ?? 0) + Math.abs(jy) * 0.6 + jy * 0.4,
    });
  });

  const maxLeaf = Math.max(1, h.value ?? 1);
  const nodes: LNode[] = [];
  const byPath = new Map<string, LNode>();

  h.each((d) => {
    const path = d.data.rn.path;
    if (path === "") return;
    const p = pos.get(path)!;
    const parentPath = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
    const pp = pos.get(parentPath) ?? seed;
    const leaf = d.value ?? 1;
    const ln: LNode = {
      path,
      isFile: d.data.rn.isFile,
      x: p.x,
      y: p.y,
      px: pp.x,
      py: pp.y,
      bornTs: d.data.rn.ts,
      width: 0.7 + 6.5 * Math.sqrt(leaf / maxLeaf),
      color: DIR_PALETTE[Math.abs(hashInt(topDir(path))) % DIR_PALETTE.length],
      wob: hashWob(path),
      hasParent: true,
    };
    nodes.push(ln);
    byPath.set(ln.path, ln);
  });

  return { nodes, byPath };
}

function hashInt(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h | 0;
}

const SPEED_DAYS = [3, 7, 30, 90, 180];

export default function GrowthView({ data }: { data: ChronographExport }) {
  const tl = useMemo(() => new Timeline(data), [data]);
  const layout = useMemo(() => buildLayout(tl), [tl]);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [playing, setPlaying] = useState(false);
  const [speedI, setSpeedI] = useState(2);
  const [uiTs, setUiTs] = useState(tl.minTs);

  const st = useRef({
    animTs: tl.minTs,
    ptr: 0,
    alive: new Map<string, number>(), // path -> ts последнего касания
    lastReal: 0,
    speedI: 2,
    playing: false,
  });

  const n = tl.events.length;

  const applyEvent = (i: number, s: typeof st.current) => {
    const ev = tl.events[i];
    for (const ch of ev.changes) {
      if (ch.type === "D") s.alive.delete(ch.path);
      else {
        if (ch.type === "R" && ch.old_path) s.alive.delete(ch.old_path);
        s.alive.set(ch.path, ev.ts);
      }
    }
  };

  const seekTo = (targetTs: number) => {
    const s = st.current;
    s.alive.clear();
    s.ptr = 0;
    s.animTs = targetTs;
    while (s.ptr < n && tl.events[s.ptr].ts <= targetTs) {
      applyEvent(s.ptr, s);
      s.ptr += 1;
    }
    setUiTs(targetTs);
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d")!;
    const dpr = Math.min(1.5, window.devicePixelRatio || 1);
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    ctx.scale(dpr, dpr);

    let raf = 0;
    const render = (now: number) => {
      const s = st.current;
      if (s.playing) {
        const dtReal = s.lastReal ? (now - s.lastReal) / 1000 : 0;
        s.lastReal = now;
        s.animTs += dtReal * SPEED_DAYS[s.speedI] * 86400;
        if (s.animTs >= tl.maxTs) {
          s.animTs = tl.maxTs;
          s.playing = false;
          setPlaying(false);
        }
        while (s.ptr < n && tl.events[s.ptr].ts <= s.animTs) {
          applyEvent(s.ptr, s);
          s.ptr += 1;
        }
        setUiTs(s.animTs);
      } else {
        s.lastReal = 0;
      }
      draw(ctx, s.animTs, s.alive, layout);
      raf = requestAnimationFrame(render);
    };
    raf = requestAnimationFrame(render);
    return () => cancelAnimationFrame(raf);
  }, [tl, layout, n]);

  useEffect(() => {
    st.current.speedI = speedI;
  }, [speedI]);
  useEffect(() => {
    st.current.playing = playing;
    if (playing) st.current.lastReal = 0;
  }, [playing]);

  if (n === 0) {
    return (
      <div className="panel" style={{ padding: 40, textAlign: "center", color: "var(--muted)" }}>
        No event stream in the export.
      </div>
    );
  }

  const aliveCount = st.current.alive.size;

  return (
    <div>
      <div className="panel controls" style={{ alignItems: "center" }}>
        <button
          className="pick"
          style={{ marginTop: 0, padding: "6px 16px" }}
          onClick={() => {
            if (st.current.animTs >= tl.maxTs) seekTo(tl.minTs);
            setPlaying((p) => !p);
          }}
        >
          {playing ? "⏸ pause" : "▶ play"}
        </button>
        <button className="linkish" onClick={() => { setPlaying(false); seekTo(tl.minTs); }}>
          ⟲ reset
        </button>
        <label>
          speed
          <select value={speedI} onChange={(e) => setSpeedI(Number(e.target.value))}>
            {SPEED_DAYS.map((d, i) => (
              <option key={d} value={i}>
                {d} d/s
              </option>
            ))}
          </select>
        </label>
        <label style={{ flex: 1 }}>
          <input
            type="range"
            min={tl.minTs}
            max={tl.maxTs}
            value={Math.round(uiTs)}
            style={{ width: "100%" }}
            onChange={(e) => { setPlaying(false); seekTo(Number(e.target.value)); }}
          />
        </label>
        <span className="stats" style={{ marginLeft: 0 }}>
          {fmtDate(uiTs)} · {aliveCount.toLocaleString("en-US")} files
        </span>
      </div>

      <div className="panel" style={{ padding: 0, overflow: "hidden", borderRadius: 8, background: BG }}>
        <canvas
          ref={canvasRef}
          style={{ width: "100%", height: "auto", display: "block" }}
        />
      </div>
      <div className="graph-legend">
        <span>
          branches grow over time (thickness — files in the subtree) ·
          file buds bloom and flash when a commit touches them
        </span>
        <span style={{ marginLeft: "auto" }}>
          color — top-level directory · authors not shown (principle 2.4)
        </span>
      </div>
    </div>
  );
}

const DAY = 86400;

function draw(
  ctx: CanvasRenderingContext2D,
  animTs: number,
  alive: Map<string, number>,
  layout: Layout,
) {
  ctx.fillStyle = BG;
  ctx.fillRect(0, 0, W, H);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  // --- Ветви: рисуем от толстых к тонким (ствол под корешками) ---
  const branches = layout.nodes
    .filter((nd) => nd.bornTs <= animTs)
    .sort((a, b) => b.width - a.width);

  for (const nd of branches) {
    // Файл-ветвь рисуем только пока файл жив; директорную — всегда после рождения.
    if (nd.isFile && !alive.has(nd.path)) continue;
    const grow = Math.max(0, Math.min(1, (animTs - nd.bornTs) / (GROW_DAYS * DAY)));
    if (grow <= 0) continue;

    // Конец растущей ветви.
    const ex = nd.px + (nd.x - nd.px) * grow;
    const ey = nd.py + (nd.y - nd.py) * grow;
    // Органический изгиб вниз: кубическая Безье с вертикально смещёнными
    // контрольными точками (корень «свисает» от родителя, потом уходит к цели)
    // + боковой S-извив по хэшу пути — ветви не прямые, как настоящие корешки.
    const dxTot = nd.x - nd.px;
    const c1x = nd.px + dxTot * 0.15 + nd.wob * 10;
    const c1y = nd.py + (ey - nd.py) * 0.42;
    const c2x = nd.px + dxTot * 0.72 - nd.wob * 12;
    const c2y = nd.py + (ey - nd.py) * 0.8;
    const curve = () => {
      ctx.beginPath();
      ctx.moveTo(nd.px, nd.py);
      ctx.bezierCurveTo(c1x, c1y, c2x, c2y, ex, ey);
      ctx.stroke();
    };

    // Подсветка недавно тронутой ветви (свежесть касания файла-листа).
    let live = 0;
    if (nd.isFile) {
      const t = alive.get(nd.path);
      if (t != null) live = Math.max(0, 1 - (animTs - t) / (FLASH_DAYS * DAY));
    }

    // Свечение: широкий бледный штрих + яркая сердцевина.
    ctx.strokeStyle = `rgba(201,158,106,${(0.05 + 0.12 * live).toFixed(3)})`;
    ctx.lineWidth = nd.width * 2.4;
    curve();

    ctx.strokeStyle = live > 0.02
      ? `rgba(${mix(201, 255, live)},${mix(158, 220, live)},${mix(106, 150, live)},${(0.55 + 0.4 * live).toFixed(3)})`
      : "rgba(178,138,92,0.5)";
    ctx.lineWidth = nd.width;
    curve();
  }

  // --- Файлы-почки на концах живых веток ---
  for (const nd of layout.nodes) {
    if (!nd.isFile || !alive.has(nd.path)) continue;
    if (nd.bornTs > animTs) continue;
    const bloom = Math.max(0, Math.min(1, (animTs - nd.bornTs) / (BLOOM_DAYS * DAY)));
    if (bloom <= 0) continue;
    const t = alive.get(nd.path)!;
    const flash = Math.max(0, 1 - (animTs - t) / (FLASH_DAYS * DAY));
    const r = (1.6 + 1.8 * bloom) * (1 + 0.9 * flash);

    // Гало.
    ctx.fillStyle = hexA(nd.color, 0.16 + 0.34 * flash);
    ctx.beginPath();
    ctx.arc(nd.x, nd.y, r + 3 + 6 * flash, 0, 2 * Math.PI);
    ctx.fill();
    // Ядро.
    ctx.fillStyle = hexA(nd.color, 0.75 + 0.25 * flash);
    ctx.beginPath();
    ctx.arc(nd.x, nd.y, r, 0, 2 * Math.PI);
    ctx.fill();
  }
}

function mix(a: number, b: number, t: number): number {
  return Math.round(a + (b - a) * Math.max(0, Math.min(1, t)));
}

function hexA(hex: string, a: number): string {
  const m = hex.match(/#(..)(..)(..)/);
  if (!m) return `rgba(224,161,90,${a})`;
  return `rgba(${parseInt(m[1], 16)},${parseInt(m[2], 16)},${parseInt(m[3], 16)},${a.toFixed(3)})`;
}
