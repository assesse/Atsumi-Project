import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiResult, AppActiveWorkSnapshot, AppQuitResult } from "../api/contracts";
import { useAppExit, type AppExitApi } from "./useAppExit";

const idleSnapshot: AppActiveWorkSnapshot = {
  queriedAt: "2026-09-10T00:00:00.000Z",
  workSetFingerprint: "idle",
  downloads: { activeCount: 0 },
};
const activeSnapshot: AppActiveWorkSnapshot = {
  ...idleSnapshot,
  workSetFingerprint: "download-1",
  downloads: { activeCount: 1 },
};
const success = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
const unavailable: ApiResult<never> = {
  ok: false,
  error: { code: "APP_ACTIVE_WORK_STATUS_UNAVAILABLE", message: "작업 상태 오류", retryable: true },
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
  let handler: Parameters<AppExitApi["on"]>[1] | undefined;
  const unsubscribe = vi.fn();
  const api = {
    appActiveWorkSnapshot: vi.fn<AppExitApi["appActiveWorkSnapshot"]>().mockResolvedValue(success(idleSnapshot)),
    appMinimizeToTray: vi.fn<AppExitApi["appMinimizeToTray"]>().mockResolvedValue(success(null)),
    appQuit: vi.fn<AppExitApi["appQuit"]>().mockResolvedValue(success({ accepted: true })),
    on: vi.fn<AppExitApi["on"]>().mockImplementation(async (_event, listener) => {
      handler = listener;
      return unsubscribe;
    }),
  } satisfies AppExitApi;
  return { api, unsubscribe, requestExit: () => handler?.({ source: "window_close" }) };
}

type ExitController = ReturnType<typeof useAppExit>;
type HarnessProps = { api: AppExitApi; showToast: (message: string) => void; onReady: (value: ExitController) => void };

function Harness({ api, showToast, onReady }: HarnessProps) {
  const controller = useAppExit(api, showToast);
  onReady(controller);
  return null;
}

const disposers: Array<() => Promise<void>> = [];

async function mountExit(api: AppExitApi, strict = false) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let controller!: ExitController;
  let unmounted = false;
  const showToast = vi.fn();
  const render = async (toast: (message: string) => void = showToast) => {
    const harness = <Harness api={api} showToast={toast} onReady={(value) => { controller = value; }} />;
    await act(async () => root.render(strict ? <StrictMode>{harness}</StrictMode> : harness));
  };
  const unmount = async () => {
    if (unmounted) return;
    unmounted = true;
    await act(async () => root.unmount());
    container.remove();
  };
  disposers.push(unmount);
  await render();
  return { get current() { return controller; }, showToast, render, unmount };
}

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const dispose of disposers.splice(0)) await dispose();
  vi.unstubAllGlobals();
});

