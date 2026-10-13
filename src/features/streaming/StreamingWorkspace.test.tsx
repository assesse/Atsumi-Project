import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { StreamingWorkspace, type StreamingWorkspaceProps } from "./StreamingWorkspace";

const calls = vi.hoisted(() => ({ panel: vi.fn(), live: vi.fn(), screenshots: vi.fn(), reservations: vi.fn(), invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: calls.invoke }));
vi.mock("./OfficialBrowserPanel", () => ({ OfficialBrowserPanel: (props: { view: string }) => { calls.panel(props); return <section aria-label="archive" data-view={props.view} />; } }));
vi.mock("./MadoWorkspace", () => ({ MadoWorkspace: (props: { unifiedLive: boolean }) => { calls.live(props); return <section aria-label="unified live" data-unified={props.unifiedLive} />; } }));
vi.mock("./ScreenshotLibrary", () => ({ ScreenshotLibrary: (props: unknown) => { calls.screenshots(props); return <section aria-label="screenshots" />; } }));
vi.mock("./AutoRecordingPanel", () => ({ AutoRecordingPanel: (props: unknown) => { calls.reservations(props); return <section aria-label="reservations" />; } }));
const base: StreamingWorkspaceProps = { runtime: "tauri", active: true, railCollapsed: false, onToggleRail: vi.fn(), onSourceChange: vi.fn() };
const fixture = () => {
  const container = document.createElement("div"), root = createRoot(container);
  return { container, render: (props: Partial<StreamingWorkspaceProps> = {}) => act(async () => root.render(<StreamingWorkspace {...base} {...props} />)), close: () => act(async () => root.unmount()),
    navigate: async (label: string) => { const button = [...container.querySelectorAll("button")].find(b => b.textContent === label || b.getAttribute("aria-label") === label)!; await act(async () => button.click()); } };
};
beforeEach(() => { vi.clearAllMocks(); localStorage.removeItem("atsumi.chzzk.view.v1"); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); });
afterEach(() => { vi.restoreAllMocks(); localStorage.removeItem("atsumi.chzzk.view.v1"); vi.useRealTimers(); vi.unstubAllGlobals(); });
describe("one live entry", () => {
  it("keeps live navigation separate from the bottom connection drawer", async () => {
    const view = fixture();
    try {
      await view.render();
      expect(view.container.querySelector('main')).toHaveClass('is-unified-live');
      expect(view.container.querySelector('.main-nav [data-tour="chzzk-channels"]')).toBeNull();
      await view.navigate('녹화 목록');
      expect(view.container.querySelector('main')).not.toHaveClass('is-unified-live');
      await view.navigate('라이브');
      expect(calls.live.mock.calls.at(-1)![0].connectionRequest).toBeUndefined();
      expect(view.container.querySelector('.main-nav [aria-label="라이브"]')).toHaveAttribute('aria-current', 'page');
      expect(view.container.querySelector('.main-nav [aria-label="라이브"]')).not.toHaveAttribute('aria-expanded');
      expect(calls.invoke).not.toHaveBeenCalled();
    } finally { await view.close(); }
  });
  it("auto-hides navigation only while watching, keeps tutorial visible and preserves the ordinary rail preference", async () => {
    const view = fixture();
    try {
      await view.render({ railCollapsed: true });
      const watch = calls.live.mock.calls.at(-1)![0].onWatchingChange as (watching: boolean) => void;
      await act(async () => watch(true));
      expect(view.container.querySelector('.streaming-shell')).toHaveClass('is-live-immersive');
      expect(view.container.querySelector('.streaming-shell')).not.toHaveClass('sidebar-collapsed');
      expect(view.container.querySelector('.edge-left .live-edge-overlay')).toHaveAttribute('hidden');
      expect(view.container.querySelector('[aria-label="메뉴 열기"]')).not.toBeNull();
      await view.render({ railCollapsed: true, tutorialActive: true });
      expect(view.container.querySelector('.streaming-shell')).not.toHaveClass('is-live-immersive');
      expect(view.container.querySelector('.streaming-shell')).toHaveClass('sidebar-collapsed');
      await view.render({ railCollapsed: true });
      await view.navigate('녹화 목록');
      expect(view.container.querySelector('.streaming-shell')).not.toHaveClass('is-live-immersive');
      expect(view.container.querySelector('.streaming-shell')).toHaveClass('sidebar-collapsed');
      expect(base.onToggleRail).not.toHaveBeenCalled();
    } finally { await view.close(); }
  });
  it("uses the shared receiver layout only in Live and has no archive/reservation watch route", async () => {
    const view = fixture();
    try {
      await view.render();
      expect(view.container.querySelector('[aria-label="unified live"]')).toHaveAttribute("data-unified", "true");
      expect(calls.panel).not.toHaveBeenCalled();
      expect([...view.container.querySelectorAll("button")].some(button => button.textContent === "자동 녹화")).toBe(false);
      expect(calls.reservations).not.toHaveBeenCalled();
      await view.navigate("녹화 목록");
      expect(view.container.querySelector('[aria-label="archive"]')).toHaveAttribute("data-view", "recordings");
      await view.navigate("라이브");
      expect(view.container.querySelectorAll('[aria-label="unified live"]')).toHaveLength(1);
      expect(view.container.querySelector('[aria-label="archive"]')).toBeNull();
      expect(calls.invoke).not.toHaveBeenCalled();
    } finally { await view.close(); }
  });
  it("applies shared privacy to live and recordings and retains the selected section", async () => {
    const view = fixture();
    try {
      await view.render({ runtime: "browser-mock", railCollapsed: true, privacyMode: true });
      expect(calls.live).toHaveBeenLastCalledWith(expect.objectContaining({ runtime: "browser-mock", privacy: true, unifiedLive: true }));
      expect(view.container.querySelector(".streaming-shell")).toHaveClass("sidebar-collapsed");
      await view.navigate("녹화 목록");
      expect(calls.panel).toHaveBeenLastCalledWith(expect.objectContaining({ privacyMode: true }));
      await view.render({ active: false }); expect(view.container).toBeEmptyDOMElement();
      await view.render(); expect(view.container.querySelector('[aria-label="archive"]')).toHaveAttribute("data-view", "recordings");
    } finally { await view.close(); }
  });
  it("opens screenshots above recordings without creating a player and forwards privacy", async () => {
    const view = fixture();
    try {
      await view.render({ privacyMode: true, navigationRequest: { source: "chzzk", view: "screenshots", sequence: 1 } });
      const labels = [...view.container.querySelectorAll('.main-nav button')].map(button => button.getAttribute('aria-label'));
      expect(labels.indexOf('스크린샷')).toBeLessThan(labels.indexOf('녹화 목록'));
      expect(calls.screenshots).toHaveBeenLastCalledWith(expect.objectContaining({ runtime: "tauri", privacy: true }));
      expect(calls.live).not.toHaveBeenCalled(); expect(calls.panel).not.toHaveBeenCalled();
      expect(calls.invoke).not.toHaveBeenCalled();
      await view.navigate('녹화 목록');
      expect(view.container.querySelector('[aria-label="screenshots"]')).toBeNull();
    } finally { await view.close(); }
  });
  it.each([
    ["라이브", "live", "unified live"],
    ["스크린샷", "screenshots", "screenshots"],
    ["녹화 목록", "recordings", "archive"],
  ])("restores %s after a new workspace is mounted", async (label, saved, target) => {
    const first = fixture();
    try { await first.render(); await first.navigate(label); } finally { await first.close(); }
    expect(localStorage.getItem("atsumi.chzzk.view.v1")).toBe(saved);
    vi.clearAllMocks();
    const reopened = fixture();
    try {
      await reopened.render();
      expect(reopened.container.querySelector(`[aria-label="${target}"]`)).not.toBeNull();
      if (saved !== "live") expect(calls.live).not.toHaveBeenCalled();
      expect(calls.invoke).not.toHaveBeenCalled();
    } finally { await reopened.close(); }
  });
  it("uses an explicit destination before rendering the saved tab and follows later requests", async () => {
    localStorage.setItem("atsumi.chzzk.view.v1", "live");
    const view = fixture();
    try {
      await view.render({ navigationRequest: { source: "chzzk", view: "recordings", sequence: 1 } });
      expect(view.container.querySelector('[aria-label="archive"]')).not.toBeNull();
      expect(calls.live).not.toHaveBeenCalled();
      expect(localStorage.getItem("atsumi.chzzk.view.v1")).toBe("recordings");
      await view.render({ navigationRequest: { source: "chzzk", view: "live", sequence: 2 } });
      expect(view.container.querySelector('[aria-label="unified live"]')).not.toBeNull();
      expect(localStorage.getItem("atsumi.chzzk.view.v1")).toBe("live");
      await view.render({ navigationRequest: { source: "hitomi", view: "explore", sequence: 3 } });
      expect(view.container.querySelector('[aria-label="unified live"]')).not.toBeNull();
    } finally { await view.close(); }
  });
  it.each(["", "unknown", "auto-record", '{"view":"recordings"}'])("uses Live for a legacy or invalid saved tab %j", async saved => {
    localStorage.setItem("atsumi.chzzk.view.v1", saved);
    const view = fixture();
    try { await view.render(); expect(view.container.querySelector('[aria-label="unified live"]')).not.toBeNull(); }
    finally { await view.close(); }
  });
  it("keeps navigation available when reading and saving preferences fails", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("storage unavailable"); });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("storage unavailable"); });
    const view = fixture();
    try {
      await view.render();
      expect(view.container.querySelector('[aria-label="unified live"]')).not.toBeNull();
      await view.navigate("녹화 목록");
      expect(view.container.querySelector('[aria-label="archive"]')).not.toBeNull();
    } finally { await view.close(); }
  });
  it("does not create streams or mutate recording sessions during tab navigation", async () => {
    vi.useFakeTimers(); const fetch = vi.fn(); vi.stubGlobal("fetch", fetch); const view = fixture();
    try {
      await view.render(); await view.navigate("녹화 목록"); await view.navigate("라이브");
      await view.render({ active: false }); await act(async () => vi.advanceTimersByTimeAsync(60_000));
      expect(calls.invoke).not.toHaveBeenCalled(); expect(fetch).not.toHaveBeenCalled(); expect(vi.getTimerCount()).toBe(0);
    } finally { await view.close(); }
  });
});
