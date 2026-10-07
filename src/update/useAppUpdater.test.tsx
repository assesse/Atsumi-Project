import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useAppUpdater, type UpdateInstallGuard } from "./useAppUpdater";

const pluginMocks = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
  invoke: vi.fn(async (_command: string) => "installed"),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({ check: pluginMocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: pluginMocks.relaunch }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: pluginMocks.invoke, Channel: class { onmessage?: (value: unknown) => void } }));

type HarnessProps = {
  runtime: "tauri" | "browser-mock";
  onReady: (updater: ReturnType<typeof useAppUpdater>) => void;
  guard?: UpdateInstallGuard;
};

function Harness({ runtime, onReady, guard }: HarnessProps) {
  const updater = useAppUpdater(runtime, guard);
  onReady(updater);
  return <span data-phase={updater.state.phase}>{updater.state.info?.version ?? "none"}</span>;
}

afterEach(() => {
  pluginMocks.check.mockReset();
  pluginMocks.relaunch.mockReset();
  pluginMocks.invoke.mockReset().mockResolvedValue("installed");
});

describe("useAppUpdater", () => {
  it("uses a portable-only target and native helper, never the installer or old-app relaunch", async () => {
    pluginMocks.invoke.mockImplementation(async (command: string) => command === "app_update_mode" ? "portable" : "");
    const downloadAndInstall = vi.fn();
    pluginMocks.check.mockResolvedValue({ currentVersion: "2.1.1", version: "2.1.2", downloadAndInstall, close: vi.fn(async () => undefined) });
    const container = document.createElement("div"); const root = createRoot(container);
    let updater!: ReturnType<typeof useAppUpdater>;
    const guard = { acquire: vi.fn(async () => undefined), release: vi.fn(async () => undefined) };
    try {
      await act(async () => root.render(<Harness runtime="tauri" guard={guard} onReady={value => { updater = value; }} />));
      expect(pluginMocks.check).toHaveBeenCalledWith({ timeout: 10_000, target: "windows-x86_64-portable" });
      expect(updater.state.info?.portable).toBe(true);
      await act(async () => updater.installUpdate());
      expect(guard.acquire).toHaveBeenCalledOnce();
      expect(pluginMocks.invoke).toHaveBeenCalledWith("app_update_portable", expect.objectContaining({ version: "2.1.2" }));
      expect(downloadAndInstall).not.toHaveBeenCalled(); expect(pluginMocks.relaunch).not.toHaveBeenCalled();
    } finally { await act(async () => root.unmount()); }
  });
  it("does not offer installer updates to a development build or an unknown distribution", async () => {
    for (const mode of ["development", "unknown"]) {
      pluginMocks.invoke.mockResolvedValue(mode);
      const container = document.createElement("div"); const root = createRoot(container);
      let updater!: ReturnType<typeof useAppUpdater>;
      try {
        await act(async () => root.render(<Harness runtime="tauri" onReady={value => { updater = value; }} />));
        expect(updater.state.phase).toBe("idle"); expect(pluginMocks.check).not.toHaveBeenCalled();
      } finally { await act(async () => root.unmount()); }
    }
  });
  it("does not install or relaunch while a recording holds the backend reservation", async () => {
    const downloadAndInstall = vi.fn();
    const guard = { acquire: vi.fn(async () => { throw new Error("녹화를 중지한 후 업데이트해 주세요."); }), release: vi.fn(async () => undefined) };
    pluginMocks.check.mockResolvedValue({ currentVersion: "1.6.0", version: "1.7.0", downloadAndInstall, close: vi.fn(async () => undefined) });
    const container = document.createElement("div");
    const root = createRoot(container);
    let updater!: ReturnType<typeof useAppUpdater>;
    try {
      await act(async () => root.render(<Harness runtime="tauri" guard={guard} onReady={(value) => { updater = value; }} />));
      await act(async () => { await updater.installUpdate(); });
      expect(guard.acquire).toHaveBeenCalledOnce();
      expect(downloadAndInstall).not.toHaveBeenCalled();
      expect(pluginMocks.relaunch).not.toHaveBeenCalled();
      expect(guard.release).not.toHaveBeenCalled();
      expect(updater.state.error).toContain("녹화를 중지");
    } finally { await act(async () => root.unmount()); }
  });

  it("releases the start barrier when an installation fails", async () => {
    const guard = { acquire: vi.fn(async () => undefined), release: vi.fn(async () => undefined) };
    pluginMocks.check.mockResolvedValue({ currentVersion: "1.6.0", version: "1.7.0", downloadAndInstall: vi.fn(async () => { throw new Error("network"); }), close: vi.fn(async () => undefined) });
    const container = document.createElement("div");
    const root = createRoot(container);
    let updater!: ReturnType<typeof useAppUpdater>;
    try {
      await act(async () => root.render(<Harness runtime="tauri" guard={guard} onReady={(value) => { updater = value; }} />));
      await act(async () => { await updater.installUpdate(); });
      expect(guard.acquire).toHaveBeenCalledOnce();
      expect(guard.release).toHaveBeenCalledOnce();
      expect(pluginMocks.relaunch).not.toHaveBeenCalled();
    } finally { await act(async () => root.unmount()); }
  });

  it("checks at desktop startup, downloads an accepted update, and relaunches", async () => {
    const close = vi.fn(async () => undefined);
    const downloadAndInstall = vi.fn(async (onEvent?: (event: unknown) => void) => {
      onEvent?.({ event: "Started", data: { contentLength: 100 } });
      onEvent?.({ event: "Progress", data: { chunkLength: 100 } });
      onEvent?.({ event: "Finished", data: {} });
    });
    pluginMocks.check.mockResolvedValue({
      currentVersion: "1.0.0",
      version: "1.1.0",
      date: "2026-08-27T00:00:00Z",
      body: "새 기능",
      close,
      downloadAndInstall,
    });
    pluginMocks.relaunch.mockResolvedValue(undefined);

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    let updater: ReturnType<typeof useAppUpdater> | undefined;
    try {
      await act(async () => root.render(
        <Harness runtime="tauri" onReady={(value) => { updater = value; }} />,
      ));
      await act(async () => { await Promise.resolve(); });
      expect(pluginMocks.check).toHaveBeenCalledWith({ timeout: 10_000 });
      expect(updater?.state.phase).toBe("available");
      expect(updater?.state.info?.version).toBe("1.1.0");

      await act(async () => { await updater?.installUpdate(); });
      expect(downloadAndInstall).toHaveBeenCalledOnce();
      expect(pluginMocks.relaunch).toHaveBeenCalledOnce();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("does not call the native updater in browser preview", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    let updater: ReturnType<typeof useAppUpdater> | undefined;
    try {
      await act(async () => root.render(
        <Harness runtime="browser-mock" onReady={(value) => { updater = value; }} />,
      ));
      const result = await updater?.checkForUpdates("manual");
      expect(result).toEqual({ status: "unavailable" });
      expect(pluginMocks.check).not.toHaveBeenCalled();
      expect(updater?.state.phase).toBe("idle");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("keeps startup network errors silent and reports a manual retry failure", async () => {
    pluginMocks.check.mockRejectedValue(new Error("offline"));
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    let updater: ReturnType<typeof useAppUpdater> | undefined;
    try {
      await act(async () => root.render(
        <Harness runtime="tauri" onReady={(value) => { updater = value; }} />,
      ));
      await act(async () => { await Promise.resolve(); });
      expect(updater?.state).toEqual({ phase: "idle", info: null, downloadedBytes: 0 });

      let result;
      await act(async () => { result = await updater?.checkForUpdates("manual"); });
      expect(result).toEqual({
        status: "failed",
        message: "업데이트 정보를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.",
      });
      expect(pluginMocks.check).toHaveBeenCalledTimes(2);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
