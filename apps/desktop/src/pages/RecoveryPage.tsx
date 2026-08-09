import { useState } from "react";
import { backend } from "../services/backend";
import type { RecoveryTransaction } from "../types";
import { quarantineStateLabel } from "../services/labels";

interface Props {
  projectId: string;
  onProjectChange: (id: string) => void;
}

export function RecoveryPage({ projectId, onProjectChange }: Props) {
  const [transactions, setTransactions] = useState<RecoveryTransaction[]>([]);
  const [tokens, setTokens] = useState<Record<string, string>>({});
  const [message, setMessage] = useState("");

  async function inspect() {
    try {
      setTransactions(await backend.inspectRecovery(projectId));
      setMessage("Kiểm tra chỉ đọc. Đối soát luôn xác minh, không bao giờ di chuyển mù quáng.");
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function reconcile(id: string) {
    try {
      const outcome = await backend.reconcile(id, tokens[id] ?? "");
      setMessage(`Kết quả đối soát: ${outcome}`);
      await inspect();
    } catch (error) {
      setMessage(String(error));
    }
  }

  return (
    <div className="stack">
      <section className="hero-card recovery-hero">
        <div>
          <p className="eyebrow">AN TOÀN KHI KHỞI ĐỘNG</p>
          <h2>Quan sát trước, đối soát có chủ đích</h2>
          <p>
            Nếu tiến trình dừng giữa giao dịch, bộ máy sẽ kiểm tra thực tế ở nguồn và đích, đồng
            thời giữ nguyên các trường hợp chưa rõ để bạn xem xét thủ công.
          </p>
        </div>
        <div className="shield">↺</div>
      </section>
      <section className="card">
        <div className="toolbar-card">
          <label>
            Mã dự án
            <input value={projectId} onChange={(event) => onProjectChange(event.target.value)} />
          </label>
          <button className="secondary" disabled={!projectId} onClick={() => void inspect()}>
            Kiểm tra giao dịch bị gián đoạn
          </button>
        </div>
        <div className="inventory">
          {transactions.map((transaction) => (
            <article key={transaction.id}>
              <div className="inventory-main">
                <span className="badge warning">{quarantineStateLabel(transaction.state)}</span>
                <strong>{transaction.source}</strong>
                <small>Đích: {transaction.destination}</small>
              </div>
              <div className="restore-control">
                <input
                  placeholder="Nhập RECONCILE"
                  value={tokens[transaction.id] ?? ""}
                  onChange={(event) =>
                    setTokens((current) => ({ ...current, [transaction.id]: event.target.value }))
                  }
                />
                <button
                  className="secondary"
                  disabled={tokens[transaction.id] !== "RECONCILE"}
                  onClick={() => void reconcile(transaction.id)}
                >
                  Đối soát
                </button>
              </div>
            </article>
          ))}
          {!transactions.length && (
            <div className="empty">Chưa tải giao dịch bị gián đoạn nào.</div>
          )}
        </div>
      </section>
      {message && <div className="notice">{message}</div>}
    </div>
  );
}
