import type { ChronographExport } from "./types";

/** Распарсить и минимально проверить chronograph.json. */
export function parseExport(text: string): ChronographExport {
  let doc: unknown;
  try {
    doc = JSON.parse(text);
  } catch {
    throw new Error("file is not valid JSON");
  }
  const d = doc as Partial<ChronographExport>;
  if (!d || typeof d !== "object" || !d.meta) {
    throw new Error("does not look like chronograph.json: missing meta field");
  }
  if (d.meta.schema_version !== 1) {
    throw new Error(
      `unsupported export schema version: ${d.meta.schema_version} (expected 1)`,
    );
  }
  return d as ChronographExport;
}

/** Прочитать File (drag-drop / file picker). */
export function readFile(file: File): Promise<ChronographExport> {
  return file.text().then(parseExport);
}

/** Загрузить по URL (?src=... при отдаче по http). */
export async function fetchExport(url: string): Promise<ChronographExport> {
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`HTTP ${resp.status} while loading ${url}`);
  return parseExport(await resp.text());
}
