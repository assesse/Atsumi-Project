import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { OfficialBrowserPanelProps } from "./OfficialBrowserPanel";
import { StreamingWorkspace, type StreamingWorkspaceProps } from "./StreamingWorkspace";

const calls = vi.hoisted(() => ({ panel: vi.fn(), mado: vi.fn(), invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: calls.invoke }));
vi.mock("./OfficialBrowserPanel", () => ({ OfficialBrowserPanel: (props: OfficialBrowserPanelProps) => {
  calls.panel(props);
  return <section aria-label="official panel fixture" data-view={props.view} data-runtime={props.runtime} data-private={props.privacyMode}>
    <button type="button" onClick={props.onMadoMode}>마도</button>
  </section>;
} }));
vi.mock("./MadoWorkspace", () => ({ MadoWorkspace: (props: { runtime: string; privacy: boolean; onLeave: () => void }) => {
  calls.mado(props);
  return <section aria-label="mado fixture" data-runtime={props.runtime} data-private={props.privacy}><button type="button" onClick={props.onLeave}>마도 나가기</button></section>;
} }));

const baseProps: StreamingWorkspaceProps = { runtime: "tauri", active: true, railCollapsed: false, onToggleRail: vi.fn(), onSourceChange: vi.fn(), privacyMode: false };
const navigate = async (container: HTMLElement, label: string) => {
  const button = [...container.querySelectorAll<HTMLButtonElement>("button")].find((entry) => entry.getAttribute("aria-label") === label || entry.textContent === label);
  if (!button) throw new Error(`Missing button ${label}`);
  await act(async () => button.click());
};
const fixture = () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  return { container, render: (props: Partial<StreamingWorkspaceProps> = {}) => act(async () => root.render(<StreamingWorkspace {...baseProps} {...props} />)), close: () => act(async () => root.unmount()) };
};

beforeEach(() => { vi.clearAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); });
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe("official CHZZK workspace composition", () => {
  it("uses only the official panel for live and recording archive views", async () => {
    const view = fixture();
    try {
      await view.render();
      expect(view.container.querySelector('[aria-label="official panel fixture"]')).toHaveAttribute("data-view", "live");
      expect(view.container.querySelector(".streaming-workspace")).toHaveClass("is-official-view");
      expect(view.container.querySelector(".main-nav")).not.toHaveTextContent("Auto Find");
      await navigate(view.container, "녹화 목록");
      expect(view.container.querySelector('[aria-label="official panel fixture"]')).toHaveAttribute("data-view", "recordings");
      expect(view.container.querySelector(".streaming-workspace")).not.toHaveClass("is-official-view");
      await navigate(view.container, "라이브");
      expect(view.container.querySelectorAll('[aria-label="official panel fixture"]')).toHaveLength(1);
      expect(view.container.querySelector("video,.streaming-player,.streaming-chat,.streaming-current,.streaming-connect")).toBeNull();
    } finally { await view.close(); }
  });

  it("passes runtime and privacy to official and Mado presentation without mounting both", async () => {
    const view = fixture();
    try {
      await view.render({ runtime: "browser-mock", privacyMode: true, railCollapsed: true });
      expect(view.container.querySelector(".streaming-shell")).toHaveClass("sidebar-collapsed");
      expect(calls.panel).toHaveBeenLastCalledWith(expect.objectContaining({ runtime: "browser-mock", active: true, privacyMode: true }));
      await navigate(view.container, "마도");
      expect(view.container.querySelector('[aria-label="official panel fixture"]')).toBeNull();
      expect(calls.mado).toHaveBeenLastCalledWith(expect.objectContaining({ runtime: "browser-mock", privacy: true }));
      await navigate(view.container, "녹화 목록");
      expect(view.container.querySelector('[aria-label="mado fixture"]')).toBeNull();
      expect(view.container.querySelector('[aria-label="official panel fixture"]')).toHaveAttribute("data-view", "recordings");
      await navigate(view.container, "라이브");
      expect(view.container.querySelector('[aria-label="mado fixture"]')).not.toBeNull();
      await navigate(view.container, "마도 나가기");
      expect(view.container.querySelector('[aria-label="mado fixture"]')).toBeNull();
      expect(view.container.querySelector('[aria-label="official panel fixture"]')).toHaveAttribute("data-view", "live");
    } finally { await view.close(); }
  });

  it("detaches inactive presentation while retaining archive navigation on return", async () => {
    const view = fixture();
    try {
      await view.render({ active: false });
      expect(view.container).toBeEmptyDOMElement();
      expect(calls.panel).not.toHaveBeenCalled();
      expect(calls.mado).not.toHaveBeenCalled();
      await view.render();
      await navigate(view.container, "녹화 목록");
      const renders = calls.panel.mock.calls.length;
      await view.render({ active: false });
      expect(view.container).toBeEmptyDOMElement();
      expect(calls.panel).toHaveBeenCalledTimes(renders);
      await view.render();
      expect(view.container.querySelector('[aria-label="official panel fixture"]')).toHaveAttribute("data-view", "recordings");
    } finally { await view.close(); }
  });

  it("owns no legacy polling, network fetching, or session mutations across mode changes", async () => {
    vi.useFakeTimers();
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    const view = fixture();
    try {
      await view.render();
      await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
      await navigate(view.container, "녹화 목록");
      await view.render({ active: false });
      await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
      await view.render();
      expect(calls.invoke).not.toHaveBeenCalled();
      expect(fetch).not.toHaveBeenCalled();
      expect(vi.getTimerCount()).toBe(0);
      expect(view.container).not.toHaveTextContent("이전 방식");
    } finally { await view.close(); }
  });
});
