import type { DuplicateGroup, RecoveryTransaction, ScanSession } from "../types";

export interface WorkflowState {
  scan: ScanSession | null;
  groups: DuplicateGroup[];
  selectedPolicy: "default" | "oldest" | "newest" | "shortest";
  confirmations: Record<string, string>;
  recovery: RecoveryTransaction[];
}

type Listener = () => void;

/** UI-only workflow projection. Backend commands remain authoritative for every safety decision. */
export class WorkflowStore {
  private snapshot: WorkflowState = {
    scan: null,
    groups: [],
    selectedPolicy: "default",
    confirmations: {},
    recovery: [],
  };
  private readonly listeners = new Set<Listener>();

  getSnapshot = () => this.snapshot;

  subscribe = (listener: Listener) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  mergeScan(next: ScanSession) {
    const current = this.snapshot.scan;
    this.patch({
      scan:
        current?.id === next.id
          ? {
              ...next,
              discovered_files: Math.max(current.discovered_files, next.discovered_files),
              processed_files: Math.max(current.processed_files, next.processed_files),
              bytes_read: Math.max(current.bytes_read, next.bytes_read),
              errors: Math.max(current.errors, next.errors),
              skipped: Math.max(current.skipped, next.skipped),
              unstable: Math.max(current.unstable, next.unstable),
            }
          : next,
    });
  }

  setGroups(groups: DuplicateGroup[]) {
    this.patch({ groups: [...groups] });
  }

  selectPolicy(policy: WorkflowState["selectedPolicy"]) {
    this.patch({ selectedPolicy: policy });
  }

  setConfirmation(operation: string, value: string) {
    this.patch({ confirmations: { ...this.snapshot.confirmations, [operation]: value } });
  }

  isConfirmed(operation: string, expected: string) {
    return this.snapshot.confirmations[operation] === expected;
  }

  setRecovery(recovery: RecoveryTransaction[]) {
    this.patch({ recovery: [...recovery] });
  }

  private patch(next: Partial<WorkflowState>) {
    this.snapshot = { ...this.snapshot, ...next };
    this.listeners.forEach((listener) => listener());
  }
}

export const workflowStore = new WorkflowStore();
