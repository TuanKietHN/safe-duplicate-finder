import type { Mode } from "../types";

export const isFinalScanState = (state: string) =>
  state === "completed" || state === "cancelled" || state === "blocked";

export const requiresDurableResume = (state: string) =>
  state === "interrupted" || state === "blocked";

export const canStartScan = (projectId: string, mode: Mode, acknowledged: boolean) =>
  projectId.trim().length > 0 && (mode === "strict" || acknowledged);

export const exactConfirmation = (actual: string, expected: string) => actual === expected;
