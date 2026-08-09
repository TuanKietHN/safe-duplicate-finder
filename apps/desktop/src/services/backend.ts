import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type {
  DatabaseMaintenance,
  DuplicateGroup,
  DesktopEvent,
  FilterConfig,
  FileHistoryPage,
  LatestPlanContext,
  LogCleanup,
  PermanentDeleteChallenge,
  PermanentDeleteOutcome,
  PlanSummary,
  ProjectRecord,
  ProjectRootRecord,
  QuarantineEntry,
  RecoveryTransaction,
  ScanSession,
  StorageOverview,
} from "../types";

const tauriAvailable = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const call = <T>(command: string, args?: Record<string, unknown>) => {
  if (!tauriAvailable)
    return Promise.reject(new Error("Thao tác này yêu cầu ứng dụng desktop Tauri."));
  return invoke<T>(command, args);
};

export const backend = {
  status: () =>
    tauriAvailable
      ? call<string>("engine_status")
      : Promise.resolve("Bản xem trước trình duyệt · chưa kết nối bộ máy desktop"),
  listProjects: () =>
    tauriAvailable ? call<ProjectRecord[]>("list_projects") : Promise.resolve([]),
  createProject: (name: string, mode: string) => call<string>("create_project", { name, mode }),
  updateProject: (projectId: string, name: string, mode: string) =>
    call<void>("update_project", { projectId, name, mode }),
  setProjectWorkers: (projectId: string, workers: number) =>
    call<void>("set_project_workers", { projectId, workers }),
  archiveProject: (projectId: string, confirmation: string) =>
    call<void>("archive_project", { projectId, confirmation }),
  getFilterConfig: (projectId: string) => call<FilterConfig>("get_filter_config", { projectId }),
  saveFilterConfig: (projectId: string, config: FilterConfig) =>
    call<void>("save_filter_config", { projectId, config }),
  addRoot: (projectId: string, path: string, primary: boolean) =>
    call<string>("add_root", { projectId, path, primary }),
  listRoots: (projectId: string) => call<ProjectRootRecord[]>("list_roots", { projectId }),
  removeRoot: (projectId: string, rootId: string) =>
    call<void>("remove_root", { projectId, rootId }),
  startScan: (projectId: string, mode: string, acknowledged: boolean, allFiles: boolean) =>
    call<string>("start_scan", { projectId, mode, acknowledged, allFiles }),
  resumeScan: (sessionId: string) => call<string>("resume_scan", { sessionId }),
  scanStatus: (sessionId: string) => call<ScanSession>("scan_status", { sessionId }),
  nextScanEvent: (sessionId: string) => call<DesktopEvent | null>("next_scan_event", { sessionId }),
  controlScan: (sessionId: string, action: "pause" | "resume" | "cancel") =>
    call<void>("control_scan", { sessionId, action }),
  listResults: (sessionId: string) => call<DuplicateGroup[]>("list_results", { sessionId }),
  createPlan: (sessionId: string, policy: string) =>
    call<string>("create_plan", { sessionId, policy }),
  latestPlanForSession: (sessionId: string) =>
    call<string | null>("latest_plan_for_session", { sessionId }),
  latestPlanContext: () =>
    tauriAvailable ? call<LatestPlanContext | null>("latest_plan_context") : Promise.resolve(null),
  dryRun: (planId: string) => call<PlanSummary>("dry_run", { planId }),
  applyQuarantine: (planId: string, confirmation: string) =>
    call<number>("apply_quarantine", { planId, confirmation }),
  listQuarantine: (projectId: string) => call<QuarantineEntry[]>("list_quarantine", { projectId }),
  preparePermanentDelete: (entryIds: string[], deleteNow: boolean) =>
    call<PermanentDeleteChallenge>("prepare_permanent_delete", { entryIds, deleteNow }),
  executePermanentDelete: (batchId: string, token: string, confirmation: string) =>
    call<PermanentDeleteOutcome>("execute_permanent_delete", {
      batchId,
      token,
      confirmation,
    }),
  restore: (entryId: string, confirmation: string) =>
    call<void>("restore_entry", { entryId, confirmation }),
  inspectRecovery: (projectId: string) =>
    call<RecoveryTransaction[]>("inspect_recovery", { projectId }),
  reconcile: (transactionId: string, confirmation: string) =>
    call<string>("reconcile_transaction", { transactionId, confirmation }),
  exportReport: (sessionId: string, format: string, destination: string) =>
    call<void>("export_report", { sessionId, format, destination }),
  listFileHistory: (
    projectId: string,
    search: string,
    duplicateOnly: boolean,
    offset: number,
    limit: number,
  ) =>
    call<FileHistoryPage>("list_file_history", {
      projectId,
      search,
      duplicateOnly,
      offset,
      limit,
    }),
  storageOverview: () => call<StorageOverview>("storage_overview"),
  optimizeStorage: () => call<DatabaseMaintenance>("optimize_storage"),
  cleanupOldLogs: (olderThanDays: number) =>
    call<LogCleanup>("cleanup_old_logs", { olderThanDays }),
  clearInterfaceCache: () => getCurrentWebview().clearAllBrowsingData(),
};
