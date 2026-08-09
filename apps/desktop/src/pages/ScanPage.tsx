import { useEffect, useState } from "react";
import { backend } from "../services/backend";
import { canStartScan, isFinalScanState, requiresDurableResume } from "../services/safety";
import type { Mode, ScanSession } from "../types";
import { scanStateLabel } from "../services/labels";

interface Props {
  projectId: string;
  sessionId: string;
  onProjectChange: (id: string) => void;
  onSession: (id: string) => void;
  onReview: () => void;
}

export function ScanPage({ projectId, sessionId, onProjectChange, onSession, onReview }: Props) {
  const [mode, setMode] = useState<Mode>("strict");
  const [acknowledged, setAcknowledged] = useState(false);
  const [allFiles, setAllFiles] = useState(false);
  const [status, setStatus] = useState<ScanSession | null>(null);
  const [message, setMessage] = useState("");

  useEffect(() => {
    if (!projectId) return;
    void backend
      .listProjects()
      .then((projects) => {
        const project = projects.find((candidate) => candidate.id === projectId);
        if (project) setMode(project.mode);
      })
      .catch((error: unknown) => setMessage(String(error)));
  }, [projectId]);

  useEffect(() => {
    if (!sessionId || isFinalScanState(status?.state ?? "")) return;
    const eventTimer = window.setInterval(() => {
      void backend
        .nextScanEvent(sessionId)
        .then((event) => {
          if (!event) return;
          setStatus((current) => (current ? { ...current, ...event.progress } : current));
        })
        .catch((error: unknown) => setMessage(String(error)));
    }, 250);
    const timer = window.setInterval(() => {
      void backend
        .scanStatus(sessionId)
        .then(setStatus)
        .catch((error: unknown) => setMessage(String(error)));
    }, 800);
    return () => {
      window.clearInterval(eventTimer);
      window.clearInterval(timer);
    };
  }, [sessionId, status?.state]);

  async function start() {
    try {
      const session = await backend.startScan(projectId, mode, acknowledged, allFiles);
      onSession(session);
      setStatus(await backend.scanStatus(session));
      setMessage("Đã bắt đầu quét trong nền. Các tệp nguồn vẫn ở chế độ chỉ đọc.");
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function control(action: "pause" | "resume" | "cancel") {
    try {
      await backend.controlScan(sessionId, action);
      setStatus(await backend.scanStatus(sessionId));
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function resume() {
    try {
      if (requiresDurableResume(status?.state ?? "")) {
        await backend.resumeScan(sessionId);
        setMessage("Đã khởi động lại phiên bị gián đoạn từ mốc chỉ đọc bền vững.");
      } else {
        await backend.controlScan(sessionId, "resume");
      }
      setStatus(await backend.scanStatus(sessionId));
    } catch (error) {
      setMessage(String(error));
    }
  }

  const progress = status?.discovered_files
    ? Math.min(100, Math.round((status.processed_files / status.discovered_files) * 100))
    : 0;
  const startedAt = status?.started_at ? Date.parse(status.started_at) : Number.NaN;
  const endedAt = status?.finished_at ? Date.parse(status.finished_at) : Date.now();
  const elapsedSeconds = Number.isFinite(startedAt)
    ? Math.max(0, Math.round((endedAt - startedAt) / 1000))
    : 0;
  const readRate = elapsedSeconds > 0 ? (status?.bytes_read ?? 0) / elapsedSeconds : 0;
  const remainingSeconds =
    status && status.processed_files > 0 && status.discovered_files > status.processed_files
      ? Math.round(
          (elapsedSeconds * (status.discovered_files - status.processed_files)) /
            status.processed_files,
        )
      : 0;

  return (
    <div className="stack">
      <section className="hero-card">
        <div>
          <p className="eyebrow">QUY TRÌNH CHỈ ĐỌC</p>
          <h2>Siêu dữ liệu → loại nhanh bằng mẫu → BLAKE3 → SHA-256</h2>
          <p>
            Mẫu băm trùng nhau không bao giờ là bằng chứng đủ. Chỉ các tệp ổn định, được lưu độc lập
            và khớp cả hai giá trị băm đầy đủ mới xuất hiện trong Kết quả.
          </p>
        </div>
        <div className="shield">✓</div>
      </section>
      <div className="content-grid two-column">
        <section className="card">
          <h2>Cấu hình quét</h2>
          <label>
            Mã dự án
            <input value={projectId} onChange={(event) => onProjectChange(event.target.value)} />
          </label>
          <label>
            Chế độ
            <select value={mode} onChange={(event) => setMode(event.target.value as Mode)}>
              <option value="strict">Nghiêm ngặt (khuyến nghị)</option>
              <option value="content">Chỉ so sánh nội dung</option>
            </select>
          </label>
          {mode === "content" && (
            <label className="warning-box">
              <input
                type="checkbox"
                checked={acknowledged}
                onChange={(event) => setAcknowledged(event.target.checked)}
              />
              Tôi hiểu rằng chế độ chỉ so sánh nội dung có thể nhóm các tệp khác tên.
            </label>
          )}
          <label className="inline-check">
            <input
              type="checkbox"
              checked={allFiles}
              onChange={(event) => setAllFiles(event.target.checked)}
            />
            Quét mọi phần mở rộng trong lần này (ghi đè danh sách đã lưu)
          </label>
          <button
            className="primary"
            disabled={!canStartScan(projectId, mode, acknowledged)}
            onClick={() => void start()}
          >
            Bắt đầu quét chỉ đọc
          </button>
        </section>
        <section className="card scan-monitor">
          <div className="card-heading">
            <h2>Phiên đang chạy</h2>
            <span className={`badge ${status?.state === "completed" ? "safe" : ""}`}>
              {scanStateLabel(status?.state ?? "idle")}
            </span>
          </div>
          <code>{sessionId || "Chưa có phiên quét"}</code>
          <div className="progress-track" aria-label={`${progress}%`}>
            <div style={{ width: `${progress}%` }} />
          </div>
          <div className="metrics">
            <Metric label="Đã phát hiện" value={status?.discovered_files ?? 0} />
            <Metric label="Đã xử lý" value={status?.processed_files ?? 0} />
            <Metric label="Lỗi" value={status?.errors ?? 0} />
            <Metric label="Nhóm trùng" value={status?.duplicate_groups ?? 0} />
            <Metric label="Đã bỏ qua" value={status?.skipped ?? 0} />
            <Metric label="Không ổn định" value={status?.unstable ?? 0} />
            <Metric label="Đã đọc" value={formatBytes(status?.bytes_read ?? 0)} />
            <Metric label="Tốc độ đọc" value={`${formatBytes(readRate)}/giây`} />
          </div>
          <p className="scan-timing">
            Đã chạy: {formatDuration(elapsedSeconds)} · Còn khoảng:{" "}
            {formatDuration(remainingSeconds)} (chỉ là ước tính) · Có thể thu hồi đã xác minh:{" "}
            {formatBytes(status?.reclaimable_bytes ?? 0)}
          </p>
          {status?.state === "blocked" && status.blocked_reason && (
            <div className="warning-box scan-block-reason" role="alert">
              <strong>Nguyên nhân bị chặn</strong>
              <span>{status.blocked_reason}</span>
            </div>
          )}
          <div className="button-row">
            <button
              className="secondary"
              disabled={!sessionId}
              onClick={() => void control("pause")}
            >
              Tạm dừng
            </button>
            <button className="secondary" disabled={!sessionId} onClick={() => void resume()}>
              {requiresDurableResume(status?.state ?? "") ? "Khởi động lại an toàn" : "Tiếp tục"}
            </button>
            <button
              className="danger-quiet"
              disabled={!sessionId}
              onClick={() => void control("cancel")}
            >
              Hủy
            </button>
          </div>
          <button className="primary" disabled={status?.state !== "completed"} onClick={onReview}>
            Xem kết quả đã chứng minh
          </button>
        </section>
      </div>
      {message && <div className="notice">{message}</div>}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: number | string }) {
  return (
    <div>
      <strong>{value.toLocaleString()}</strong>
      <span>{label}</span>
    </div>
  );
}

function formatBytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(units.length - 1, Math.floor(Math.log(value) / Math.log(1024)));
  return `${(value / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatDuration(seconds: number) {
  if (!seconds) return "—";
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return hours > 0 ? `${hours} giờ ${minutes} phút` : `${minutes} phút ${remainder} giây`;
}
