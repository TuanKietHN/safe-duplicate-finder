import { backend } from "../services/backend";
import type { FilterConfig, Mode, ProjectRecord, ProjectRootRecord } from "../types";

export interface ProjectBackend {
  listProjects(): Promise<ProjectRecord[]>;
  listRoots(projectId: string): Promise<ProjectRootRecord[]>;
  getFilterConfig(projectId: string): Promise<FilterConfig>;
  createProject(name: string, mode: string): Promise<string>;
  updateProject(projectId: string, name: string, mode: string): Promise<void>;
  setProjectWorkers(projectId: string, workers: number): Promise<void>;
  saveFilterConfig(projectId: string, config: FilterConfig): Promise<void>;
  addRoot(projectId: string, path: string, primary: boolean): Promise<string>;
  removeRoot(projectId: string, rootId: string): Promise<void>;
}

export interface ProjectState {
  projects: ProjectRecord[];
  roots: ProjectRootRecord[];
  selectedProject: string;
  filters: FilterConfig;
  loading: boolean;
  error: string;
}

const defaultFilters: FilterConfig = {
  include_extensions: ["pdf", "epub", "mobi"],
  exclude_extensions: [],
  exclude_globs: [],
  minimum_size: 0,
  skip_hidden: true,
  skip_system: true,
};

type Listener = () => void;

/** External project store. Configuration methods intentionally contain no scan-start capability. */
export class ProjectStore {
  private snapshot: ProjectState = {
    projects: [],
    roots: [],
    selectedProject: "",
    filters: defaultFilters,
    loading: false,
    error: "",
  };
  private readonly listeners = new Set<Listener>();

  constructor(private readonly api: ProjectBackend = backend) {}

  getSnapshot = () => this.snapshot;

  subscribe = (listener: Listener) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async load() {
    this.patch({ loading: true, error: "" });
    try {
      const projects = (await this.api.listProjects()).filter(
        (project) => project.status === "active",
      );
      const selectedProject = projects.some(
        (project) => project.id === this.snapshot.selectedProject,
      )
        ? this.snapshot.selectedProject
        : (projects[0]?.id ?? "");
      this.patch({ projects, selectedProject });
      await this.loadSelection(selectedProject);
    } catch (error) {
      this.patch({ error: String(error) });
    } finally {
      this.patch({ loading: false });
    }
  }

  async select(projectId: string) {
    this.patch({ selectedProject: projectId, error: "" });
    await this.loadSelection(projectId);
  }

  async create(name: string, mode: Mode) {
    const id = await this.api.createProject(name, mode);
    await this.api.saveFilterConfig(id, this.snapshot.filters);
    this.patch({ selectedProject: id });
    await this.load();
    return id;
  }

  async save(name: string, mode: Mode, workers: number, filters: FilterConfig) {
    const projectId = this.requireSelection();
    await Promise.all([
      this.api.updateProject(projectId, name, mode),
      this.api.setProjectWorkers(projectId, workers),
      this.api.saveFilterConfig(projectId, filters),
    ]);
    this.patch({ filters });
    await this.load();
  }

  async addRoot(path: string, primary: boolean) {
    const projectId = this.requireSelection();
    await this.api.addRoot(projectId, path, primary);
    this.patch({ roots: await this.api.listRoots(projectId) });
  }

  async removeRoot(rootId: string) {
    const projectId = this.requireSelection();
    await this.api.removeRoot(projectId, rootId);
    this.patch({ roots: await this.api.listRoots(projectId) });
  }

  private async loadSelection(projectId: string) {
    if (!projectId) {
      this.patch({ roots: [], filters: defaultFilters });
      return;
    }
    const [roots, filters] = await Promise.all([
      this.api.listRoots(projectId),
      this.api.getFilterConfig(projectId),
    ]);
    this.patch({ roots, filters });
  }

  private requireSelection() {
    if (!this.snapshot.selectedProject) throw new Error("Hãy chọn dự án trước.");
    return this.snapshot.selectedProject;
  }

  private patch(next: Partial<ProjectState>) {
    this.snapshot = { ...this.snapshot, ...next };
    this.listeners.forEach((listener) => listener());
  }
}

export const projectStore = new ProjectStore();
