// Общий движок воспроизведения истории для Timeline (scrubber) и Роста
// репозитория (Gource-анимация). Обе вкладки — ЖИВОЙ рендер, правило
// байт-идентичности не действует (CONTEXT.md); детерминированный артефакт —
// сам chronograph.json, а он уже несёт полный поток событий per-commit.
//
// Модель: инкрементальный reducer по отсортированному потоку событий строит
// состояние дерева файлов на любой момент. Быстрый scrub назад — через
// ЧЕКПОЙНТЫ (снимок состояния каждые CHECKPOINT_EVERY событий): переход к
// индексу i = откат к ближайшему чекпойнту ≤ i + доигрывание вперёд, а не
// проигрывание с нуля. Производительность на репо с тысячами коммитов —
// решение из плана 5c.

import type { ChronographExport, CommitEvent } from "./types";

const CHECKPOINT_EVERY = 400;

/** Живой файл на момент воспроизведения. */
export interface LiveFile {
  path: string;
  /** Накопленный churn (число коснувшихся коммитов) — вес узла в анимации. */
  churn: number;
  /** Индекс события последнего касания (для «пульса» свежести). */
  lastTouch: number;
}

export interface Frame {
  /** Индекс последнего применённого события (−1 — пустой старт). */
  index: number;
  files: Map<string, LiveFile>;
}

/** Один шаг ленты для scrubber-графика (после каждого коммита-события). */
export interface TimelinePoint {
  index: number;
  ts: number;
  aliveFiles: number;
  /** Файлов, затронутых этим коммитом (высота «активности»). */
  touched: number;
  author: string;
  sha: string;
}

export class Timeline {
  readonly events: CommitEvent[];
  readonly points: TimelinePoint[];
  readonly minTs: number;
  readonly maxTs: number;
  private checkpoints: Frame[] = [];

  constructor(data: ChronographExport) {
    // Экспорт уже отсортировал события по (ts, sha); механические оставляем —
    // фильтрация на стороне UI (флаг у вьюхи), данные не режем.
    this.events = data.events;
    this.points = [];

    const files = new Map<string, LiveFile>();
    const apply = (ev: CommitEvent, idx: number) => {
      let touched = 0;
      for (const ch of ev.changes) {
        if (ch.type === "D") {
          files.delete(ch.path);
        } else if (ch.type === "R" && ch.old_path) {
          const prev = files.get(ch.old_path);
          files.delete(ch.old_path);
          const churn = (prev?.churn ?? 0) + 1;
          files.set(ch.path, { path: ch.path, churn, lastTouch: idx });
        } else {
          const cur = files.get(ch.path);
          files.set(ch.path, {
            path: ch.path,
            churn: (cur?.churn ?? 0) + 1,
            lastTouch: idx,
          });
        }
        touched += 1;
      }
      return touched;
    };

    this.events.forEach((ev, i) => {
      const touched = apply(ev, i);
      this.points.push({
        index: i,
        ts: ev.ts,
        aliveFiles: files.size,
        touched,
        author: ev.author,
        sha: ev.sha,
      });
      if (i % CHECKPOINT_EVERY === 0) {
        this.checkpoints.push({ index: i, files: cloneFiles(files) });
      }
    });

    this.minTs = this.events.length ? this.events[0].ts : 0;
    this.maxTs = this.events.length ? this.events[this.events.length - 1].ts : 0;
  }

  /** Состояние дерева файлов после применения событий [0..index]. */
  frameAt(index: number): Frame {
    const clamped = Math.max(-1, Math.min(index, this.events.length - 1));
    // Ближайший чекпойнт с index ≤ clamped.
    let base: Frame | null = null;
    for (const cp of this.checkpoints) {
      if (cp.index <= clamped) base = cp;
      else break;
    }
    const files = base ? cloneFiles(base.files) : new Map<string, LiveFile>();
    let from = base ? base.index + 1 : 0;
    if (!base && clamped < 0) return { index: -1, files };
    for (let i = from; i <= clamped; i++) {
      const ev = this.events[i];
      for (const ch of ev.changes) {
        if (ch.type === "D") {
          files.delete(ch.path);
        } else if (ch.type === "R" && ch.old_path) {
          const prev = files.get(ch.old_path);
          files.delete(ch.old_path);
          files.set(ch.path, {
            path: ch.path,
            churn: (prev?.churn ?? 0) + 1,
            lastTouch: i,
          });
        } else {
          const cur = files.get(ch.path);
          files.set(ch.path, {
            path: ch.path,
            churn: (cur?.churn ?? 0) + 1,
            lastTouch: i,
          });
        }
      }
    }
    return { index: clamped, files };
  }

  /** Индекс первого события с ts ≥ заданного (для scrub по времени). */
  indexAtTs(ts: number): number {
    let lo = 0;
    let hi = this.events.length - 1;
    let ans = this.events.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (this.events[mid].ts >= ts) {
        ans = mid;
        hi = mid - 1;
      } else {
        lo = mid + 1;
      }
    }
    return ans;
  }
}

function cloneFiles(files: Map<string, LiveFile>): Map<string, LiveFile> {
  const copy = new Map<string, LiveFile>();
  for (const [k, v] of files) copy.set(k, { ...v });
  return copy;
}

/** Формат даты из unix-секунд (UTC) — RU, без времени. */
export function fmtDate(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
}
