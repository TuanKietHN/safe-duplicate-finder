import { useEffect, useState } from "react";
import { backend } from "../services/backend";
import { actionLabel, quarantineStateLabel, reasonLabel } from "../services/labels";
import type { FileHistoryPage, StorageOverview } from "../types";

interface Props {
  projectId: string;
  onProjectChange: (id: string) => void;
}

const pageSize = 50;

export function HistoryPage({ projectId, onProjectChange }: Props) {
  const [history, setHistory] = useState<FileHistoryPage | null>(null);
  const [storage, setStorage] = useState<StorageOverview | null>(null);
  const [search, setSearch] = useState("");
  const [duplicateOnly, setDuplicateOnly] = useState(true);
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(false);
  const [maintenance, setMaintenance] = useState(false);
  const [cacheConfirmed, setCacheConfirmed] = useState(false);
  const [message, setMessage] = useState("");

  useEffect(() => {
    void refreshStorage();
  }, []);

  useEffect(() => {
    setOffset(0);
    setSearch("");
    setDuplicateOnly(true);
    if (!projectId.trim()) {
      setHistory(null);
      return;
    }
    setLoading(true);
    void backend
      .listFileHistory(projectId.trim(), "", true, 0, pageSize)
      .then(setHistory)
      .catch((error: unknown) => setMessage(String(error)))
      .finally(() => setLoading(false));
  }, [projectId]);

  async function loadHistory(nextOffset = offset) {
    if (!projectId.trim()) return;
    setLoading(true);
    try {
      const result = await backend.listFileHistory(
        projectId.trim(),
        search.trim(),
        duplicateOnly,
        nextOffset,
        pageSize,
      );
      setHistory(result);
      setOffset(nextOffset);
      setMessage(`Đã tải ${result.items.length.toLocaleString("vi-VN")} mục từ lịch sử bền vững.`);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setLoading(false);
    }
  }

  async function refreshStorage() {
    try {
      setStorage(await backend.storageOverview());
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function optimizeDatabase() {
    setMaintenance(true);
    setMessage("Đang checkpoint WAL, tối ưu và nén cơ sở dữ liệu…");
    try {
      const result = await backend.optimizeStorage();
      setMessage(
        `Đã tối ưu cơ sở dữ liệu, thu hồi ${formatBytes(result.reclaimed_bytes)}. Không xóa lịch sử hoặc tệp.`,
      );
      await refreshStorage();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setMaintenance(false);
    }
  }

  async function cleanupLogs() {
    setMaintenance(true);
    try {
      const result = await backend.cleanupOldLogs(30);
      setMessage(
        `Đã xóa ${result.deleted_files.toLocaleString("vi-VN")} tệp log cũ hơn 30 ngày, thu hồi ${formatBytes(result.reclaimed_bytes)}.`,
      );
      await refreshStorage();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setMaintenance(false);
    }
  }

  async function clearInterfaceCache() {
    if (!cacheConfirmed) return;
    setMaintenance(true);
    try {
      await backend.clearInterfaceCache();
      setCacheConfirmed(false);
      setMessage(
        "Đã yêu cầu dọn cache giao diện. Lịch sử SQLite, tệp nguồn và vùng cách ly không bị thay đổi.",
      );
      await refreshStorage();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setMaintenance(false);
    }
  }

  const first = history && history.total_matching > 0 ? offset + 1 : 0;
  const last = history ? Math.min(offset + history.items.length, history.total_matching) : 0;

  return (
    <div className="stack history-page">
      <section className="hero-card history-hero">
        <div>
          <p className="eyebrow">BẰNG CHỨNG BỀN VỮNG · KHÔNG NHÂN ĐÔI LOG</p>
          <h2>Lịch sử tệp và vị trí trùng</h2>
          <p>
            Mỗi dòng được dựng trực tiếp từ snapshot, nhóm trùng, kế hoạch và giao dịch trong
            SQLite. Nội dung tệp không bao giờ được lưu vào lịch sử.
          </p>
        </div>
        <div className="shield">≣</div>
      </section>

      <section className="card">
        <div className="section-heading">
          <div>
            <p className="eyebrow">DỮ LIỆU ỨNG DỤNG</p>
            <h2>Dọn dẹp an toàn</h2>
          </div>
          <button
            className="secondary"
            disabled={maintenance}
            onClick={() => void refreshStorage()}
          >
            Đo lại dung lượng
          </button>
        </div>
        {storage ? (
          <>
            <div className="storage-metrics">
              <StorageMetric label="Tổng dữ liệu ứng dụng" value={storage.total_bytes} />
              <StorageMetric label="SQLite + WAL" value={storage.database_bytes} />
              <StorageMetric label="Cache giao diện" value={storage.interface_cache_bytes} />
              <StorageMetric label="Log chẩn đoán" value={storage.log_bytes} />
              <StorageMetric label="Nhật ký giao dịch" value={storage.manifest_bytes} />
            </div>
            <p className="storage-path">Thư mục được đo: {storage.data_directory}</p>
          </>
        ) : (
          <div className="empty">Đang đo dữ liệu cục bộ…</div>
        )}
        <div className="maintenance-actions">
          <button
            className="primary"
            disabled={maintenance}
            onClick={() => void optimizeDatabase()}
          >
            Tối ưu SQLite và dọn WAL
          </button>
          <button className="secondary" disabled={maintenance} onClick={() => void cleanupLogs()}>
            Xóa log cũ hơn 30 ngày
          </button>
          <div className="cache-cleanup">
            <label>
              <input
                type="checkbox"
                checked={cacheConfirmed}
                onChange={(event) => setCacheConfirmed(event.target.checked)}
              />
              Tôi hiểu cache giao diện và tùy chọn tạm thời sẽ được đặt lại
            </label>
            <button
              className="secondary"
              disabled={maintenance || !cacheConfirmed}
              onClick={() => void clearInterfaceCache()}
            >
              Dọn cache giao diện
            </button>
          </div>
        </div>
        <p className="maintenance-scope">
          Các nút trên không xóa lịch sử SQLite, tệp nguồn, vùng cách ly hoặc cache biên dịch Rust
          trong thư mục mã nguồn.
        </p>
      </section>

      <section className="card">
        <div className="history-toolbar">
          <label>
            Mã dự án
            <input value={projectId} onChange={(event) => onProjectChange(event.target.value)} />
          </label>
          <label>
            Tìm đường dẫn hoặc mã nhóm
            <input
              value={search}
              placeholder="Ví dụ: Ebooks hoặc mã nhóm trùng"
              onChange={(event) => setSearch(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void loadHistory(0);
              }}
            />
          </label>
          <label className="history-checkbox">
            <input
              type="checkbox"
              checked={duplicateOnly}
              onChange={(event) => setDuplicateOnly(event.target.checked)}
            />
            Chỉ hiện tệp thuộc nhóm trùng
          </label>
          <button
            className="secondary"
            disabled={!projectId.trim() || loading}
            onClick={() => void loadHistory(0)}
          >
            {loading ? "Đang tải…" : "Tra cứu lịch sử"}
          </button>
        </div>

        {history && (
          <div className="history-summary">
            <SummaryMetric label="Snapshot đã xử lý" value={history.total_processed} />
            <SummaryMetric label="Tệp trong nhóm trùng" value={history.duplicate_files} />
            <SummaryMetric label="Nhóm trùng" value={history.duplicate_groups} />
            <SummaryMetric label="Tệp có vấn đề" value={history.problem_files} />
          </div>
        )}

        <div className="history-list">
          {history?.items.map((item) => (
            <article key={item.snapshot_id}>
              <div className="history-record-heading">
                <span className={`badge ${item.group_id ? "success" : ""}`}>
                  {item.group_id
                    ? actionLabel(item.action ?? "manual")
                    : fileStateLabel(item.state)}
                </span>
                <strong>{item.path}</strong>
              </div>
              <div className="history-record-meta">
                <span>{formatBytes(item.size_bytes)}</span>
                <span>Ghi nhận: {formatDate(item.observed_at)}</span>
                <span>Phiên: {item.session_id}</span>
                <span>Truy cập: {accessLabel(item.access_status)}</span>
              </div>
              {item.group_id && (
                <div className="duplicate-evidence">
                  <p>
                    <strong>Nhóm trùng:</strong> {item.group_id} · {reasonLabel(item.reason ?? "")}
                    {item.plan_status ? ` · kế hoạch ${planLabel(item.plan_status)}` : ""}
                  </p>
                  <details>
                    <summary>
                      Xem {item.duplicate_locations.length.toLocaleString("vi-VN")} vị trí trùng còn
                      lại
                    </summary>
                    <ul>
                      {item.duplicate_locations.map((path) => (
                        <li key={path}>{path}</li>
                      ))}
                    </ul>
                  </details>
                </div>
              )}
              {(item.transaction_status || item.quarantine_path) && (
                <div className="transaction-evidence">
                  <span>
                    Giao dịch: {quarantineStateLabel(item.transaction_status ?? "planned")}
                  </span>
                  {item.quarantine_path && <span>Đích cách ly: {item.quarantine_path}</span>}
                  {item.quarantine_state && (
                    <span>Hiện tại: {quarantineStateLabel(item.quarantine_state)}</span>
                  )}
                </div>
              )}
            </article>
          ))}
          {history && history.items.length === 0 && (
            <div className="empty">Không có lịch sử nào khớp bộ lọc hiện tại.</div>
          )}
          {!history && <div className="empty">Chọn dự án rồi tải lịch sử để xem chi tiết.</div>}
        </div>

        {history && (
          <div className="pager history-pager">
            <button
              className="secondary"
              disabled={loading || offset === 0}
              onClick={() => void loadHistory(Math.max(0, offset - pageSize))}
            >
              Trang trước
            </button>
            <span>
              Hiển thị {first.toLocaleString("vi-VN")}–{last.toLocaleString("vi-VN")}/
              {history.total_matching.toLocaleString("vi-VN")} mục
            </span>
            <button
              className="secondary"
              disabled={loading || offset + history.items.length >= history.total_matching}
              onClick={() => void loadHistory(offset + pageSize)}
            >
              Trang sau
            </button>
          </div>
        )}
      </section>
      {message && <div className="notice">{message}</div>}
    </div>
  );
}

function StorageMetric({ label, value }: { label: string; value: number }) {
  return (
    <div>
      <strong>{formatBytes(value)}</strong>
      <span>{label}</span>
    </div>
  );
}

function SummaryMetric({ label, value }: { label: string; value: number }) {
  return (
    <div>
      <strong>{value.toLocaleString("vi-VN")}</strong>
      <span>{label}</span>
    </div>
  );
}

function formatBytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toLocaleString("vi-VN", {
    maximumFractionDigits: index === 0 ? 0 : 1,
  })} ${units[index]}`;
}

function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString("vi-VN");
}

function fileStateLabel(state: string) {
  const labels: Record<string, string> = {
    discovered: "Đã phát hiện",
    unique: "Không trùng",
    skipped: "Đã bỏ qua",
    duplicate_confirmed: "Đã chứng minh trùng",
    planned_keep: "Đã chọn giữ",
    planned_quarantine: "Đã chọn cách ly",
    quarantined_verified: "Đã cách ly và xác minh",
    restored_verified: "Đã khôi phục và xác minh",
    unstable: "Không ổn định",
    error: "Lỗi",
  };
  return labels[state] ?? state;
}

function accessLabel(status: string) {
  const labels: Record<string, string> = {
    readable: "Đọc được",
    locked: "Bị khóa",
    denied: "Bị từ chối",
    offline: "Ngoại tuyến",
    missing: "Không còn tồn tại",
    error: "Lỗi",
  };
  return labels[status] ?? status;
}

function planLabel(status: string) {
  const labels: Record<string, string> = {
    draft: "bản nháp",
    sealed: "đã khóa",
    executing: "đang thực thi",
    completed: "đã hoàn tất",
    stale: "đã cũ",
    cancelled: "đã hủy",
  };
  return labels[status] ?? status;
}
