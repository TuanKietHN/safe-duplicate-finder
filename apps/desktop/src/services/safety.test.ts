import { describe, expect, it } from "vitest";
import { canStartScan, exactConfirmation, isFinalScanState, requiresDurableResume } from "./safety";

describe("non-destructive workflow gates", () => {
  it("requires an explicit acknowledgement for content-only mode", () => {
    expect(canStartScan("project-id", "strict", false)).toBe(true);
    expect(canStartScan("project-id", "content", false)).toBe(false);
    expect(canStartScan("project-id", "content", true)).toBe(true);
  });

  it("accepts mutation confirmations only as exact, case-sensitive phrases", () => {
    expect(exactConfirmation("QUARANTINE", "QUARANTINE")).toBe(true);
    expect(exactConfirmation("quarantine", "QUARANTINE")).toBe(false);
    expect(exactConfirmation("QUARANTINE ", "QUARANTINE")).toBe(false);
    expect(exactConfirmation("RESTORE", "RESTORE")).toBe(true);
  });

  it("distinguishes terminal scan states from pausable work", () => {
    expect(isFinalScanState("completed")).toBe(true);
    expect(isFinalScanState("blocked")).toBe(true);
    expect(isFinalScanState("paused")).toBe(false);
    expect(isFinalScanState("quick_hashing")).toBe(false);
    expect(requiresDurableResume("interrupted")).toBe(true);
    expect(requiresDurableResume("blocked")).toBe(true);
    expect(requiresDurableResume("paused")).toBe(false);
  });
});
