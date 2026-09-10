import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiResult, SettingsSnapshot } from "../api/contracts";
import { usePreferenceQueue, type PreferenceQueueApi } from "./usePreferenceQueue";

function snapshot(revision: number): SettingsSnapshot {
  return {
    revision, downloadRoot: "", folderNameTemplate: "{title}", autoFindHistoryMode: "include_all_history",
    downloadOverlapAutoMode: "off", explorePageSize: 50, danbooruPageSize: 50,
    maxColumns: 5, previewWidth: 200, danbooruPreviewWidth: 200, relatedPreviewWidth: 200,
    privacyMode: false, cacheLimitGb: 4, concurrentImageRequests: 4, requestStartIntervalMs: 0,
    autoFindGrouping: "all", downloadsGrouping: "all", exploreDisplayMode: "detail",
    autoFindDisplayMode: "detail", downloadsDisplayMode: "detail", collapsedGroupKeys: [],
    searchIncludeTags: [], searchExcludeTags: [],
  };
}
const success = (revision: number): ApiResult<SettingsSnapshot> => ({ ok: true, data: snapshot(revision) });
const failure: ApiResult<SettingsSnapshot> = {
  ok: false, error: { code: "SETTINGS_REVISION_CONFLICT", message: "설정이 변경되었습니다.", retryable: true },
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function fakeApi() {
  return {
    settingsGet: vi.fn<PreferenceQueueApi["settingsGet"]>().mockResolvedValue(success(1)),
    settingsUpdate: vi.fn<PreferenceQueueApi["settingsUpdate"]>().mockResolvedValue(success(2)),
  } satisfies PreferenceQueueApi;
}

type Queue = ReturnType<typeof usePreferenceQueue>;
type HarnessProps = { api: PreferenceQueueApi; showToast: (message: string) => void; onReady: (queue: Queue) => void };

function Harness({ api, showToast, onReady }: HarnessProps) {
  onReady(usePreferenceQueue(api, showToast));
  return null;
}

const disposers: Array<() => Promise<void>> = [];

async function mountQueue(initialApi: PreferenceQueueApi) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let queue!: Queue;
  let unmounted = false;
  const showToast = vi.fn();
  const render = async (api = initialApi, toast: (message: string) => void = showToast) => {
    await act(async () => root.render(<Harness api={api} showToast={toast} onReady={(value) => { queue = value; }} />));
  };
  const unmount = async () => {
    if (unmounted) return;
    unmounted = true;
    await act(async () => root.unmount());
    container.remove();
  };
  disposers.push(unmount);
  await render();
  return { get current() { return queue; }, showToast, render, unmount };
}

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const dispose of disposers.splice(0)) await dispose();
  vi.unstubAllGlobals();
});

