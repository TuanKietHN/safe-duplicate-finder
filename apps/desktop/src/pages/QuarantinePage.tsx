import { useEffect, useState } from "react";
import { backend } from "../services/backend";
import { exactConfirmation } from "../services/safety";
import type { PermanentDeleteOutcome, QuarantineEntry } from "../types";
import { PermanentDeleteDialog } from "../components/PermanentDeleteDialog";
import { quarantineStateLabel } from "../services/labels";

interface Props {
  projectId: string;
  sessionId?: string;
  planId: string;
  onProjectChange: (id: string) => void;
  onPlanChange: (id: string) => void;
}

export function QuarantinePage({
  projectId,
  sessionId = "",
  planId,
  onProjectChange,
  onPlanChange,
}: Props) {
  const [confirmation, setConfirmation] = useState("");
  const [entries, setEntries] = useState<QuarantineEntry[]>([]);
  const [restoreTokens, setRestoreTokens] = useState<Record<string, string>>({});
  const [search, setSearch] = useState("");
  const [stateFilter, setStateFilter] = useState("all");
  const [quarantinedAfter, setQuarantinedAfter] = useState("");
  const [message, setMessage] = useState("");
  const [deleteSelection, setDeleteSelection] = useState<Record<string, boolean>>({});
  const [deleteNow, setDeleteNow] = useState(false);
  const visibleEntries = entries.filter((entry) => {
    const query = search.trim().toLocaleLowerCase();
    const matchesSearch =
      !query ||
      entry.original_path.toLocaleLowerCase().includes(query) ||
      entry.quarantine_path.toLocaleLowerCase().includes(query);
    const matchesDate =
      !quarantinedAfter || Date.parse(entry.quarantined_at) >= Date.parse(quarantinedAfter);
    return matchesSearch && matchesDate && (stateFilter === "all" || entry.state === stateFilter);
  });
  const selectedForDelete = entries.filter(
    (entry) => deleteSelection[entry.id] && deleteEligible(entry, deleteNow),
  );
  const eligibleVisibleEntries = visibleEntries.filter((entry) => deleteEligible(entry, deleteNow));
  const selectedVisibleCount = eligibleVisibleEntries.filter(
    (entry) => deleteSelection[entry.id],
  ).length;
  const allEligibleVisibleSelected =
    eligibleVisibleEntries.length > 0 && selectedVisibleCount === eligibleVisibleEntries.length;

  useEffect(() => {
    const requestedSession = sessionId.trim();
    if (!requestedSession || planId) return;
    void backend
      .latestPlanForSession(requestedSession)
      .then((id) => {
        if (id) onPlanChange(id);
      })
      .catch((error: unknown) => setMessage(String(error)));
  }, [onPlanChange, planId, sessionId]);

  async function load() {
    try {
      const loaded = await backend.listQuarantine(projectId);
      setEntries(loaded);
      setDeleteSelection((current) =>
        Object.fromEntries(
          loaded
            .filter((entry) => current[entry.id] && deleteEligible(entry, deleteNow))
            .map((entry) => [entry.id, true]),
        ),
      );
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function deleted(outcome: PermanentDeleteOutcome) {
    setMessage(
      `Đã xóa vĩnh viễn ${outcome.deleted_entries.toLocaleString("vi-VN")} mục cách ly (${outcome.deleted_bytes.toLocaleString("vi-VN")} byte).`,
    );
    setDeleteSelection({});
    await load();
  }

  async function apply() {
    try {
      const bytes = await backend.applyQuarantine(planId, confirmation);
      setMessage(
        `Đã di chuyển và xác minh ${bytes.toLocaleString("vi-VN")} byte. Không có dữ liệu nào bị xóa.`,
      );
      await load();
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function restore(entry: QuarantineEntry) {
    try {
      await backend.restore(entry.id, restoreTokens[entry.id] ?? "");
      setMessage(`Đã xác minh khôi phục: ${entry.original_path}`);
      await load();
    } catch (error) {
      setMessage(String(error));
    }
  }

  function changeDeleteNow(enabled: boolean) {
    setDeleteNow(enabled);
    setDeleteSelection((current) =>
      Object.fromEntries(
        entries
          .filter((entry) => current[entry.id] && deleteEligible(entry, enabled))
          .map((entry) => [entry.id, true]),
      ),
    );
  }

  function selectAllVisibleForDelete() {
    setDeleteSelection((current) => ({
      ...current,
      ...Object.fromEntries(eligibleVisibleEntries.map((entry) => [entry.id, true])),
    }));
  }

  function clearDeleteSelection() {
    setDeleteSelection({});
  }

  return (
    <div className="stack">
      <section className="card quarantine-gate">
        <div>
          <p className="eyebrow">DI CHUYỂN CÓ THỂ KHÔI PHỤC</p>
          <h2>Áp dụng một kế hoạch đã khóa</h2>
          <p>
            Mỗi tệp được kiểm tra lại, di chuyển trong cùng ổ đĩa mà không ghi đè, rồi được băm đầy
            đủ tại đích. Số byte chỉ được tính sau khi xác minh.
          </p>
        </div>
        <label>
          Mã kế hoạch
          <input value={planId} onChange={(event) => onPlanChange(event.target.value)} />
        </label>
        <label>
          Nhập chính xác QUARANTINE
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            autoComplete="off"
          />
        </label>
        <button
          className="danger"
          disabled={!planId || !exactConfirmation(confirmation, "QUARANTINE")}
          onClick={() => void apply()}
        >
          Chuyển các bản sao đã xem xét vào vùng cách ly
        </button>
      </section>
      <section className="card">
        <div className="card-heading">
          <h2>Danh sách trong vùng cách ly</h2>
          <button className="secondary" onClick={() => void load()} disabled={!projectId}>
            Làm mới
          </button>
        </div>
        <label>
          Mã dự án
          <input value={projectId} onChange={(event) => onProjectChange(event.target.value)} />
        </label>
        <div className="inventory-filters">
          <label>
            Tìm theo đường dẫn gốc hoặc đường dẫn cách ly
            <input value={search} onChange={(event) => setSearch(event.target.value)} />
          </label>
          <label>
            Trạng thái
            <select value={stateFilter} onChange={(event) => setStateFilter(event.target.value)}>
              <option value="all">Tất cả trạng thái</option>
              <option value="verified">Đã xác minh</option>
              <option value="restoring">Đang khôi phục</option>
              <option value="restored">Đã khôi phục</option>
              <option value="recovery_required">Cần phục hồi</option>
              <option value="deleting">Đang xóa / cần phục hồi</option>
              <option value="failed">Xóa thất bại</option>
              <option value="deleted">Đã xóa vĩnh viễn</option>
            </select>
          </label>
          <label>
            Được cách ly từ ngày
            <input
              type="date"
              value={quarantinedAfter}
              onChange={(event) => setQuarantinedAfter(event.target.value)}
            />
          </label>
        </div>
        <div className={deleteNow ? "immediate-delete-mode active" : "immediate-delete-mode"}>
          <label>
            <input
              type="checkbox"
              aria-label="Bật chế độ Xóa ngay"
              checked={deleteNow}
              onChange={(event) => changeDeleteNow(event.target.checked)}
            />
            <strong>Xóa ngay — bỏ qua thời gian giữ 30 ngày</strong>
          </label>
          <p>
            Chỉ bỏ qua ngày lưu giữ. Bạn phải chủ động chọn tệp, chuẩn bị xác nhận ngắn hạn rồi đánh
            dấu ô xác nhận cuối cùng trước khi xóa.
          </p>
        </div>
        <div className="bulk-delete-selection">
          <div>
            <strong>
              Đã chọn {selectedForDelete.length.toLocaleString("vi-VN")} mục để xóa vĩnh viễn
            </strong>
            <small>
              Có {eligibleVisibleEntries.length.toLocaleString("vi-VN")} mục đủ điều kiện trong kết
              quả đang hiển thị.
            </small>
          </div>
          <button
            className="secondary"
            disabled={!eligibleVisibleEntries.length || allEligibleVisibleSelected}
            onClick={selectAllVisibleForDelete}
          >
            {allEligibleVisibleSelected
              ? `Đã chọn tất cả ${eligibleVisibleEntries.length.toLocaleString("vi-VN")} mục`
              : `Chọn tất cả ${eligibleVisibleEntries.length.toLocaleString("vi-VN")} mục đủ điều kiện`}
          </button>
          <button
            className="secondary"
            disabled={!selectedForDelete.length}
            onClick={clearDeleteSelection}
          >
            Bỏ chọn tất cả
          </button>
        </div>
        <div className="inventory">
          {visibleEntries.map((entry) => (
            <article key={entry.id}>
              <div className="inventory-main">
                <span className={`badge ${entry.state === "verified" ? "safe" : ""}`}>
                  {quarantineStateLabel(entry.state)}
                </span>
                <strong>{entry.original_path}</strong>
                <small>Vùng cách ly: {entry.quarantine_path}</small>
                <small>
                  {entry.size_bytes.toLocaleString("vi-VN")} byte · cách ly lúc{" "}
                  {entry.quarantined_at} · giữ đến {entry.retain_until}
                </small>
              </div>
              {entry.state === "verified" && (
                <div className="inventory-actions">
                  <div className="restore-control">
                    <input
                      aria-label={`Xác nhận khôi phục ${entry.original_path}`}
                      placeholder="Nhập RESTORE"
                      value={restoreTokens[entry.id] ?? ""}
                      onChange={(event) =>
                        setRestoreTokens((current) => ({
                          ...current,
                          [entry.id]: event.target.value,
                        }))
                      }
                    />
                    <button
                      className="secondary"
                      disabled={!exactConfirmation(restoreTokens[entry.id] ?? "", "RESTORE")}
                      onClick={() => void restore(entry)}
                    >
                      Khôi phục
                    </button>
                  </div>
                  {deleteEligible(entry, deleteNow) && (
                    <label className="delete-selection">
                      <input
                        type="checkbox"
                        aria-label={`Chọn ${entry.original_path} để xóa vĩnh viễn`}
                        checked={Boolean(deleteSelection[entry.id])}
                        onChange={(event) =>
                          setDeleteSelection((current) => ({
                            ...current,
                            [entry.id]: event.target.checked,
                          }))
                        }
                      />
                      {deleteNow
                        ? "Chọn mục này để xóa ngay vĩnh viễn"
                        : "Chọn mục này để xóa vĩnh viễn"}
                    </label>
                  )}
                </div>
              )}
            </article>
          ))}
          {!visibleEntries.length && (
            <div className="empty">Không có mục cách ly nào khớp với bộ lọc hiện tại.</div>
          )}
        </div>
      </section>
      <PermanentDeleteDialog
        key={
          selectedForDelete
            .map((entry) => entry.id)
            .sort()
            .join(":") + `:${deleteNow ? "immediate" : "retained"}`
        }
        entries={selectedForDelete}
        deleteNow={deleteNow}
        onDeleted={deleted}
      />
      {message && <div className="notice">{message}</div>}
    </div>
  );
}

function deleteEligible(entry: QuarantineEntry, deleteNow: boolean) {
  return (
    entry.state === "verified" &&
    entry.permanent_delete_state === "active" &&
    (deleteNow ||
      (Number.isFinite(Date.parse(entry.retain_until)) &&
        Date.parse(entry.retain_until) <= Date.now()))
  );
}
