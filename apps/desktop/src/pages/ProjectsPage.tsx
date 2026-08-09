import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { backend } from "../services/backend";
import type { FilterConfig, Mode, ProjectRecord, ProjectRootRecord } from "../types";
import { modeLabel } from "../services/labels";

const DEFAULT_INCLUDES = "pdf, epub, mobi";

function splitCommaSeparated(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function splitLines(value: string) {
  return value
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}

interface Props {
  selectedProject: string;
  onSelectProject: (id: string) => void;
  onContinue: () => void;
}

export function ProjectsPage({ selectedProject, onSelectProject, onContinue }: Props) {
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [roots, setRoots] = useState<ProjectRootRecord[]>([]);
  const [name, setName] = useState("Thư viện tài liệu");
  const [mode, setMode] = useState<Mode>("strict");
  const [primary, setPrimary] = useState(false);
  const [includeExtensions, setIncludeExtensions] = useState(DEFAULT_INCLUDES);
  const [excludeExtensions, setExcludeExtensions] = useState("");
  const [excludeGlobs, setExcludeGlobs] = useState("");
  const [minimumSize, setMinimumSize] = useState("0");
  const [workerLimit, setWorkerLimit] = useState("4");
  const [skipHidden, setSkipHidden] = useState(true);
  const [skipSystem, setSkipSystem] = useState(true);
  const [message, setMessage] = useState("");

  function filterConfig(): FilterConfig {
    return {
      include_extensions: splitCommaSeparated(includeExtensions),
      exclude_extensions: splitCommaSeparated(excludeExtensions),
      exclude_globs: splitLines(excludeGlobs),
      minimum_size: Number(minimumSize),
      skip_hidden: skipHidden,
      skip_system: skipSystem,
    };
  }

  const refresh = useCallback(async () => {
    const rows = await backend.listProjects();
    const activeProjects = rows.filter((project) => project.status === "active");
    setProjects(activeProjects);
    if (!activeProjects.some((project) => project.id === selectedProject)) {
      onSelectProject(activeProjects[0]?.id ?? "");
    }
  }, [onSelectProject, selectedProject]);

  useEffect(() => {
    void refresh().catch((error: unknown) => setMessage(String(error)));
  }, [refresh]);

  useEffect(() => {
    if (!selectedProject) {
      setRoots([]);
      return;
    }
    void Promise.all([backend.listRoots(selectedProject), backend.getFilterConfig(selectedProject)])
      .then(([nextRoots, filters]) => {
        setRoots(nextRoots);
        setIncludeExtensions(filters.include_extensions.join(", "));
        setExcludeExtensions(filters.exclude_extensions.join(", "));
        setExcludeGlobs(filters.exclude_globs.join("\n"));
        setMinimumSize(String(filters.minimum_size));
        setSkipHidden(filters.skip_hidden);
        setSkipSystem(filters.skip_system);
      })
      .catch((error: unknown) => setMessage(String(error)));
  }, [selectedProject]);

  useEffect(() => {
    const selected = projects.find((project) => project.id === selectedProject);
    if (selected) {
      setName(selected.name);
      setMode(selected.mode);
      setWorkerLimit(String(selected.worker_limit));
    }
  }, [projects, selectedProject]);

  async function createProject() {
    try {
      const id = await backend.createProject(name, mode);
      await backend.saveFilterConfig(id, filterConfig());
      onSelectProject(id);
      setMessage("Đã tạo dự án. Hãy thêm một hoặc nhiều thư mục; ứng dụng không tự động quét.");
      await refresh();
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function addFolder() {
    if (!selectedProject) return setMessage("Hãy chọn dự án trước.");
    try {
      const folder = await open({
        directory: true,
        multiple: false,
        title: "Chọn thư mục nguồn",
      });
      if (!folder) return;
      await backend.addRoot(selectedProject, folder, primary);
      setRoots(await backend.listRoots(selectedProject));
      setMessage(`Đã thêm thư mục: ${folder}. Thư mục này chưa được quét.`);
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function saveProject() {
    if (!selectedProject) return;
    try {
      await Promise.all([
        backend.updateProject(selectedProject, name, mode),
        backend.setProjectWorkers(selectedProject, Number(workerLimit)),
        backend.saveFilterConfig(selectedProject, filterConfig()),
      ]);
      await refresh();
      setMessage("Đã lưu cài đặt dự án. Không có phiên quét nào được khởi động.");
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function archiveProject() {
    if (
      !selectedProject ||
      !window.confirm("Lưu trữ hồ sơ dự án này? Các tệp nguồn sẽ không bị thay đổi.")
    )
      return;
    try {
      await backend.archiveProject(selectedProject, "ARCHIVE");
      onSelectProject("");
      setRoots([]);
      const rows = (await backend.listProjects()).filter((project) => project.status === "active");
      setProjects(rows);
      onSelectProject(rows[0]?.id ?? "");
      setMessage("Đã lưu trữ dự án. Dữ liệu nguồn và dữ liệu cách ly không bị xóa.");
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function removeFolder(rootId: string) {
    try {
      await backend.removeRoot(selectedProject, rootId);
      setRoots(await backend.listRoots(selectedProject));
      setMessage("Đã gỡ thư mục khỏi cấu hình. Các tệp trên ổ đĩa không bị thay đổi.");
    } catch (error) {
      setMessage(String(error));
    }
  }

  return (
    <div className="content-grid two-column">
      <section className="card">
        <div className="card-heading">
          <div>
            <p className="eyebrow">BƯỚC 1</p>
            <h2>Tạo hoặc chọn dự án</h2>
          </div>
          <span className="badge safe">Không tự động quét</span>
        </div>
        <label>
          Tên dự án
          <input value={name} onChange={(event) => setName(event.target.value)} />
        </label>
        <fieldset className="mode-picker">
          <legend>Chế độ so sánh</legend>
          <label className={mode === "strict" ? "choice active" : "choice"}>
            <input type="radio" checked={mode === "strict"} onChange={() => setMode("strict")} />
            <span>
              <strong>Nghiêm ngặt</strong>
              Cùng tên đã chuẩn hóa + kích thước + BLAKE3 đầy đủ + SHA-256 đầy đủ
            </span>
          </label>
          <label className={mode === "content" ? "choice warning" : "choice"}>
            <input type="radio" checked={mode === "content"} onChange={() => setMode("content")} />
            <span>
              <strong>Chỉ so sánh nội dung</strong>
              Bỏ qua tên tệp và yêu cầu xác nhận cảnh báo khi bắt đầu quét
            </span>
          </label>
        </fieldset>
        <button className="primary" onClick={() => void createProject()} disabled={!name.trim()}>
          Tạo dự án
        </button>
        <div className="button-row">
          <button
            className="secondary"
            onClick={() => void saveProject()}
            disabled={!selectedProject || !name.trim()}
          >
            Lưu dự án đã chọn
          </button>
          <button
            className="danger-quiet"
            onClick={() => void archiveProject()}
            disabled={!selectedProject}
          >
            Lưu trữ hồ sơ
          </button>
        </div>
      </section>

      <section className="card">
        <div className="card-heading">
          <div>
            <p className="eyebrow">BƯỚC 2</p>
            <h2>Thư mục nguồn</h2>
          </div>
        </div>
        <label>
          Dự án đang chọn
          <select value={selectedProject} onChange={(event) => onSelectProject(event.target.value)}>
            <option value="">Chọn một dự án</option>
            {projects.map((project) => (
              <option key={project.id} value={project.id}>
                {project.name} · {modeLabel(project.mode)}
                {project.last_scan_at
                  ? ` · lần quét cuối ${new Date(project.last_scan_at).toLocaleDateString("vi-VN")}`
                  : ""}
              </option>
            ))}
          </select>
        </label>
        <div className="folder-drop">
          <span className="folder-icon">＋</span>
          <strong>Chủ động thêm thư mục</strong>
          <p>Ứng dụng từ chối thư mục cha/con chồng lấn, cơ sở dữ liệu và thư mục cách ly.</p>
          <button className="secondary" onClick={() => void addFolder()}>
            Chọn thư mục
          </button>
          <label className="inline-check">
            <input
              type="checkbox"
              checked={primary}
              onChange={(event) => setPrimary(event.target.checked)}
            />
            Ưu tiên thư mục này khi chọn tệp giữ lại
          </label>
        </div>
        <div className="root-list" aria-label="Các thư mục nguồn đã cấu hình">
          {roots.length === 0 && <p>Chưa cấu hình thư mục nguồn.</p>}
          {roots.map((root) => (
            <div className="root-row" key={root.id}>
              <div>
                <code>{root.path}</code>
                {root.primary && <span className="badge safe">Ưu tiên</span>}
              </div>
              <button className="danger-quiet" onClick={() => void removeFolder(root.id)}>
                Gỡ bỏ
              </button>
            </div>
          ))}
        </div>
        <button className="primary" disabled={!selectedProject} onClick={onContinue}>
          Tiếp tục đến bước quét
        </button>
      </section>
      <section className="card full">
        <div className="card-heading">
          <div>
            <p className="eyebrow">ĐƯỢC LƯU CÙNG DỰ ÁN</p>
            <h2>Bộ lọc quét chỉ đọc</h2>
          </div>
          <span className="badge safe">Áp dụng trước khi băm</span>
        </div>
        <div className="filter-grid">
          <label>
            Phần mở rộng cần quét (cách nhau bằng dấu phẩy; để trống là tất cả)
            <input
              value={includeExtensions}
              onChange={(event) => setIncludeExtensions(event.target.value)}
              placeholder="pdf, epub, mobi"
            />
          </label>
          <label>
            Phần mở rộng cần loại trừ
            <input
              value={excludeExtensions}
              onChange={(event) => setExcludeExtensions(event.target.value)}
              placeholder="tmp, tải-dở"
            />
          </label>
          <label>
            Kích thước tối thiểu theo byte
            <input
              type="number"
              min="0"
              step="1"
              value={minimumSize}
              onChange={(event) => setMinimumSize(event.target.value)}
            />
          </label>
          <label>
            Giới hạn luồng xử lý toàn cục (1–64)
            <input
              type="number"
              min="1"
              max="64"
              step="1"
              value={workerLimit}
              onChange={(event) => setWorkerLimit(event.target.value)}
            />
          </label>
          <label className="filter-globs">
            Mẫu đường dẫn cần loại trừ (mỗi dòng một mẫu)
            <textarea
              rows={3}
              value={excludeGlobs}
              onChange={(event) => setExcludeGlobs(event.target.value)}
              placeholder={"**/cache/**\n**/*.download"}
            />
          </label>
        </div>
        <div className="button-row filter-checks">
          <label className="inline-check">
            <input
              type="checkbox"
              checked={skipHidden}
              onChange={(event) => setSkipHidden(event.target.checked)}
            />
            Bỏ qua tệp ẩn
          </label>
          <label className="inline-check">
            <input
              type="checkbox"
              checked={skipSystem}
              onChange={(event) => setSkipSystem(event.target.checked)}
            />
            Bỏ qua tệp hệ thống Windows
          </label>
        </div>
        <p className="muted-copy">
          Dùng “Lưu dự án đã chọn” phía trên để kiểm tra và lưu các bộ lọc này. Việc thêm thư mục
          vẫn không bao giờ tự động bắt đầu quét.
        </p>
      </section>
      {message && <div className="notice full">{message}</div>}
    </div>
  );
}
