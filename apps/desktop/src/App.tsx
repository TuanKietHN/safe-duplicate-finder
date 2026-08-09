import { useEffect, useState } from "react";
import { backend } from "./services/backend";
import type { Page } from "./types";
import { ProjectsPage } from "./pages/ProjectsPage";
import { ScanPage } from "./pages/ScanPage";
import { ResultsPage } from "./pages/ResultsPage";
import { QuarantinePage } from "./pages/QuarantinePage";
import { RecoveryPage } from "./pages/RecoveryPage";
import { HistoryPage } from "./pages/HistoryPage";

const navigation: Array<{ id: Page; label: string; icon: string }> = [
  { id: "projects", label: "Dự án và thư mục", icon: "▦" },
  { id: "scan", label: "Quét tệp", icon: "◉" },
  { id: "results", label: "Kết quả và kế hoạch", icon: "≡" },
  { id: "quarantine", label: "Vùng cách ly", icon: "◇" },
  { id: "recovery", label: "Phục hồi", icon: "↺" },
  { id: "history", label: "Lịch sử và dọn dẹp", icon: "◷" },
];

const workflowStorageKey = "safe-dedupe.workflow.v1";

interface StoredWorkflow {
  projectId: string;
  sessionId: string;
  planId: string;
}

function loadWorkflow(): StoredWorkflow {
  try {
    const value = window.localStorage.getItem(workflowStorageKey);
    if (!value) return { projectId: "", sessionId: "", planId: "" };
    const parsed = JSON.parse(value) as Partial<StoredWorkflow>;
    return {
      projectId: typeof parsed.projectId === "string" ? parsed.projectId : "",
      sessionId: typeof parsed.sessionId === "string" ? parsed.sessionId : "",
      planId: typeof parsed.planId === "string" ? parsed.planId : "",
    };
  } catch {
    return { projectId: "", sessionId: "", planId: "" };
  }
}

export default function App() {
  const [initialWorkflow] = useState(loadWorkflow);
  const [page, setPage] = useState<Page>("projects");
  const [engineStatus, setEngineStatus] = useState("Đang kết nối bộ máy cục bộ…");
  const [projectId, setProjectId] = useState(initialWorkflow.projectId);
  const [sessionId, setSessionId] = useState(initialWorkflow.sessionId);
  const [planId, setPlanId] = useState(initialWorkflow.planId);

  useEffect(() => {
    void backend
      .status()
      .then(setEngineStatus)
      .catch((error: unknown) => {
        setEngineStatus(`Không thể dùng bộ máy xử lý · ${String(error)}`);
      });
  }, []);

  useEffect(() => {
    if (initialWorkflow.projectId || initialWorkflow.sessionId || initialWorkflow.planId) return;
    void backend
      .latestPlanContext()
      .then((context) => {
        if (!context) return;
        setProjectId(context.project_id);
        setSessionId(context.session_id);
        setPlanId(context.plan_id);
      })
      .catch(() => {
        // Empty or unavailable history is valid; the user can select a project normally.
      });
  }, [initialWorkflow]);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        workflowStorageKey,
        JSON.stringify({ projectId, sessionId, planId }),
      );
    } catch {
      // The backend remains authoritative when WebView storage is unavailable.
    }
  }, [planId, projectId, sessionId]);

  function changeProject(id: string) {
    if (id !== projectId) {
      setSessionId("");
      setPlanId("");
    }
    setProjectId(id);
  }

  function changeSession(id: string) {
    if (id !== sessionId) setPlanId("");
    setSessionId(id);
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark" aria-hidden="true">
            AT
          </div>
          <div>
            <strong>Tìm tệp trùng lặp</strong>
            <span>An toàn</span>
          </div>
        </div>
        <nav aria-label="Điều hướng chính">
          {navigation.map((item) => (
            <button
              key={item.id}
              className={page === item.id ? "nav-item active" : "nav-item"}
              onClick={() => setPage(item.id)}
            >
              <span aria-hidden="true">{item.icon}</span>
              {item.label}
            </button>
          ))}
        </nav>
        <div className="safety-note">
          <span className="status-dot" />
          <strong>Đang áp dụng mặc định an toàn</strong>
          <p>Chỉ quét cho đến khi bạn khóa kế hoạch và nhập xác nhận chính xác.</p>
        </div>
      </aside>

      <main>
        <header className="topbar">
          <div>
            <p className="eyebrow">ƯU TIÊN CỤC BỘ · WINDOWS</p>
            <h1>{navigation.find((item) => item.id === page)?.label}</h1>
          </div>
          <div className="engine-chip">{engineStatus}</div>
        </header>

        <section className="page-content">
          {page === "projects" && (
            <ProjectsPage
              selectedProject={projectId}
              onSelectProject={changeProject}
              onContinue={() => setPage("scan")}
            />
          )}
          {page === "scan" && (
            <ScanPage
              projectId={projectId}
              sessionId={sessionId}
              onProjectChange={changeProject}
              onSession={changeSession}
              onReview={() => setPage("results")}
            />
          )}
          {page === "results" && (
            <ResultsPage
              sessionId={sessionId}
              planId={planId}
              onSessionChange={changeSession}
              onPlan={setPlanId}
              onQuarantine={() => setPage("quarantine")}
            />
          )}
          {page === "quarantine" && (
            <QuarantinePage
              projectId={projectId}
              sessionId={sessionId}
              planId={planId}
              onProjectChange={changeProject}
              onPlanChange={setPlanId}
            />
          )}
          {page === "recovery" && (
            <RecoveryPage projectId={projectId} onProjectChange={changeProject} />
          )}
          {page === "history" && (
            <HistoryPage projectId={projectId} onProjectChange={changeProject} />
          )}
        </section>
      </main>
    </div>
  );
}