describe("useAppExit", () => {
  it("owns one exit subscription across dialog and toast changes and releases it on unmount", async () => {
    const fake = fakeApi();
    const hook = await mountExit(fake.api);
    expect(hook.current.open).toBe(false);
    await act(async () => { fake.requestExit(); fake.requestExit(); });
    expect(hook.current.open).toBe(true);
    expect(hook.current.dialogProps.snapshot).toEqual(idleSnapshot);
    expect(fake.api.appActiveWorkSnapshot).toHaveBeenCalledOnce();

    const nextToast = vi.fn();
    await hook.render(nextToast);
    fake.api.appMinimizeToTray.mockRejectedValueOnce(new Error("tray unavailable"));
    await act(async () => hook.current.dialogProps.onMinimizeToTray());
    expect(nextToast).toHaveBeenCalledWith("트레이로 최소화하지 못했습니다.");
    expect(hook.showToast).not.toHaveBeenCalled();
    await act(async () => hook.current.closeExitConfirm());
    expect(hook.current.open).toBe(false);
    expect(fake.api.on).toHaveBeenCalledOnce();
    expect(fake.api.on).toHaveBeenCalledWith("app:exit-requested", expect.any(Function));
    await hook.unmount();
    expect(fake.unsubscribe).toHaveBeenCalledOnce();
    fake.requestExit();
    expect(fake.api.appActiveWorkSnapshot).toHaveBeenCalledOnce();
  });

  it.each(["success", "failure"] as const)("ignores stale snapshot %s after closing and reopening", async (outcome) => {
    const fake = fakeApi();
    const first = deferred<ApiResult<AppActiveWorkSnapshot>>();
    fake.api.appActiveWorkSnapshot.mockReturnValueOnce(first.promise).mockResolvedValueOnce(success(activeSnapshot));
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => hook.current.dialogProps.onQuit());
    expect(fake.api.appQuit).not.toHaveBeenCalled();
    await act(async () => hook.current.closeExitConfirm());
    await act(async () => hook.current.openExitConfirm());
    await act(async () => {
      if (outcome === "success") first.resolve(success(idleSnapshot));
      else first.reject(new Error("stale failure"));
    });
    expect(hook.current.dialogProps.snapshot).toEqual(activeSnapshot);
    expect(hook.current.dialogProps.statusError).toBe(false);
  });

  it("rechecks unavailable work before a separate forced quit and locks duplicate actions", async () => {
    const fake = fakeApi();
    fake.api.appActiveWorkSnapshot.mockResolvedValue(unavailable);
    const quit = deferred<ApiResult<AppQuitResult>>();
    fake.api.appQuit.mockReturnValue(quit.promise);
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    expect(hook.current.dialogProps).toMatchObject({ snapshot: null, statusError: true, forceQuitArmed: false });
    await act(async () => { hook.current.dialogProps.onQuit(); hook.current.dialogProps.onQuit(); });
    expect(fake.api.appActiveWorkSnapshot).toHaveBeenCalledTimes(2);
    expect(fake.api.appQuit).not.toHaveBeenCalled();
    expect(hook.current.dialogProps).toMatchObject({ forceQuitArmed: true, actionPending: false });
    expect(hook.showToast).toHaveBeenCalledWith("작업 상태를 다시 확인하지 못했습니다. 트레이로 보내거나 상태 확인 없이 종료할 수 있습니다.");

    await act(async () => {
      hook.current.dialogProps.onQuit();
      hook.current.dialogProps.onQuit();
      hook.current.dialogProps.onMinimizeToTray();
      hook.current.closeExitConfirm();
      fake.requestExit();
    });
    expect(fake.api.appQuit).toHaveBeenCalledExactlyOnceWith({
      expectedWorkSetFingerprint: "", confirmActiveWork: true, forceWhenStatusUnknown: true,
    });
    expect(fake.api.appMinimizeToTray).not.toHaveBeenCalled();
    expect(hook.current.open).toBe(true);
    expect(hook.current.dialogProps.actionPending).toBe(true);
    await act(async () => quit.resolve(success({ accepted: true })));
    await act(async () => hook.current.dialogProps.onQuit());
    expect(fake.api.appQuit).toHaveBeenCalledOnce();
    expect(hook.current.dialogProps.actionPending).toBe(true);
  });

  it("requires a new quit choice after recovering the work snapshot", async () => {
    const fake = fakeApi();
    fake.api.appActiveWorkSnapshot.mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(success(activeSnapshot));
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => hook.current.dialogProps.onQuit());
    expect(hook.current.dialogProps).toMatchObject({ snapshot: activeSnapshot, statusError: false, forceQuitArmed: false, actionPending: false });
    expect(fake.api.appQuit).not.toHaveBeenCalled();
    expect(hook.showToast).not.toHaveBeenCalled();
    await act(async () => hook.current.dialogProps.onQuit());
    expect(fake.api.appQuit).toHaveBeenCalledExactlyOnceWith({ expectedWorkSetFingerprint: "download-1", confirmActiveWork: true });
  });

  it("shows a changed work snapshot and requires renewed confirmation", async () => {
    const fake = fakeApi();
    fake.api.appQuit.mockResolvedValueOnce(success({ accepted: false, reason: "active_work_changed", snapshot: activeSnapshot }));
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => hook.current.dialogProps.onQuit());
    expect(fake.api.appQuit).toHaveBeenNthCalledWith(1, { expectedWorkSetFingerprint: "idle", confirmActiveWork: false });
    expect(hook.current.dialogProps).toMatchObject({ snapshot: activeSnapshot, actionPending: false, statusError: false });
    expect(hook.showToast).toHaveBeenCalledWith("진행 작업이 변경되었습니다. 내용을 확인하고 다시 선택해 주세요.");
    await act(async () => hook.current.dialogProps.onQuit());
    expect(fake.api.appQuit).toHaveBeenNthCalledWith(2, { expectedWorkSetFingerprint: "download-1", confirmActiveWork: true });
  });

  it("retains the snapshot when a declined quit supplies no replacement", async () => {
    const fake = fakeApi();
    fake.api.appQuit.mockResolvedValueOnce(success({ accepted: false, reason: "active_work_confirmation_required" }));
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => hook.current.dialogProps.onQuit());
    expect(hook.current.dialogProps).toMatchObject({ snapshot: idleSnapshot, actionPending: false });
    expect(hook.showToast).toHaveBeenCalledWith("진행 중인 작업을 확인한 뒤 종료를 다시 선택해 주세요.");
  });

  it("resets unavailable work and permits retry after a failed quit", async () => {
    const fake = fakeApi();
    fake.api.appQuit.mockResolvedValueOnce(unavailable).mockRejectedValueOnce(new Error("quit failed"));
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => hook.current.dialogProps.onQuit());
    expect(hook.current.dialogProps).toMatchObject({ snapshot: null, statusError: true, forceQuitArmed: false, actionPending: false });
    expect(hook.showToast).toHaveBeenCalledWith("작업 상태 오류");
    await act(async () => hook.current.dialogProps.onQuit());
    await act(async () => hook.current.dialogProps.onQuit());
    expect(hook.current.dialogProps.actionPending).toBe(false);
    expect(hook.showToast).toHaveBeenCalledWith("프로그램을 종료하지 못했습니다.");
  });

  it("minimizes once and discards a snapshot that arrives after the dialog closes", async () => {
    const fake = fakeApi();
    const snapshot = deferred<ApiResult<AppActiveWorkSnapshot>>();
    const tray = deferred<ApiResult<null>>();
    fake.api.appActiveWorkSnapshot.mockReturnValueOnce(snapshot.promise);
    fake.api.appMinimizeToTray.mockReturnValueOnce(tray.promise);
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => {
      hook.current.dialogProps.onMinimizeToTray();
      hook.current.dialogProps.onMinimizeToTray();
      hook.current.closeExitConfirm();
    });
    expect(hook.current.open).toBe(true);
    expect(fake.api.appMinimizeToTray).toHaveBeenCalledOnce();
    await act(async () => tray.resolve(success(null)));
    expect(hook.current.open).toBe(false);
    await act(async () => snapshot.resolve(success(activeSnapshot)));
    expect(hook.current.dialogProps.snapshot).toBeNull();
  });

  it("does not let an earlier snapshot erase a minimize status-unavailable error", async () => {
    const fake = fakeApi();
    const snapshot = deferred<ApiResult<AppActiveWorkSnapshot>>();
    fake.api.appActiveWorkSnapshot.mockReturnValueOnce(snapshot.promise);
    fake.api.appMinimizeToTray.mockResolvedValueOnce(unavailable);
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => hook.current.dialogProps.onMinimizeToTray());
    await act(async () => snapshot.resolve(success(activeSnapshot)));
    expect(hook.current.dialogProps).toMatchObject({ open: true, snapshot: null, statusError: true, forceQuitArmed: false, actionPending: false });
    expect(hook.showToast).toHaveBeenCalledWith("작업 상태 오류");
  });

  it.each(["resolve", "reject"] as const)("handles listener registration %s after unmount without notifying", async (outcome) => {
    const fake = fakeApi();
    const registration = deferred<() => void>();
    fake.api.on.mockReturnValueOnce(registration.promise);
    const hook = await mountExit(fake.api);
    const listener = fake.api.on.mock.calls[0]![1];
    await hook.unmount();
    listener({ source: "tray_menu" });
    await act(async () => {
      if (outcome === "resolve") registration.resolve(fake.unsubscribe);
      else registration.reject(new Error("registration failed"));
    });
    expect(fake.unsubscribe).toHaveBeenCalledTimes(outcome === "resolve" ? 1 : 0);
    expect(hook.showToast).not.toHaveBeenCalled();
    expect(fake.api.appActiveWorkSnapshot).not.toHaveBeenCalled();
  });

  it("reports listener registration failure while mounted", async () => {
    const fake = fakeApi();
    fake.api.on.mockRejectedValueOnce(new Error("registration failed"));
    const hook = await mountExit(fake.api);
    expect(hook.showToast).toHaveBeenCalledExactlyOnceWith("창 닫기 동작을 연결하지 못했습니다.");
  });

  it("does not continue a failed status recheck after unmount", async () => {
    const fake = fakeApi();
    const refresh = deferred<ApiResult<AppActiveWorkSnapshot>>();
    fake.api.appActiveWorkSnapshot.mockResolvedValueOnce(unavailable).mockReturnValueOnce(refresh.promise);
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => hook.current.dialogProps.onQuit());
    await hook.unmount();
    await act(async () => refresh.reject(new Error("offline")));
    expect(hook.showToast).not.toHaveBeenCalled();
    expect(fake.api.appQuit).not.toHaveBeenCalled();
  });

  it.each(["quit", "tray"] as const)("ignores a pending %s failure after unmount", async (action) => {
    const fake = fakeApi();
    const pending = deferred<never>();
    if (action === "quit") fake.api.appQuit.mockReturnValueOnce(pending.promise);
    else fake.api.appMinimizeToTray.mockReturnValueOnce(pending.promise);
    const hook = await mountExit(fake.api);
    await act(async () => hook.current.openExitConfirm());
    await act(async () => {
      if (action === "quit") hook.current.dialogProps.onQuit();
      else hook.current.dialogProps.onMinimizeToTray();
    });
    await hook.unmount();
    await act(async () => pending.reject(new Error("too late")));
    expect(hook.showToast).not.toHaveBeenCalled();
  });

  it("cleans the superseded Strict Mode listener even when registration resolves late", async () => {
    const fake = fakeApi();
    const first = deferred<() => void>();
    const firstUnsubscribe = vi.fn();
    fake.api.on.mockReturnValueOnce(first.promise);
    const hook = await mountExit(fake.api, true);
    expect(fake.api.on).toHaveBeenCalledTimes(2);
    const oldListener = fake.api.on.mock.calls[0]![1];
    await act(async () => {
      oldListener({ source: "window_close" });
      first.resolve(firstUnsubscribe);
    });
    expect(firstUnsubscribe).toHaveBeenCalledOnce();
    expect(fake.api.appActiveWorkSnapshot).not.toHaveBeenCalled();
    await act(async () => fake.requestExit());
    expect(hook.current.open).toBe(true);
    expect(fake.api.appActiveWorkSnapshot).toHaveBeenCalledOnce();
    await hook.unmount();
    expect(fake.unsubscribe).toHaveBeenCalledOnce();
  });
});
