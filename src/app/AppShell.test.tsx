import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiResult, AppExitRequestedEvent, SettingsSnapshot } from "../api/contracts";
import { AppShell, useAppShell, type AppShellApi } from "./AppShell";

const initialSettings: SettingsSnapshot = {
  revision: 42,
  downloadRoot: "",
  folderNameTemplate: "[{artist}] {title} [{group}] {id}",
  autoFindHistoryMode: "include_all_history",
  downloadOverlapAutoMode: "off",
  explorePageSize: 50,
  danbooruPageSize: 60,
  maxColumns: 3,
  previewWidth: 220,
  danbooruPreviewWidth: 190,
  relatedPreviewWidth: 240,
  privacyMode: false,
  cacheLimitGb: 10,
  concurrentImageRequests: 5,
  downloadAdaptiveConcurrency: true,
  downloadAdaptiveMaxRequests: 8,
  requestStartIntervalMs: 25,
  autoFindGrouping: "all",
  downloadsGrouping: "all",
  exploreDisplayMode: "detail",
  autoFindDisplayMode: "detail",
  downloadsDisplayMode: "detail",
  collapsedGroupKeys: [],
  searchIncludeTags: [],
  searchExcludeTags: [],
};

const success = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
type SubscriptionArgs =
  | [event: "settings:changed", listener: (snapshot: SettingsSnapshot) => void]
  | [event: "app:exit-requested", listener: (event: AppExitRequestedEvent) => void];

function fakeShellApi() {
  let settings = initialSettings;
  const settingsListeners = new Set<(snapshot: SettingsSnapshot) => void>();
  const exitListeners = new Set<(event: AppExitRequestedEvent) => void>();
  const unsubscribeSettings = vi.fn(() => settingsListeners.clear());
  const unsubscribeExit = vi.fn(() => exitListeners.clear());
  const api = {
    runtime: "browser-mock",
    settingsGet: vi.fn<AppShellApi["settingsGet"]>().mockImplementation(async () => success(settings)),
    settingsUpdate: vi.fn<AppShellApi["settingsUpdate"]>().mockImplementation(async (patch) => {
      settings = { ...settings, ...patch, revision: settings.revision + 1 };
      return success(settings);
    }),
    appActiveWorkSnapshot: vi.fn<AppShellApi["appActiveWorkSnapshot"]>().mockResolvedValue(success({
      queriedAt: "2026-09-10T00:00:00Z", workSetFingerprint: "idle", downloads: { activeCount: 0 },
    })),
    appMinimizeToTray: vi.fn<AppShellApi["appMinimizeToTray"]>().mockResolvedValue(success(null)),
    appQuit: vi.fn<AppShellApi["appQuit"]>().mockResolvedValue(success({ accepted: true })),
    on: vi.fn(async (...[event, listener]: SubscriptionArgs) => {
      if (event === "settings:changed") {
        settingsListeners.add(listener);
        return unsubscribeSettings;
      }
      exitListeners.add(listener);
      return unsubscribeExit;
    }),
  } satisfies AppShellApi;
  return {
    api, settingsListeners, exitListeners, unsubscribeSettings, unsubscribeExit,
    emitSettings: (snapshot: SettingsSnapshot) => {
      settings = snapshot;
      settingsListeners.forEach((listener) => listener(snapshot));
    },
    requestExit: () => exitListeners.forEach((listener) => listener({ source: "window_close" })),
  };
}

type Shell = ReturnType<typeof useAppShell>;
function Probe({ onReady }: { onReady: (shell: Shell) => void }) {
  const shell = useAppShell();
  onReady(shell);
  return <output aria-label="active workspace">{shell.source}</output>;
}

const disposers: Array<() => Promise<void>> = [];
const storageKeys = ["atsumi.content-source.v1", "atsumi.tutorial.dismissed.v1"];
const previousStorage = new Map<string, string | null>();
const originalShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");

async function mountShell(api: AppShellApi) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let shell!: Shell;
  let unmounted = false;
  const unmount = async () => {
    if (unmounted) return;
    unmounted = true;
    await act(async () => root.unmount());
    container.remove();
  };
  disposers.push(unmount);
  await act(async () => {
    root.render(<AppShell api={api}><Probe onReady={(value) => { shell = value; }} /></AppShell>);
  });
  return { get current() { return shell; }, container, unmount };
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0));
  vi.stubGlobal("cancelAnimationFrame", (id: number) => window.clearTimeout(id));
  storageKeys.forEach((key) => previousStorage.set(key, window.localStorage.getItem(key)));
  window.localStorage.removeItem("atsumi.content-source.v1");
  window.localStorage.setItem("atsumi.tutorial.dismissed.v1", "true");
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value(this: HTMLDialogElement) { this.setAttribute("open", ""); },
  });
});

