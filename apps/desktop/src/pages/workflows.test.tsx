import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { QuarantinePage } from "./QuarantinePage";
import { RecoveryPage } from "./RecoveryPage";
import { ResultsPage } from "./ResultsPage";
import { ScanPage } from "./ScanPage";
import { HistoryPage } from "./HistoryPage";
import type { DuplicateGroup, ScanSession } from "../types";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
  true;

const mocks = vi.hoisted(() => ({
  listProjects: vi.fn(),
  startScan: vi.fn(),
  scanStatus: vi.fn(),
  nextScanEvent: vi.fn(),
  controlScan: vi.fn(),
  resumeScan: vi.fn(),
  listResults: vi.fn(),
  createPlan: vi.fn(),
  latestPlanForSession: vi.fn(),
  dryRun: vi.fn(),
  exportReport: vi.fn(),
  applyQuarantine: vi.fn(),
  listQuarantine: vi.fn(),
  restore: vi.fn(),
  preparePermanentDelete: vi.fn(),
  executePermanentDelete: vi.fn(),
  inspectRecovery: vi.fn(),
  reconcile: vi.fn(),
  listFileHistory: vi.fn(),
  storageOverview: vi.fn(),
  optimizeStorage: vi.fn(),
  cleanupOldLogs: vi.fn(),
  clearInterfaceCache: vi.fn(),
}));

vi.mock("../services/backend", () => ({ backend: mocks }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));

const running: ScanSession = {
  id: "session-1",
  state: "quick_hashing",
  discovered_files: 10,
  processed_files: 5,
  bytes_read: 4096,
  errors: 1,
  skipped: 2,
  unstable: 0,
  duplicate_groups: 0,
  reclaimable_bytes: 0,
  started_at: new Date(Date.now() - 10_000).toISOString(),
};

