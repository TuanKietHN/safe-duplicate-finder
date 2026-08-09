import { describe, expect, it, vi } from "vitest";
import type { FilterConfig, ProjectRecord, ProjectRootRecord } from "../types";
import { ProjectStore, type ProjectBackend } from "./project";

const filters: FilterConfig = {
  include_extensions: ["pdf"],
  exclude_extensions: [],
  exclude_globs: [],
  minimum_size: 0,
  skip_hidden: true,
  skip_system: true,
};

describe("ProjectStore", () => {
  it("loads, creates, and configures folders without ever starting a scan", async () => {
    const projects: ProjectRecord[] = [
      {
        id: "project-1",
        name: "Library",
        mode: "strict",
        worker_limit: 4,
        status: "active",
      },
    ];
    const roots = new Map<string, ProjectRootRecord[]>([["project-1", []]]);
    const startScan = vi.fn();
    const api: ProjectBackend & { startScan: typeof startScan } = {
      startScan,
      listProjects: vi.fn(async () => [...projects]),
      listRoots: vi.fn(async (projectId) => [...(roots.get(projectId) ?? [])]),
      getFilterConfig: vi.fn(async () => filters),
      createProject: vi.fn(async (name, mode) => {
        projects.push({
          id: "project-2",
          name,
          mode: mode === "content" ? "content" : "strict",
          worker_limit: 4,
          status: "active",
        });
        roots.set("project-2", []);
        return "project-2";
      }),
      updateProject: vi.fn(async () => undefined),
      setProjectWorkers: vi.fn(async () => undefined),
      saveFilterConfig: vi.fn(async () => undefined),
      addRoot: vi.fn(async (projectId, path, primary) => {
        roots.set(projectId, [{ id: "root-1", path, primary }]);
        return "root-1";
      }),
      removeRoot: vi.fn(async (projectId) => {
        roots.set(projectId, []);
      }),
    };
    const store = new ProjectStore(api);
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    await store.load();
    expect(store.getSnapshot().selectedProject).toBe("project-1");
    expect(store.getSnapshot().roots).toEqual([]);
    const created = await store.create("Research", "strict");
    expect(created).toBe("project-2");
    expect(store.getSnapshot().selectedProject).toBe("project-2");
    await store.addRoot("D:\\Documents", true);
    expect(store.getSnapshot().roots).toEqual([
      { id: "root-1", path: "D:\\Documents", primary: true },
    ]);
    await store.save("Research", "strict", 3, filters);

    expect(api.createProject).toHaveBeenCalledOnce();
    expect(api.setProjectWorkers).toHaveBeenCalledWith("project-2", 3);
    expect(api.saveFilterConfig).toHaveBeenCalled();
    expect(startScan).not.toHaveBeenCalled();
    expect(listener).toHaveBeenCalled();
    unsubscribe();
  });

  it("rejects folder mutation when no project is selected", async () => {
    const api = {
      listProjects: vi.fn(async () => []),
      listRoots: vi.fn(async () => []),
      getFilterConfig: vi.fn(async () => filters),
      createProject: vi.fn(),
      updateProject: vi.fn(),
      setProjectWorkers: vi.fn(),
      saveFilterConfig: vi.fn(),
      addRoot: vi.fn(),
      removeRoot: vi.fn(),
    } satisfies ProjectBackend;
    const store = new ProjectStore(api);
    await store.load();
    await expect(store.addRoot("D:\\NoProject", false)).rejects.toThrow("Hãy chọn dự án trước.");
    expect(api.addRoot).not.toHaveBeenCalled();
  });
});