afterEach(async () => {
  for (const dispose of disposers.splice(0)) await dispose();
  if (originalShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", originalShowModal);
  else Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
  previousStorage.forEach((value, key) => {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  });
  previousStorage.clear();
  vi.unstubAllGlobals();
});

describe("AppShell without feature workspaces", () => {
  it("preserves shared state and one settings/exit subscription across mode changes", async () => {
    const fake = fakeShellApi();
    const shell = await mountShell(fake.api);
    expect(shell.current.settingsStore).toMatchObject({ settings: initialSettings, loading: false, error: null });
    expect(fake.api.settingsGet).toHaveBeenCalledOnce();
    expect(shell.current.source).toBe("hitomi");

    await act(async () => {
      shell.current.toggleRail();
      shell.current.setSettingsOpen(true);
      shell.current.setActivityOpen(true);
      shell.current.selectSource("danbooru");
    });
    expect(shell.current).toMatchObject({ source: "danbooru", railCollapsed: true, settingsOpen: true, activityOpen: true });
    expect(shell.container.querySelector("output")).toHaveTextContent("danbooru");
    expect(window.localStorage.getItem("atsumi.content-source.v1")).toBe("danbooru");

    await act(async () => shell.current.togglePrivacyMode());
    expect(fake.api.settingsUpdate).not.toHaveBeenCalled();
    expect(document.documentElement).toHaveAttribute("data-privacy-mode", "off");

    await act(async () => {
      shell.current.selectSource("hitomi");
      fake.requestExit();
    });
    expect(shell.current).toMatchObject({ source: "hitomi", railCollapsed: true, settingsOpen: true, activityOpen: true, exitConfirmOpen: true });
    expect(shell.current.privacyMode).toBe(false);
    expect(shell.current.settingsStore.settings.privacyMode).toBe(false);
    expect(shell.container.querySelector(".exit-dialog")).toHaveAttribute("open");
    expect(fake.api.appActiveWorkSnapshot).toHaveBeenCalledOnce();

    await act(async () => {
      shell.current.selectSource("danbooru");
      fake.emitSettings({ ...initialSettings, revision: 44, privacyMode: false });
    });
    expect(shell.current.exitConfirmOpen).toBe(true);
    expect(document.documentElement).toHaveAttribute("data-privacy-mode", "off");
    expect(fake.api.settingsGet).toHaveBeenCalledOnce();
    expect(fake.api.on.mock.calls.map(([event]) => event).sort()).toEqual(["app:exit-requested", "settings:changed"]);
    expect(fake.settingsListeners.size).toBe(1);
    expect(fake.exitListeners.size).toBe(1);

    await shell.unmount();
    expect(fake.unsubscribeSettings).toHaveBeenCalledOnce();
    expect(fake.unsubscribeExit).toHaveBeenCalledOnce();
    expect(fake.settingsListeners.size).toBe(0);
    expect(fake.exitListeners.size).toBe(0);
    expect(document.documentElement).toHaveAttribute("data-privacy-mode", "off");
  });

  it("does not overwrite the current privacy choice when the startup setting changes", async () => {
    const fake = fakeShellApi(), shell = await mountShell(fake.api);
    expect(shell.current.privacyMode).toBe(true);
    await act(async () => shell.current.togglePrivacyMode());
    expect(shell.current.privacyMode).toBe(false);
    await act(async () => fake.emitSettings({...initialSettings,revision:43,privacyOnStartup:true}));
    expect(shell.current.privacyMode).toBe(false);
    await shell.unmount();
    const next=await mountShell(fake.api);
    expect(next.current.privacyMode).toBe(true);
    expect(fake.api.settingsUpdate).not.toHaveBeenCalled();
  });
  it("honors an explicitly disabled startup mask", async () => {
    const fake=fakeShellApi();
    fake.api.settingsGet.mockResolvedValueOnce(success({...initialSettings,privacyOnStartup:false}));
    const shell=await mountShell(fake.api);
    expect(shell.current.privacyMode).toBe(false);
    await act(async()=>shell.current.togglePrivacyMode());
    expect(shell.current.privacyMode).toBe(true);
  });
});
