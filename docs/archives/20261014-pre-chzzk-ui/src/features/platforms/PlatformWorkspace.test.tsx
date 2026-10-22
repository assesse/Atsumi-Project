import { act, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { PlatformWorkspace, type PlatformSource } from "./PlatformWorkspace";
import workspaceCss from "./PlatformWorkspace.css?raw";
import edgeCss from "../streaming/LiveEdgePanel.css?raw";

const fixture = vi.hoisted(() => ({ tutorial: false, collapsed: false, mount: vi.fn(), unmount: vi.fn() }));
vi.mock("../../app/AppShell", () => ({ useAppShell: () => ({
  privacyMode: false, privacyModePending: false, tutorialOpen: fixture.tutorial,
  railCollapsed: fixture.collapsed, showToast: vi.fn(), toggleRail: vi.fn(), setSettingsOpen: vi.fn(), togglePrivacyMode: vi.fn(),
}) }));
vi.mock("../../components/SideRail", () => ({ SideRail: ({ onNavigate, collapsed }: { onNavigate: (view: string) => void; collapsed: boolean }) =>
  <nav className="sidebar" data-collapsed={collapsed}>{["explore", "live", "favorites", "screenshots", "downloads"].map(view => <button key={view} onClick={() => onNavigate(view)}>{view}</button>)}</nav>,
}));
vi.mock("./ReloadableView", () => ({ ReloadableView: ({ viewProps }: { viewProps: { view: string; onWatchingChange?: (watching: boolean) => void; onViewChange?: (view: string) => void } }) => {
  useEffect(() => { fixture.mount(); return () => { fixture.unmount(); }; }, []);
  return <section data-testid="provider" data-view={viewProps.view}><button onClick={() => viewProps.onWatchingChange?.(true)}>start</button><button onClick={() => viewProps.onWatchingChange?.(false)}>stop</button><button onClick={() => viewProps.onViewChange?.("explore")}>open creator</button><video /></section>;
} }));
let root: Root, host: HTMLDivElement;
beforeEach(() => { fixture.tutorial = false; fixture.collapsed = false; fixture.mount.mockClear(); fixture.unmount.mockClear(); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
const render = async (active = true, source: PlatformSource = "soop") => act(async () => root.render(<PlatformWorkspace source={source} active={active} runtime="browser-mock" onSourceChange={vi.fn()} />));
const clickText = async (text: string) => act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === text)!.click());

it("connects YouTube favorite creator navigation to Explore without remounting its workspace", async () => {
  await render(true, "youtube"); const video = host.querySelector("video");
  await clickText("favorites"); expect(host.querySelector('[data-testid="provider"]')).toHaveAttribute("data-view", "favorites");
  await clickText("open creator"); expect(host.querySelector('[data-testid="provider"]')).toHaveAttribute("data-view", "explore");
  expect(host.querySelector("video")).toBe(video); expect(fixture.mount).toHaveBeenCalledTimes(1); expect(fixture.unmount).not.toHaveBeenCalled();
});

it.each(["soop", "youtube"] as const)("%s shows a persistent edge handle only in the watching view and never remounts media", async source => {
  await render(true, source); const video = host.querySelector("video"), rail = host.querySelector("nav");
  await clickText("live"); expect(host.querySelector(".live-edge-trigger")).toBeNull();
  await clickText("start"); expect(host.querySelector(".platform-shell")).toHaveClass("is-live-immersive");
  const trigger = host.querySelector<HTMLButtonElement>(".live-edge-trigger")!;
  expect(trigger).toHaveAccessibleName("메뉴 열기"); expect(host.querySelector(".live-edge-overlay")).toHaveAttribute("hidden");
  await act(async () => trigger.click()); expect(trigger).toHaveAccessibleName("메뉴 닫기");
  expect(host.querySelector(".live-edge-overlay")).not.toHaveAttribute("hidden");
  await clickText("downloads"); expect(host.querySelector(".platform-shell")).not.toHaveClass("is-live-immersive");
  expect(host.querySelector(".live-edge-trigger")).toBeNull(); expect(host.querySelector("nav")).toBe(rail);
  await clickText("screenshots"); expect(host.querySelector('[data-testid="provider"]')).toHaveAttribute("data-view", "screenshots");
  expect(host.querySelector(".platform-shell")).not.toHaveClass("is-live-immersive");
  expect(host.querySelector("video")).toBe(video); expect(fixture.unmount).not.toHaveBeenCalled();
  await clickText("live"); expect(host.querySelector(".live-edge-trigger")).toHaveAccessibleName("메뉴 열기");
  await render(false, source); expect(host.querySelector(".platform-shell")).toHaveAttribute("hidden");
  await render(true, source); expect(host.querySelector("video")).toBe(video); expect(fixture.mount).toHaveBeenCalledTimes(1); expect(fixture.unmount).not.toHaveBeenCalled();
  await clickText("stop"); expect(host.querySelector(".platform-shell")).not.toHaveClass("is-live-immersive"); expect(host.querySelector("nav")).toBe(rail);
});

it("keeps tutorial navigation visible and restores the collapsed preference after viewing", async () => {
  fixture.collapsed = true; await render(); await clickText("live"); await clickText("start");
  expect(host.querySelector("nav")).toHaveAttribute("data-collapsed", "false");
  fixture.tutorial = true; await render(); expect(host.querySelector(".live-edge-trigger")).toBeNull(); expect(host.querySelector("nav")).toHaveAttribute("data-collapsed", "true");
  fixture.tutorial = false; await render(); expect(host.querySelector(".live-edge-trigger")).not.toBeNull();
  await clickText("stop"); expect(host.querySelector(".platform-shell")).toHaveClass("sidebar-collapsed");
});

it.each(["pinterest", "pixiv"] as const)("%s never hides its sidebar as a live workspace", async source => {
  await render(true, source); await clickText("start"); expect(host.querySelector(".live-edge-trigger")).toBeNull();
});

it("keeps the overlay outside layout and the handle next to the opened rail without looping animation", () => {
  expect(workspaceCss).toMatch(/\.platform-shell\.is-live-immersive\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  expect(edgeCss).toContain(".live-edge-panel.edge-left.is-open>.live-edge-trigger{left:var(--live-rail-width)}");
  expect(edgeCss).not.toMatch(/animation:/);
});