const group: DuplicateGroup = {
  id: "group-1",
  mode: "strict",
  size_bytes: 12,
  normalized_name: "book.pdf",
  blake3: Array(32).fill(1),
  sha256: Array(32).fill(2),
  members: [
    {
      file: {
        metadata: { path: "D:\\Primary\\book.pdf", size_bytes: 12, modified_ns: 0 },
        blake3: { algorithm: "blake3", digest: [1], bytes_read: 12, stable: true },
        sha256: { algorithm: "sha256", digest: [2], bytes_read: 12, stable: true },
      },
      action: "keep",
      reason: "primary root",
    },
    {
      file: {
        metadata: { path: "D:\\Copy\\book.pdf", size_bytes: 12, modified_ns: 1 },
        blake3: { algorithm: "blake3", digest: [1], bytes_read: 12, stable: true },
        sha256: { algorithm: "sha256", digest: [2], bytes_read: 12, stable: true },
      },
      action: "quarantine",
      reason: "duplicate copy",
    },
  ],
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  mocks.listProjects.mockResolvedValue([
    { id: "project-1", name: "Library", mode: "strict", worker_limit: 4, status: "active" },
  ]);
  mocks.startScan.mockResolvedValue("session-1");
  mocks.scanStatus.mockResolvedValue(running);
  mocks.nextScanEvent.mockResolvedValue(null);
  mocks.controlScan.mockResolvedValue(undefined);
  mocks.resumeScan.mockResolvedValue("session-1");
  mocks.listResults.mockResolvedValue([group]);
  mocks.createPlan.mockResolvedValue("plan-1");
  mocks.latestPlanForSession.mockResolvedValue(null);
  mocks.dryRun.mockResolvedValue({
    plan_id: "plan-1",
    session_id: "session-1",
    status: "sealed",
    groups: 1,
    quarantine_files: 1,
    quarantine_bytes: 12,
  });
  mocks.applyQuarantine.mockResolvedValue(12);
  mocks.listQuarantine.mockResolvedValue([]);
  mocks.restore.mockResolvedValue(undefined);
  mocks.preparePermanentDelete.mockResolvedValue({
    batch_id: "delete-batch-1",
    token: "one-time-token",
    mode: "retention_expired",
    confirmation_phrase:
      "XÓA VĨNH VIỄN 1 TỆP ĐÃ CÁCH LY (12 BYTE) TRONG TRÌNH TÌM TỆP TRÙNG LẶP AN TOÀN 0.2.0",
    entry_count: 1,
    total_bytes: 12,
    expires_at: "2099-01-01T00:00:00Z",
  });
  mocks.executePermanentDelete.mockResolvedValue({ deleted_entries: 1, deleted_bytes: 12 });
  mocks.inspectRecovery.mockResolvedValue([
    {
      id: "transaction-1",
      state: "recovery_required",
      source: "D:\\Primary\\book.pdf",
      destination: "D:\\.quarantine\\book.pdf",
      size_bytes: 12,
    },
  ]);
  mocks.reconcile.mockResolvedValue("VerifiedDestination");
  mocks.listFileHistory.mockResolvedValue({
    total_matching: 1,
    total_processed: 51_597,
    duplicate_files: 2_950,
    duplicate_groups: 1_149,
    problem_files: 0,
    items: [
      {
        snapshot_id: "snapshot-1",
        session_id: "session-1",
        path: "F:\\Library\\copy.pdf",
        size_bytes: 1024,
        state: "planned_quarantine",
        access_status: "readable",
        observed_at: "2026-07-22T00:00:00Z",
        group_id: "group-1",
        action: "quarantine",
        reason: "duplicate copy",
        plan_status: "sealed",
        duplicate_locations: ["F:\\Library\\original.pdf"],
      },
    ],
  });
  mocks.storageOverview.mockResolvedValue({
    data_directory: "C:\\AppData\\SafeDedupe",
    database_bytes: 200_000_000,
    manifest_bytes: 6_000_000,
    log_bytes: 1_600,
    interface_cache_bytes: 70_000_000,
    other_bytes: 0,
    total_bytes: 276_001_600,
  });
  mocks.optimizeStorage.mockResolvedValue({
    before_bytes: 200_000_000,
    after_bytes: 190_000_000,
    reclaimed_bytes: 10_000_000,
  });
  mocks.cleanupOldLogs.mockResolvedValue({ deleted_files: 0, reclaimed_bytes: 0 });
  mocks.clearInterfaceCache.mockResolvedValue(undefined);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("desktop workflows", () => {
  it("starts only on click and renders monotonic scan progress", async () => {
    const onSession = vi.fn();
    await render(
      <ScanPage
        projectId="project-1"
        sessionId=""
        onProjectChange={vi.fn()}
        onSession={onSession}
        onReview={vi.fn()}
      />,
    );
    expect(mocks.startScan).not.toHaveBeenCalled();

    await clickButton("Bắt đầu quét chỉ đọc");

    expect(mocks.startScan).toHaveBeenCalledWith("project-1", "strict", false, false);
    expect(onSession).toHaveBeenCalledWith("session-1");
    expect(container.textContent).toContain("10Đã phát hiện");
    expect(container.textContent).toContain("5Đã xử lý");
    expect(container.querySelector('[aria-label="50%"]')).not.toBeNull();
  });

  it("shows the persisted root cause when a scan is blocked", async () => {
    mocks.scanStatus.mockResolvedValue({
      ...running,
      state: "blocked",
      blocked_reason: "Lỗi lưu trữ bền vững: đường dẫn Unicode trùng khóa trong lô metadata",
    });
    await render(
      <ScanPage
        projectId="project-1"
        sessionId=""
        onProjectChange={vi.fn()}
        onSession={vi.fn()}
        onReview={vi.fn()}
      />,
    );

    await clickButton("Bắt đầu quét chỉ đọc");
    expect(container.textContent).toContain("Nguyên nhân bị chặn");
    expect(container.textContent).toContain("đường dẫn Unicode trùng khóa");
  });

  it("automatically loads proven results, summarizes removable copies, and shows dry-run totals", async () => {
    const onPlan = vi.fn();
    await render(
      <ResultsPage
        sessionId="session-1"
        planId=""
        onSessionChange={vi.fn()}
        onPlan={onPlan}
        onQuarantine={vi.fn()}
      />,
    );
    expect(mocks.listResults).toHaveBeenCalledWith("session-1");
    expect(container.textContent).toContain("book.pdf");
    expect(container.textContent).toContain("BLAKE3 + SHA-256");
    expect(container.textContent).toContain("1Bản sao có thể cách ly");
    expect(container.textContent).toContain("12 BDung lượng có thể thu hồi");
    expect(container.textContent).toContain("Bước tiếp theo");

    const policy = container.querySelector("select");
    if (!policy) throw new Error("keeper policy select missing");
    await setValue(policy, "oldest");
    await clickButton("Tạo và khóa kế hoạch");

    expect(mocks.createPlan).toHaveBeenCalledWith("session-1", "oldest");
    expect(onPlan).toHaveBeenCalledWith("plan-1");
    expect(container.textContent).toContain("Chạy thử · không thay đổi dữ liệu");
    expect(container.textContent).toContain("1 tệp");
  });

  it("restores the newest sealed plan for the selected session", async () => {
    mocks.latestPlanForSession.mockResolvedValue("restored-plan");
    const onPlan = vi.fn();
    await render(
      <ResultsPage
        sessionId="session-1"
        planId=""
        onSessionChange={vi.fn()}
        onPlan={onPlan}
        onQuarantine={vi.fn()}
      />,
    );

    expect(mocks.latestPlanForSession).toHaveBeenCalledWith("session-1");
    expect(onPlan).toHaveBeenCalledWith("restored-plan");
  });

  it("renders large result sets in bounded pages", async () => {
    mocks.listResults.mockResolvedValue(
      Array.from({ length: 81 }, (_, index) => ({
        ...group,
        id: `group-${index + 1}`,
        normalized_name: `book-${index + 1}.pdf`,
      })),
    );
    await render(
      <ResultsPage
        sessionId="session-large"
        planId=""
        onSessionChange={vi.fn()}
        onPlan={vi.fn()}
        onQuarantine={vi.fn()}
      />,
    );

    expect(container.querySelectorAll("article.group")).toHaveLength(40);
    expect(container.textContent).toContain("Trang 1/3 · hiển thị 1–40/81 nhóm");
    expect(container.textContent).toContain("book-1.pdf");
    expect(container.textContent).not.toContain("book-41.pdf");

    await clickButton("Trang sau");
    expect(container.querySelectorAll("article.group")).toHaveLength(40);
    expect(container.textContent).toContain("Trang 2/3 · hiển thị 41–80/81 nhóm");
    expect(container.textContent).toContain("book-41.pdf");
    expect(container.textContent).not.toContain("book-1.pdf");
  });

  it("gates quarantine and recovery actions on exact typed confirmations", async () => {
    await render(
      <QuarantinePage
        projectId="project-1"
        planId="plan-1"
        onProjectChange={vi.fn()}
        onPlanChange={vi.fn()}
      />,
    );
    const quarantineButton = button("Chuyển các bản sao đã xem xét vào vùng cách ly");
    expect(quarantineButton.disabled).toBe(true);
    const confirmation = inputFollowingText("Nhập chính xác QUARANTINE");
    await setValue(confirmation, "quarantine");
    expect(quarantineButton.disabled).toBe(true);
    await setValue(confirmation, "QUARANTINE");
    expect(quarantineButton.disabled).toBe(false);
    await act(async () => quarantineButton.click());
    expect(mocks.applyQuarantine).toHaveBeenCalledWith("plan-1", "QUARANTINE");

    await act(async () => root.unmount());
    root = createRoot(container);
    await render(<RecoveryPage projectId="project-1" onProjectChange={vi.fn()} />);
    await clickButton("Kiểm tra giao dịch bị gián đoạn");
    const reconcileButton = button("Đối soát");
    expect(reconcileButton.disabled).toBe(true);
    const recoveryToken = container.querySelector<HTMLInputElement>(
      'input[placeholder="Nhập RECONCILE"]',
    );
    if (!recoveryToken) throw new Error("recovery confirmation missing");
    await setValue(recoveryToken, "RECONCILE");
    expect(reconcileButton.disabled).toBe(false);
    await act(async () => reconcileButton.click());
    expect(mocks.reconcile).toHaveBeenCalledWith("transaction-1", "RECONCILE");
  });

  it("requires explicit selection and a simple checkbox while preserving the bound backend challenge", async () => {
    mocks.listQuarantine.mockResolvedValue([
      {
        id: "entry-1",
        project_id: "project-1",
        original_path: "D:\\Copy\\book.pdf",
        quarantine_path: "D:\\.quarantine\\book.pdf",
        size_bytes: 12,
        state: "verified",
        permanent_delete_state: "active",
        retain_until: "2020-01-01T00:00:00Z",
        quarantined_at: "2019-01-01T00:00:00Z",
      },
    ]);
    await render(
      <QuarantinePage
        projectId="project-1"
        planId=""
        onProjectChange={vi.fn()}
        onPlanChange={vi.fn()}
      />,
    );
    await clickButton("Làm mới");
    const selection = container.querySelector<HTMLInputElement>(
      'input[type="checkbox"][aria-label*="xóa vĩnh viễn"]',
    );
    if (!selection) throw new Error("individual permanent-delete selection missing");
    await act(async () => selection.click());
    expect(container.textContent).toContain("1 tệp");
    expect(container.textContent).toContain("12 byte");
    await clickButton("Chuẩn bị thử thách xóa");
    expect(mocks.preparePermanentDelete).toHaveBeenCalledWith(["entry-1"], false);

    const executeButton = button("Xóa vĩnh viễn đúng 1 tệp");
    expect(executeButton.disabled).toBe(true);
    expect(container.querySelector('input[aria-label="Token xóa vĩnh viễn"]')).toBeNull();
    expect(
      container.querySelector('textarea[aria-label="Câu xác nhận xóa vĩnh viễn chính xác"]'),
    ).toBeNull();
    const finalConfirmation = container.querySelector<HTMLInputElement>(
      'input[aria-label="Tôi xác nhận xóa vĩnh viễn đúng 1 tệp"]',
    );
    if (!finalConfirmation) throw new Error("simple permanent-delete confirmation missing");
    await act(async () => finalConfirmation.click());
    expect(executeButton.disabled).toBe(false);
    await act(async () => executeButton.click());
    expect(mocks.executePermanentDelete).toHaveBeenCalledWith(
      "delete-batch-1",
      "one-time-token",
      "XÓA VĨNH VIỄN 1 TỆP ĐÃ CÁCH LY (12 BYTE) TRONG TRÌNH TÌM TỆP TRÙNG LẶP AN TOÀN 0.2.0",
    );
  });

  it("requires an explicit Xóa ngay mode before selecting a retained entry", async () => {
    mocks.listQuarantine.mockResolvedValue([
      {
        id: "retained-entry",
        project_id: "project-1",
        original_path: "F:\\Library\\retained.pdf",
        quarantine_path: "F:\\.quarantine\\retained.pdf",
        size_bytes: 4096,
        state: "verified",
        permanent_delete_state: "active",
        retain_until: "2099-01-01T00:00:00Z",
        quarantined_at: "2026-01-01T00:00:00Z",
      },
    ]);
    mocks.preparePermanentDelete.mockResolvedValueOnce({
      batch_id: "immediate-batch",
      token: "immediate-token",
      mode: "immediate",
      confirmation_phrase:
        "XÓA NGAY VĨNH VIỄN 1 TỆP ĐÃ CÁCH LY (4096 BYTE) TRONG TRÌNH TÌM TỆP TRÙNG LẶP AN TOÀN 0.2.0",
      entry_count: 1,
      total_bytes: 4096,
      expires_at: "2099-01-01T00:10:00Z",
    });
    await render(
      <QuarantinePage
        projectId="project-1"
        planId=""
        onProjectChange={vi.fn()}
        onPlanChange={vi.fn()}
      />,
    );
    await clickButton("Làm mới");
    expect(
      container.querySelector('input[type="checkbox"][aria-label*="xóa vĩnh viễn"]'),
    ).toBeNull();

    const immediateMode = container.querySelector<HTMLInputElement>(
      'input[aria-label="Bật chế độ Xóa ngay"]',
    );
    if (!immediateMode) throw new Error("immediate-delete mode missing");
    await act(async () => immediateMode.click());
    const selection = container.querySelector<HTMLInputElement>(
      'input[type="checkbox"][aria-label*="xóa vĩnh viễn"]',
    );
    if (!selection) throw new Error("retained entry did not become explicitly selectable");
    await act(async () => selection.click());
    await clickButton("Chuẩn bị thử thách Xóa ngay");

    expect(mocks.preparePermanentDelete).toHaveBeenCalledWith(["retained-entry"], true);
    expect(container.textContent).toContain("Tôi hiểu và xác nhận xóa vĩnh viễn đúng 1 tệp");
  });

  it("selects and clears all eligible visible quarantine entries in one action", async () => {
    mocks.listQuarantine.mockResolvedValue([
      {
        id: "retained-1",
        project_id: "project-1",
        original_path: "F:\\Library\\one.pdf",
        quarantine_path: "F:\\.quarantine\\one.pdf",
        size_bytes: 100,
        state: "verified",
        permanent_delete_state: "active",
        retain_until: "2099-01-01T00:00:00Z",
        quarantined_at: "2026-01-01T00:00:00Z",
      },
      {
        id: "retained-2",
        project_id: "project-1",
        original_path: "F:\\Library\\two.pdf",
        quarantine_path: "F:\\.quarantine\\two.pdf",
        size_bytes: 200,
        state: "verified",
        permanent_delete_state: "active",
        retain_until: "2099-01-01T00:00:00Z",
        quarantined_at: "2026-01-01T00:00:00Z",
      },
      {
        id: "restored",
        project_id: "project-1",
        original_path: "F:\\Library\\restored.pdf",
        quarantine_path: "F:\\.quarantine\\restored.pdf",
        size_bytes: 300,
        state: "restored",
        permanent_delete_state: "active",
        retain_until: "2020-01-01T00:00:00Z",
        quarantined_at: "2019-01-01T00:00:00Z",
      },
    ]);
    await render(
      <QuarantinePage
        projectId="project-1"
        planId=""
        onProjectChange={vi.fn()}
        onPlanChange={vi.fn()}
      />,
    );
    await clickButton("Làm mới");
    const immediateMode = container.querySelector<HTMLInputElement>(
      'input[aria-label="Bật chế độ Xóa ngay"]',
    );
    if (!immediateMode) throw new Error("immediate-delete mode missing");
    await act(async () => immediateMode.click());

    await clickButton("Chọn tất cả 2 mục đủ điều kiện");
    expect(container.textContent).toContain("Đã chọn 2 mục để xóa vĩnh viễn");
    expect(container.textContent).toContain("2 tệp");
    expect(container.textContent).toContain("300 byte");
    expect(
      [...container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].filter(
        (input) => input.getAttribute("aria-label")?.includes("xóa vĩnh viễn") && input.checked,
      ),
    ).toHaveLength(2);

    await clickButton("Bỏ chọn tất cả");
    expect(container.textContent).toContain("Đã chọn 0 mục để xóa vĩnh viễn");
    expect(container.textContent).toContain("0 tệp");
  });

  it("shows durable file history with every proven duplicate location", async () => {
    await render(<HistoryPage projectId="project-1" onProjectChange={vi.fn()} />);

    expect(mocks.storageOverview).toHaveBeenCalled();
    expect(mocks.listFileHistory).toHaveBeenCalledWith("project-1", "", true, 0, 50);
    expect(container.textContent).toContain("51.597");
    expect(container.textContent).toContain("1.149");
    expect(container.textContent).toContain("F:\\Library\\copy.pdf");
    expect(container.textContent).toContain("F:\\Library\\original.pdf");
    expect(container.textContent).toContain("kế hoạch đã khóa");
  });
});

async function render(element: React.ReactNode) {
  await act(async () => {
    root.render(element);
    await Promise.resolve();
  });
}

function button(label: string) {
  const match = [...container.querySelectorAll("button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  if (!(match instanceof HTMLButtonElement)) throw new Error(`button missing: ${label}`);
  return match;
}

async function clickButton(label: string) {
  await act(async () => {
    button(label).click();
    await Promise.resolve();
  });
}

function inputFollowingText(label: string) {
  const match = [...container.querySelectorAll("label")].find((candidate) =>
    candidate.textContent?.includes(label),
  );
  const input = match?.querySelector("input");
  if (!(input instanceof HTMLInputElement)) throw new Error(`input missing: ${label}`);
  return input;
}

async function setValue(
  element: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement,
  value: string,
) {
  await act(async () => {
    const prototype =
      element instanceof HTMLSelectElement
        ? HTMLSelectElement.prototype
        : element instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
    if (!setter) throw new Error("native value setter missing");
    setter.call(element, value);
    element.dispatchEvent(new Event("change", { bubbles: true }));
    element.dispatchEvent(new Event("input", { bubbles: true }));
    await Promise.resolve();
  });
}
