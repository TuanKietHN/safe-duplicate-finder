export type Mode = "strict" | "content";
export type Page = "projects" | "scan" | "results" | "quarantine" | "recovery" | "history";

export interface LatestPlanContext {
  project_id: string;
  session_id: string;
  plan_id: string;
}

export interface ProjectRecord {
  id: string;
  name: string;
  mode: Mode;
  worker_limit: number;
  status: string;
  last_scan_at?: string;
}

export interface ProjectRootRecord {
  id: string;
  path: string;
  primary: boolean;
}

export interface FilterConfig {
  include_extensions: string[];
  exclude_extensions: string[];
  exclude_globs: string[];
  minimum_size: number;
  skip_hidden: boolean;
  skip_system: boolean;
}

export interface ScanSession {
  id: string;
  state: string;
  discovered_files: number;
  processed_files: number;
  bytes_read: number;
  errors: number;
  skipped: number;
  unstable: number;
  duplicate_groups: number;
  reclaimable_bytes: number;
  started_at?: string;
  finished_at?: string;
  blocked_reason?: string;
}

export interface DesktopEvent {
  schema_version: number;
  sequence: number;
  project_id: string;
  session_id: string;
  emitted_at: string;
  kind: "scan://snapshot" | "scan://state";
  progress: Pick<
    ScanSession,
    "discovered_files" | "processed_files" | "bytes_read" | "errors" | "skipped" | "unstable"
  >;
}

export interface HashResult {
  algorithm: string;
  digest: number[];
  bytes_read: number;
  stable: boolean;
}

export interface DuplicateMember {
  file: {
    metadata: { path: string; size_bytes: number; modified_ns: number };
    blake3: HashResult;
    sha256: HashResult;
  };
  action: "keep" | "quarantine" | "manual";
  reason: string;
}

export interface DuplicateGroup {
  id: string;
  mode: Mode;
  size_bytes: number;
  normalized_name?: string;
  blake3: number[];
  sha256: number[];
  members: DuplicateMember[];
}

export interface PlanSummary {
  plan_id: string;
  session_id: string;
  status: string;
  groups: number;
  quarantine_files: number;
  quarantine_bytes: number;
}

export interface QuarantineEntry {
  id: string;
  project_id: string;
  original_path: string;
  quarantine_path: string;
  size_bytes: number;
  state: string;
  permanent_delete_state: string;
  retain_until: string;
  quarantined_at: string;
}

export interface PermanentDeleteChallenge {
  batch_id: string;
  token: string;
  mode: "retention_expired" | "immediate";
  confirmation_phrase: string;
  entry_count: number;
  total_bytes: number;
  expires_at: string;
}

export interface PermanentDeleteOutcome {
  deleted_entries: number;
  deleted_bytes: number;
}

export interface RecoveryTransaction {
  id: string;
  state: string;
  source: string;
  destination: string;
  size_bytes: number;
}

export interface FileHistoryRecord {
  snapshot_id: string;
  session_id: string;
  path: string;
  size_bytes: number;
  state: string;
  access_status: string;
  observed_at: string;
  completed_at?: string | null;
  group_id?: string | null;
  action?: string | null;
  reason?: string | null;
  plan_status?: string | null;
  transaction_status?: string | null;
  quarantine_path?: string | null;
  quarantine_state?: string | null;
  permanent_delete_state?: string | null;
  duplicate_locations: string[];
}

export interface FileHistoryPage {
  total_matching: number;
  total_processed: number;
  duplicate_files: number;
  duplicate_groups: number;
  problem_files: number;
  items: FileHistoryRecord[];
}

export interface StorageOverview {
  data_directory: string;
  database_bytes: number;
  manifest_bytes: number;
  log_bytes: number;
  interface_cache_bytes: number;
  other_bytes: number;
  total_bytes: number;
}

export interface DatabaseMaintenance {
  before_bytes: number;
  after_bytes: number;
  reclaimed_bytes: number;
}

export interface LogCleanup {
  deleted_files: number;
  reclaimed_bytes: number;
}
