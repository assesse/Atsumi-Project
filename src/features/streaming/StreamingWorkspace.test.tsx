import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { StreamingWorkspace, type StreamingWorkspaceProps } from "./StreamingWorkspace";

const calls = vi.hoisted(() => ({ panel: vi.fn(), live: vi.fn(), reservations: vi.fn(), invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: calls.invoke }));
vi.mock("./OfficialBrowserPanel", () => ({ OfficialBrowserPanel: (props: { view: string }) => { calls.panel(props); return <section aria-label="archive" data-view={props.view} />; } }));
vi.mock("./MadoWorkspace", () => ({ MadoWorkspace: (props: { unifiedLive: boolean }) => { calls.live(props); return <section aria-label="unified live" data-unified={props.unifiedLive} />; } }));
vi.mock("./AutoRecordingPanel", () => ({ AutoRecordingPanel: (props: unknown) => { calls.reservations(props); return <section aria-label="reservations" />; } }));
const base: StreamingWorkspaceProps = { runtime: "tauri", active: true, railCollapsed: false, onToggleRail: vi.fn(), onSourceChange: vi.fn(), privacyMode: false };
const fixture = () => {
  const container = document.createElement("div"), root = createRoot(container);
  return { container, render: (props: Partial<StreamingWorkspaceProps> = {}) => act(async () => root.render(<StreamingWorkspace {...base} {...props} />)), close: () => act(async () => root.unmount()),
    navigate: async (label: string) => { const button = [...container.querySelectorAll("button")].find(b => b.textContent === label || b.getAttribute("aria-label") === label)!; await act(async () => button.click()); } };
};
beforeEach(() => { vi.clearAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); });
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });
describe("one live entry", () => {
  it("uses the shared receiver layout only in Live and has no archive/reservation watch route", async () => {
    const view = fixture();
    try {
      await view.render();
      expect(view.container.querySelector('[aria-label="unified live"]')).toHaveAttribute("data-unified", "true");
      expect(calls.panel).not.toHaveBeenCalled();
      await view.navigate("자동 녹화");
      expect(view.container.querySelector('[aria-label="unified live"]')).toBeNull();
      expect(calls.reservations).toHaveBeenLastCalledWith({ runtime: "tauri", privacy: false });
      await view.navigate("녹화 목록");
      expect(view.container.querySelector('[aria-label="archive"]')).toHaveAttribute("data-view", "recordings");
      await view.navigate("라이브");
      expect(view.container.querySelectorAll('[aria-label="unified live"]')).toHaveLength(1);
      expect(view.container.querySelector('[aria-label="archive"]')).toBeNull();
      expect(calls.invoke).not.toHaveBeenCalled();
    } finally { await view.close(); }
  });
  it("passes privacy/runtime and unmounts hidden presentation, retaining the selected tab", async () => {
    const view = fixture();
    try {
      await view.render({ runtime: "browser-mock", privacyMode: true, railCollapsed: true });
      expect(calls.live).toHaveBeenLastCalledWith(expect.objectContaining({ runtime: "browser-mock", privacy: true, unifiedLive: true }));
      expect(view.container.querySelector(".streaming-shell")).toHaveClass("sidebar-collapsed");
      await view.navigate("녹화 목록");
      await view.render({ active: false }); expect(view.container).toBeEmptyDOMElement();
      await view.render(); expect(view.container.querySelector('[aria-label="archive"]')).toHaveAttribute("data-view", "recordings");
    } finally { await view.close(); }
  });
  it("does not create streams or mutate recording sessions during tab navigation", async () => {
    vi.useFakeTimers(); const fetch = vi.fn(); vi.stubGlobal("fetch", fetch); const view = fixture();
    try {
      await view.render(); await view.navigate("자동 녹화"); await view.navigate("라이브");
      await view.render({ active: false }); await act(async () => vi.advanceTimersByTimeAsync(60_000));
      expect(calls.invoke).not.toHaveBeenCalled(); expect(fetch).not.toHaveBeenCalled(); expect(vi.getTimerCount()).toBe(0);
    } finally { await view.close(); }
  });
});
