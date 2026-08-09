import { useState } from "react";
import { backend } from "../services/backend";
import type { PermanentDeleteChallenge, PermanentDeleteOutcome, QuarantineEntry } from "../types";

interface Props {
  entries: QuarantineEntry[];
  deleteNow: boolean;
  onDeleted: (outcome: PermanentDeleteOutcome) => Promise<void>;
}

export function PermanentDeleteDialog({ entries, deleteNow, onDeleted }: Props) {
  const [challenge, setChallenge] = useState<PermanentDeleteChallenge | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const totalBytes = entries.reduce((total, entry) => total + entry.size_bytes, 0);
  const displayedCount = challenge?.entry_count ?? entries.length;
  const displayedBytes = challenge?.total_bytes ?? totalBytes;

  async function prepare() {
    setBusy(true);
    setMessage("");
    try {
      const prepared = await backend.preparePermanentDelete(
        entries.map((entry) => entry.id),
        deleteNow,
      );
      const expectedMode = deleteNow ? "immediate" : "retention_expired";
      if (prepared.mode !== expectedMode) {
        throw new Error("Chế độ của thử thách xóa không khớp lựa chọn hiện tại.");
      }
      setChallenge(prepared);
      setConfirmed(false);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function execute() {
    if (!challenge) return;
    setBusy(true);
    setMessage("");
    try {
      const outcome = await backend.executePermanentDelete(
        challenge.batch_id,
        challenge.token,
        challenge.confirmation_phrase,
      );
      await onDeleted(outcome);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="card permanent-delete-dialog" aria-label="Xóa vĩnh viễn">
      <div>
        <p className="eyebrow">
          {deleteNow
            ? "XÓA NGAY · KHÔNG THỂ HOÀN TÁC · CHỈ TRONG VÙNG CÁCH LY"
            : "KHÔNG THỂ HOÀN TÁC · CHỈ TRONG VÙNG CÁCH LY"}
        </p>
        <h2>
          {deleteNow ? "Xóa ngay vĩnh viễn các mục đã chọn" : "Xóa vĩnh viễn các mục đã chọn"}
        </h2>
        <p>
          {deleteNow && "Thời gian giữ 30 ngày sẽ bị bỏ qua cho đúng lô này. "}
          Bạn có thể chọn riêng từng tệp hoặc chọn tất cả mục đủ điều kiện đang hiển thị. Bước chuẩn
          bị không xóa dữ liệu. Trước khi xóa thật, bạn chỉ cần đánh dấu một ô xác nhận rõ ràng; ứng
          dụng tự gửi token và câu kỹ thuật cho backend kiểm tra.
        </p>
      </div>
      <div className="delete-totals" aria-label="Tổng số cần xóa vĩnh viễn">
        <strong>{displayedCount.toLocaleString("vi-VN")} tệp</strong>
        <strong>{displayedBytes.toLocaleString("vi-VN")} byte</strong>
      </div>
      {!challenge ? (
        <button
          className="danger"
          disabled={!entries.length || busy}
          onClick={() => void prepare()}
        >
          {deleteNow ? "Chuẩn bị thử thách Xóa ngay" : "Chuẩn bị thử thách xóa"}
        </button>
      ) : (
        <div className="delete-challenge">
          <p>
            Xác nhận hết hạn lúc <strong>{challenge.expires_at}</strong>. Khi bắt đầu, ứng dụng sẽ
            kiểm tra lại danh tính, kích thước, BLAKE3 và SHA-256 của toàn bộ mục trước lần xóa đầu
            tiên; quá trình này có thể mất vài phút.
          </p>
          <label className="delete-final-confirmation">
            <input
              type="checkbox"
              aria-label={`Tôi xác nhận xóa vĩnh viễn đúng ${challenge.entry_count} tệp`}
              checked={confirmed}
              onChange={(event) => setConfirmed(event.target.checked)}
            />
            <span>
              <strong>
                Tôi hiểu và xác nhận xóa vĩnh viễn đúng{" "}
                {challenge.entry_count.toLocaleString("vi-VN")} tệp
              </strong>
              <small>
                Thao tác này không thể hoàn tác và sẽ giải phóng tối đa{" "}
                {challenge.total_bytes.toLocaleString("vi-VN")} byte.
              </small>
            </span>
          </label>
          <button className="danger" disabled={busy || !confirmed} onClick={() => void execute()}>
            {busy
              ? "Đang kiểm tra lại và xóa…"
              : `Xóa vĩnh viễn đúng ${challenge.entry_count.toLocaleString("vi-VN")} tệp`}
          </button>
          <button
            className="secondary"
            disabled={busy}
            onClick={() => {
              setChallenge(null);
              setConfirmed(false);
              setMessage("");
            }}
          >
            Tạo lại xác nhận
          </button>
        </div>
      )}
      {message && <div className="notice">{message}</div>}
    </section>
  );
}
