// Типы chronograph.json — зеркало схемы экспорта (chronograph-report/src/export.rs).
// Версия схемы: meta.schema_version === 1. Меняется схема в Rust → меняется здесь.

export interface ExportMeta {
  schema_version: number;
  engine_version: string;
  config_hash: string;
  head_sha: string;
  /** unix-секунды UTC; якорь, от которого отсчитаны age-дни. */
  anchor_ts: number;
  total_commits: number;
  total_authors: number;
  anonymized: boolean;
}

export interface FileMetrics {
  path: string;
  churn_total: number | null;
  churn_30d: number | null;
  churn_90d: number | null;
  churn_365d: number | null;
  complexity: number | null;
  complexity_per_loc: number | null;
  hotspot_rank: number | null;
  is_alive: boolean | null;
}

export interface CouplingPair {
  a: string;
  b: string;
  support: number;
  ratio: number;
}

export interface KnowledgeEntry {
  path: string;
  bus_factor: number;
  top_owner_ratio: number;
  top_owner: string;
}

export interface FileAgeEntry {
  path: string;
  lines: number;
  newest_age_days: number;
  median_age_days: number;
  p90_age_days: number;
  oldest_age_days: number;
}

export interface BlameSkip {
  path: string;
  reason: string;
  cost: number | null;
  budget: number | null;
}

export interface ChangeEvent {
  path: string;
  type: string;
  old_path: string | null;
  added: number;
  deleted: number;
}

export interface CommitEvent {
  sha: string;
  ts: number;
  author: string;
  mechanical: boolean;
  changes: ChangeEvent[];
}

export interface ChronographExport {
  meta: ExportMeta;
  files: FileMetrics[];
  coupling: CouplingPair[];
  knowledge: KnowledgeEntry[];
  file_age: FileAgeEntry[];
  blame_skips: BlameSkip[];
  events: CommitEvent[];
}
