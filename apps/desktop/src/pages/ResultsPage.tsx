import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { backend } from "../services/backend";
import type { DuplicateGroup, PlanSummary } from "../types";
import { actionLabel, reasonLabel } from "../services/labels";

const GROUPS_PER_PAGE = 40;

interface Props {
  sessionId: string;
  planId: string;
  onSessionChange: (id: string) => void;
  onPlan: (id: string) => void;
  onQuarantine: () => void;
}

export function ResultsPage({ sessionId, planId, onSessionChange, onPlan, onQuarantine }: Props) {
  const [groups, setGroups] = useState<DuplicateGroup[]>([]);
  const [policy, setPolicy] = useState("default");
  const [summary, setSummary] = useState<PlanSummary | null>(null);
  const [message, setMessage] = useState("");
  const [loading, setLoading] = useState(false);
  const [planning, setPlanning] = useState(false);
  const [planMessage, setPlanMessage] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [page, setPage] = useState(1);
  const requestId = useRef(0);
  const previewPlanId = useRef("");

  const load = useCallback(async () => {
    const requestedSession = sessionId.trim();
    if (!requestedSession) {
      setGroups([]);
      setPage(1);
      setLoaded(false);
      setMessage("Chưa có phiên quét hoàn tất để tải kết quả.");
      return;
    }
    const currentRequest = ++requestId.current;
    setLoading(true);
    setLoaded(false);
    setMessage("Đang tải các nhóm trùng đã chứng minh…");
    try {
      const results = await backend.listResults(requestedSession);
      if (currentRequest !== requestId.current) return;
      setGroups(results);
      setPage(1);
      setLoaded(true);
      setMessage(
        results.length
          ? `Đã tải ${results.length.toLocaleString("vi-VN")} nhóm trùng đã chứng minh.`
          : "Phiên này đã hoàn tất nhưng không có nhóm tệp trùng được chứng minh.",
      );
    } catch (error) {
      if (currentRequest !== requestId.current) return;
      setGroups([]);
      setPage(1);
      setLoaded(false);
      setMessage(String(error));
    } finally {
      if (currentRequest === requestId.current) setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    void load();
    return () => {
      requestId.current += 1;
    };
  }, [load]);

  useEffect(() => {
    const requestedSession = sessionId.trim();
    if (!requestedSession || planId) return;
    void backend
      .latestPlanForSession(requestedSession)
      .then((id) => {
        if (id) onPlan(id);
      })
      .catch((error: unknown) => setPlanMessage(String(error)));
  }, [onPlan, planId, sessionId]);

  useEffect(() => {
    if (!planId) {
      previewPlanId.current = "";
      setSummary(null);
      return;
    }
    if (previewPlanId.current === planId) return;
    previewPlanId.current = planId;
    void backend
      .dryRun(planId)
      .then(setSummary)
      .catch((error: unknown) => setPlanMessage(String(error)));
  }, [planId]);

  const totals = useMemo(
    () =>
      groups.reduce(
        (current, group) => {
          const removableCopies = Math.max(0, group.members.length - 1);
          return {
            files: current.files + group.members.length,
            removableCopies: current.removableCopies + removableCopies,
            reclaimableBytes: current.reclaimableBytes + removableCopies * group.size_bytes,
          };
        },
        { files: 0, removableCopies: 0, reclaimableBytes: 0 },
      ),
    [groups],
  );
  const totalPages = Math.max(1, Math.ceil(groups.length / GROUPS_PER_PAGE));
  const visibleGroups = useMemo(() => {
    const start = (page - 1) * GROUPS_PER_PAGE;
    return groups.slice(start, start + GROUPS_PER_PAGE);
  }, [groups, page]);
  const visibleStart = groups.length ? (page - 1) * GROUPS_PER_PAGE + 1 : 0;
  const visibleEnd = Math.min(page * GROUPS_PER_PAGE, groups.length);

  async function makePlan() {
    if (planning) return;
    setPlanning(true);
    setSummary(null);
    setPlanMessage("Đang tạo và khóa kế hoạch trong cơ sở dữ liệu cục bộ…");
    try {
      const id = await backend.createPlan(sessionId.trim(), policy);
      previewPlanId.current = id;
      onPlan(id);
      setSummary(await backend.dryRun(id));
      setPlanMessage(`Đã khóa kế hoạch ${id}. Hệ thống tệp vẫn chưa bị thay đổi.`);
      setMessage("Đã khóa kế hoạch. Hệ thống tệp vẫn chưa bị thay đổi.");
    } catch (error) {
      setPlanMessage(String(error));
    } finally {
      setPlanning(false);
    }
  }

  async function exportReport(format: "csv" | "json" | "html") {
    try {
      const destination = await save({
        title: "Xuất báo cáo tệp trùng đã chứng minh",
        defaultPath: `bao-cao-tep-trung-an-toan.${format}`,
      });
      if (!destination) return;
      await backend.exportReport(sessionId, format, destination);
      setMessage(`Đã ghi báo cáo vào ${destination}`);
    } catch (error) {
      setMessage(String(error));
    }
  }

  return (
    <div className="stack">
      <section className="card toolbar-card">
        <label>
          Mã phiên đã hoàn tất
          <input value={sessionId} onChange={(event) => onSessionChange(event.target.value)} />
        </label>
        <button className="secondary" disabled={!sessionId || loading} onClick={() => void load()}>
          {loading ? "Đang tải…" : loaded ? "Tải lại kết quả" : "Tải các nhóm đã chứng minh"}
        </button>
        <div className="spacer" />
        <button
          className="quiet"
          onClick={() => void exportReport("csv")}
          disabled={!groups.length}
        >
          CSV
        </button>
        <button
          className="quiet"
          onClick={() => void exportReport("json")}
          disabled={!groups.length}
        >
          JSON
        </button>
        <button
          className="quiet"
          onClick={() => void exportReport("html")}
          disabled={!groups.length}
        >
          HTML
        </button>
      </section>
      {loaded && groups.length > 0 && (
        <section className="card results-summary" aria-label="Tóm tắt tệp trùng">
          <div className="summary-metrics">
            <SummaryMetric label="Nhóm trùng đã chứng minh" value={groups.length} />
            <SummaryMetric label="Tổng số tệp trong nhóm" value={totals.files} />
            <SummaryMetric label="Bản sao có thể cách ly" value={totals.removableCopies} />
            <SummaryMetric
              label="Dung lượng có thể thu hồi"
              value={formatBytes(totals.reclaimableBytes)}
            />
          </div>
          <div className="next-step">
            <strong>Bước tiếp theo</strong>
            <span>
              Xem các đường dẫn bên dưới, chọn chính sách giữ lại một tệp, rồi tạo và khóa kế hoạch.
              Ứng dụng sẽ chạy thử trước; chưa có tệp nào bị xóa hoặc di chuyển ở bước này.
            </span>
          </div>
        </section>
      )}
      <div className="content-grid results-layout">
        <section className="card result-list">
          <div className="card-heading">
            <h2>Nhóm trùng đã chứng minh</h2>
            <span className="badge">{groups.length}</span>
          </div>
          {groups.length > GROUPS_PER_PAGE && (
            <ResultsPagination
              page={page}
              totalPages={totalPages}
              visibleStart={visibleStart}
              visibleEnd={visibleEnd}
              totalGroups={groups.length}
              onPage={setPage}
            />
          )}
          {!groups.length && (
            <div className="empty">
              {loading
                ? "Đang tải bằng chứng tệp trùng…"
                : loaded
                  ? "Phiên này không có nhóm tệp trùng đã chứng minh."
                  : "Chưa có kết quả được tải."}
            </div>
          )}
          {visibleGroups.map((group) => (
            <article className="group" key={group.id}>
              <header>
                <div>
                  <strong>{group.normalized_name ?? "Nhóm chỉ so sánh nội dung"}</strong>
                  <span>
                    {group.members.length.toLocaleString("vi-VN")} tệp · có thể cách ly{" "}
                    {Math.max(0, group.members.length - 1).toLocaleString("vi-VN")} bản sao · thu
                    hồi {formatBytes(Math.max(0, group.members.length - 1) * group.size_bytes)}
                  </span>
                  <span className="evidence-line">
                    B3 {hexPrefix(group.blake3)} · SHA-256 {hexPrefix(group.sha256)}
                  </span>
                </div>
                <span className="badge safe">BLAKE3 + SHA-256</span>
              </header>
              {group.members.map((member) => (
                <div className="member" key={member.file.metadata.path}>
                  <span className={`action ${member.action}`}>{actionLabel(member.action)}</span>
                  <div>
                    <strong>{member.file.metadata.path}</strong>
                    <small>{reasonLabel(member.reason)}</small>
                    <small>
                      Sửa lúc {formatModified(member.file.metadata.modified_ns)} · bản sao vật lý ·{" "}
                      {member.file.metadata.size_bytes.toLocaleString("vi-VN")} byte
                    </small>
                  </div>
                </div>
              ))}
            </article>
          ))}
          {groups.length > GROUPS_PER_PAGE && (
            <ResultsPagination
              page={page}
              totalPages={totalPages}
              visibleStart={visibleStart}
              visibleEnd={visibleEnd}
              totalGroups={groups.length}
              onPage={setPage}
            />
          )}
        </section>
        <aside className="card plan-panel">
          <p className="eyebrow">CỔNG XEM XÉT</p>
          <h2>Khóa kế hoạch giữ tệp</h2>
          <p className="plan-help">
            Mỗi nhóm luôn giữ lại đúng một tệp. Các bản sao còn lại chỉ được chuyển vào vùng cách ly
            sau khi bạn xem phần chạy thử và nhập xác nhận chính xác.
          </p>
          <label>
            Chính sách chọn tệp giữ lại
            <select
              value={policy}
              disabled={planning || Boolean(planId)}
              onChange={(event) => setPolicy(event.target.value)}
            >
              <option value="default">Thư mục ưu tiên → cũ nhất → đường dẫn ngắn nhất</option>
              <option value="oldest">Cũ nhất</option>
              <option value="newest">Mới nhất</option>
              <option value="shortest">Đường dẫn ngắn nhất</option>
            </select>
          </label>
          <button
            className="primary"
            disabled={!groups.length || planning || Boolean(planId)}
            onClick={() => void makePlan()}
          >
            {planning
              ? "Đang tạo và khóa…"
              : planId
                ? "Kế hoạch đã được khóa"
                : "Tạo và khóa kế hoạch"}
          </button>
          <code>{planId || "Chưa khóa kế hoạch"}</code>
          {planMessage && (
            <div className="notice plan-status" role="status">
              {planMessage}
            </div>
          )}
          {summary && (
            <div className="dry-run">
              <span>Chạy thử · không thay đổi dữ liệu</span>
              <strong>{summary.quarantine_files.toLocaleString("vi-VN")} tệp</strong>
              <strong>{summary.quarantine_bytes.toLocaleString("vi-VN")} byte</strong>
              <button className="primary" onClick={onQuarantine}>
                Tiếp tục xem xét cách ly
              </button>
            </div>
          )}
        </aside>
      </div>
      {message && <div className="notice">{message}</div>}
    </div>
  );
}

function hexPrefix(bytes: number[]) {
  return `${bytes
    .slice(0, 8)
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("")}…`;
}

function formatModified(nanoseconds: number) {
  const milliseconds = nanoseconds / 1_000_000;
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? "không xác định" : date.toLocaleString("vi-VN");
}

function SummaryMetric({ label, value }: { label: string; value: number | string }) {
  return (
    <div>
      <strong>{typeof value === "number" ? value.toLocaleString("vi-VN") : value}</strong>
      <span>{label}</span>
    </div>
  );
}

function ResultsPagination({
  page,
  totalPages,
  visibleStart,
  visibleEnd,
  totalGroups,
  onPage,
}: {
  page: number;
  totalPages: number;
  visibleStart: number;
  visibleEnd: number;
  totalGroups: number;
  onPage: (page: number) => void;
}) {
  return (
    <nav className="results-pagination" aria-label="Phân trang nhóm trùng">
      <button className="quiet" disabled={page === 1} onClick={() => onPage(page - 1)}>
        Trang trước
      </button>
      <span>
        Trang {page.toLocaleString("vi-VN")}/{totalPages.toLocaleString("vi-VN")} · hiển thị{" "}
        {visibleStart.toLocaleString("vi-VN")}–{visibleEnd.toLocaleString("vi-VN")}/
        {totalGroups.toLocaleString("vi-VN")} nhóm
      </span>
      <button className="quiet" disabled={page === totalPages} onClick={() => onPage(page + 1)}>
        Trang sau
      </button>
    </nav>
  );
}

function formatBytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(units.length - 1, Math.floor(Math.log(value) / Math.log(1024)));
  return `${(value / 1024 ** index).toLocaleString("vi-VN", {
    maximumFractionDigits: index === 0 ? 0 : 1,
    minimumFractionDigits: index === 0 ? 0 : 1,
  })} ${units[index]}`;
}
