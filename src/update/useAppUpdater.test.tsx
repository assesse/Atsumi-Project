import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useAppUpdater } from "./useAppUpdater";

const pluginMocks = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({ check: pluginMocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: pluginMocks.relaunch }));

type HarnessProps = {
  runtime: "tauri" | "browser-mock";
  onReady: (updater: ReturnType<typeof useAppUpdater>) => void;
};

function Harness({ runtime, onReady }: HarnessProps) {
  const updater = useAppUpdater(runtime);
  onReady(updater);
  return <span data-phase={updater.state.phase}>{updater.state.info?.version ?? "none"}</span>;
}

afterEach(() => {
  pluginMocks.check.mockReset();
  pluginMocks.relaunch.mockReset();
});

describe("useAppUpdater", () => {
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