describe("usePreferenceQueue", () => {
  it("reads the latest revision for each save and merges pending patches with the newest field values", async () => {
    const api = fakeApi();
    const firstSave = deferred<ApiResult<SettingsSnapshot>>();
    api.settingsGet.mockResolvedValueOnce(success(4)).mockResolvedValueOnce(success(9));
    api.settingsUpdate.mockReturnValueOnce(firstSave.promise).mockResolvedValueOnce(success(10));
    const hook = await mountQueue(api);
    const isPending = hook.current.isPending;
    expect(isPending()).toBe(false);
    await act(async () => hook.current.enqueue({ autoFindGrouping: "day" }));
    expect(api.settingsUpdate).toHaveBeenCalledExactlyOnceWith({ autoFindGrouping: "day" }, 4);
    expect(isPending()).toBe(true);
    await act(async () => {
      hook.current.enqueue({ downloadsGrouping: "artist", exploreDisplayMode: "compact" });
      hook.current.enqueue({ downloadsGrouping: "day", collapsedGroupKeys: ["auto-find:day"] });
    });
    expect(api.settingsGet).toHaveBeenCalledOnce();
    expect(api.settingsUpdate).toHaveBeenCalledOnce();

    await act(async () => firstSave.resolve(success(5)));
    expect(api.settingsGet).toHaveBeenCalledTimes(2);
    expect(api.settingsUpdate).toHaveBeenNthCalledWith(2, {
      downloadsGrouping: "day", exploreDisplayMode: "compact", collapsedGroupKeys: ["auto-find:day"],
    }, 9);
    expect(isPending()).toBe(false);
    expect(hook.showToast).not.toHaveBeenCalled();
  });

  it("reports pending synchronously through both settings reads and backend update notifications", async () => {
    const api = fakeApi();
    const read = deferred<ApiResult<SettingsSnapshot>>();
    const save = deferred<ApiResult<SettingsSnapshot>>();
    api.settingsGet.mockReturnValueOnce(read.promise);
    const hook = await mountQueue(api);
    api.settingsUpdate.mockImplementationOnce(() => {
      expect(hook.current.isPending()).toBe(true);
      return save.promise;
    });
    hook.current.enqueue({ downloadsDisplayMode: "compact" });
    expect(hook.current.isPending()).toBe(true);
    expect(api.settingsUpdate).not.toHaveBeenCalled();
    await act(async () => read.resolve(success(12)));
    expect(hook.current.isPending()).toBe(true);
    await act(async () => save.resolve(success(13)));
    expect(hook.current.isPending()).toBe(false);
  });

  it.each(["read-result", "read-rejection", "save-result", "save-rejection"] as const)(
    "continues queued work after a %s failure without retrying the failed patch",
    async (mode) => {
      const api = fakeApi();
      const pending = deferred<ApiResult<SettingsSnapshot>>();
      const readFailure = mode.startsWith("read");
      if (readFailure) api.settingsGet.mockReturnValueOnce(pending.promise);
      else api.settingsUpdate.mockReturnValueOnce(pending.promise);
      const hook = await mountQueue(api);
      await act(async () => hook.current.enqueue({ autoFindGrouping: "day" }));
      hook.current.enqueue({ downloadsGrouping: "artist" });
      await act(async () => {
        if (mode.endsWith("rejection")) pending.reject(new Error("failed"));
        else pending.resolve(failure);
      });
      expect(hook.showToast).toHaveBeenCalledExactlyOnceWith(mode.endsWith("rejection")
        ? "목록 표시 설정을 저장하지 못했습니다."
        : "목록 표시 설정을 저장하지 못했습니다. 설정이 변경되었습니다.");
      expect(api.settingsGet).toHaveBeenCalledTimes(2);
      expect(api.settingsUpdate).toHaveBeenCalledTimes(readFailure ? 1 : 2);
      expect(api.settingsUpdate).toHaveBeenLastCalledWith({ downloadsGrouping: "artist" }, 1);
      expect(hook.current.isPending()).toBe(false);
    },
  );

  it("starts a new pump after a failed save when no patch was queued", async () => {
    const api = fakeApi();
    api.settingsUpdate.mockRejectedValueOnce(new Error("failed"));
    const hook = await mountQueue(api);
    await act(async () => hook.current.enqueue({ downloadsGrouping: "day" }));
    expect(hook.current.isPending()).toBe(false);
    await act(async () => hook.current.enqueue({ downloadsGrouping: "artist" }));
    expect(api.settingsGet).toHaveBeenCalledTimes(2);
    expect(api.settingsUpdate).toHaveBeenLastCalledWith({ downloadsGrouping: "artist" }, 1);
    expect(hook.current.isPending()).toBe(false);
  });

  it("keeps the public callbacks stable while using the current toast callback", async () => {
    const api = fakeApi();
    const read = deferred<ApiResult<SettingsSnapshot>>();
    api.settingsGet.mockReturnValueOnce(read.promise);
    const hook = await mountQueue(api);
    const queue = hook.current;
    hook.current.enqueue({ exploreDisplayMode: "compact" });
    const nextToast = vi.fn();
    await hook.render(api, nextToast);
    expect(hook.current).toBe(queue);
    await act(async () => read.resolve(failure));
    expect(nextToast).toHaveBeenCalledExactlyOnceWith("목록 표시 설정을 저장하지 못했습니다. 설정이 변경되었습니다.");
    expect(hook.showToast).not.toHaveBeenCalled();
  });

  it.each(["read-success", "read-failure", "save-success", "save-failure"] as const)(
    "discards queued patches and late %s responses after unmount",
    async (mode) => {
      const api = fakeApi();
      const pending = deferred<ApiResult<SettingsSnapshot>>();
      const pendingRead = mode.startsWith("read");
      if (pendingRead) api.settingsGet.mockReturnValueOnce(pending.promise);
      else api.settingsUpdate.mockReturnValueOnce(pending.promise);
      const hook = await mountQueue(api);
      await act(async () => hook.current.enqueue({ autoFindGrouping: "day" }));
      hook.current.enqueue({ downloadsGrouping: "artist" });
      await hook.unmount();
      expect(hook.current.isPending()).toBe(false);
      hook.current.enqueue({ exploreDisplayMode: "compact" });
      await act(async () => {
        if (mode.endsWith("failure")) pending.reject(new Error("late failure"));
        else pending.resolve(success(7));
      });
      expect(api.settingsGet).toHaveBeenCalledOnce();
      expect(api.settingsUpdate).toHaveBeenCalledTimes(pendingRead ? 0 : 1);
      expect(hook.showToast).not.toHaveBeenCalled();
    },
  );

  it.each(["read", "save"] as const)("ignores the old API's pending %s when the API changes", async (phase) => {
    const api = fakeApi();
    const nextApi = fakeApi();
    const oldPending = deferred<ApiResult<SettingsSnapshot>>();
    const newPending = deferred<ApiResult<SettingsSnapshot>>();
    if (phase === "read") api.settingsGet.mockReturnValueOnce(oldPending.promise);
    else api.settingsUpdate.mockReturnValueOnce(oldPending.promise);
    nextApi.settingsGet.mockResolvedValueOnce(success(21));
    nextApi.settingsUpdate.mockReturnValueOnce(newPending.promise);
    const hook = await mountQueue(api);
    const queue = hook.current;
    await act(async () => hook.current.enqueue({ autoFindGrouping: "day" }));
    hook.current.enqueue({ downloadsGrouping: "artist" });
    await hook.render(nextApi);
    expect(hook.current).toBe(queue);
    expect(hook.current.isPending()).toBe(false);
    await act(async () => hook.current.enqueue({ exploreDisplayMode: "compact" }));
    await act(async () => oldPending.resolve(failure));
    expect(hook.current.isPending()).toBe(true);
    expect(nextApi.settingsUpdate).toHaveBeenCalledExactlyOnceWith({ exploreDisplayMode: "compact" }, 21);
    expect(api.settingsGet).toHaveBeenCalledOnce();
    expect(api.settingsUpdate).toHaveBeenCalledTimes(phase === "read" ? 0 : 1);
    expect(hook.showToast).not.toHaveBeenCalled();
    await act(async () => newPending.resolve(success(22)));
    expect(hook.current.isPending()).toBe(false);
  });
});
