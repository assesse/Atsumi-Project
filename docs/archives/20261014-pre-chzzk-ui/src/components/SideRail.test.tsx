import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { SideRail } from "./SideRail";
import { CommonNavigationContext } from "../app/CommonNavigation";
import { OPEN_FEEDBACK } from "../features/feedback/api";

describe("SideRail source switcher", () => {
  it("opens live settings from the whole navigation row including its disclosure", async () => {
    const host = document.createElement("div"), root = createRoot(host);
    const navigate = vi.fn(), toggle = vi.fn();
    try {
      for (const collapsed of [false, true]) for (const open of [false, true]) {
        await act(async () => root.render(<SideRail source="chzzk" view="live" collapsed={collapsed} autoFindCount={0} attentionCount={0} onNavigate={navigate} onSourceChange={vi.fn()} onToggle={vi.fn()} onOpenLiveChannels={toggle} liveChannelsOpen={open} />));
        const row = host.querySelector('.nav-live-item')!;
        expect(row.querySelectorAll('button')).toHaveLength(1);
        expect(host.querySelector('[aria-label="방송 보기"]')).toBeNull();
        const arrow = row.querySelector<HTMLElement>('.nav-live-disclosure')!;
        expect(row.querySelector('button')).toHaveAttribute('aria-expanded', String(open));
        expect(arrow).toHaveAttribute('aria-hidden', 'true');
        navigate.mockClear(); toggle.mockClear();
        await act(async () => arrow.click());
        expect(toggle).toHaveBeenCalledOnce(); expect(navigate).not.toHaveBeenCalled();
        await act(async () => row.querySelector<HTMLButtonElement>('.nav-item')!.click());
        expect(navigate).not.toHaveBeenCalled(); expect(toggle).toHaveBeenCalledTimes(2);
      }
    } finally { await act(async () => root.unmount()); }
  });
  it("uses the boundary to fold the rail and keeps privacy/settings stacked at the bottom", async () => {
    const host = document.createElement("div"), root = createRoot(host);
    const toggle = vi.fn(), privacy = vi.fn(), settings = vi.fn();
    const render = async (collapsed: boolean, pending = false) => {
      await act(async () => root.render(<SideRail source="hitomi" view="explore" collapsed={collapsed} autoFindCount={0} attentionCount={0} onNavigate={vi.fn()} onSourceChange={vi.fn()} onToggle={toggle} onSettings={settings} privacyMode privacyModePending={pending} onPrivacyModeToggle={privacy} />));
    };
    try {
      await render(false);
      const foot = host.querySelector(".sidebar-foot")!;
      expect([...foot.querySelectorAll("button")].map(node => node.getAttribute("aria-label"))).toEqual(["오류·의견 보내기", "프라이버시 모드", "설정"]);
      const feedback = vi.fn();
      window.addEventListener(OPEN_FEEDBACK, feedback, { once: true });
      await act(async () => foot.querySelector<HTMLButtonElement>('[aria-label="오류·의견 보내기"]')!.click());
      expect(feedback).toHaveBeenCalledOnce();
      expect(host.querySelector(".live-indicator, .sidebar-toggle")).toBeNull();
      expect(host.textContent).not.toMatch(/live|fixture|Community/);
      expect(foot.querySelector('[data-tour="privacy-mode"]')).toHaveAttribute("aria-pressed", "true");
      await act(async () => foot.querySelector<HTMLButtonElement>('[aria-label="프라이버시 모드"]')!.click());
      expect(privacy).toHaveBeenCalledOnce();
      const edge = host.querySelector<HTMLButtonElement>(".sidebar-boundary")!;
      expect(edge).toHaveAttribute("aria-expanded", "true");
      expect(foot.contains(edge)).toBe(false);
      await act(async () => edge.click()); expect(toggle).toHaveBeenCalledOnce();
      await render(true, true);
      expect(edge).toHaveAccessibleName("메뉴 펼치기");
      expect(edge).toHaveAttribute("aria-expanded", "false");
      const pending = foot.querySelector<HTMLButtonElement>('[aria-label="프라이버시 모드"]')!;
      expect(pending).toBeDisabled(); expect(pending).toHaveAttribute("aria-busy", "true");
      await act(async () => pending.click()); expect(privacy).toHaveBeenCalledOnce();
      await act(async () => foot.querySelector<HTMLButtonElement>('[data-tour="hitomi-settings"]')!.click());
      expect(settings).toHaveBeenCalledOnce();
    } finally { await act(async () => root.unmount()); }
  });
  it("keeps community for album sources only and integrates automatic recording into live", async () => {
    const host = document.createElement("div"), root = createRoot(host);
    const openCommunity = vi.fn();
    try {
      for (const source of ["chzzk", "hitomi", "danbooru"] as const) {
        await act(async () => root.render(<CommonNavigationContext.Provider value={{ communityOpen: false, openCommunity }}><SideRail source={source} view={source === "chzzk" ? "live" : "explore"} collapsed={false} autoFindCount={0} attentionCount={0} onNavigate={vi.fn()} onSourceChange={vi.fn()} onToggle={vi.fn()} /></CommonNavigationContext.Provider>));
        const community = host.querySelector('[aria-label="커뮤니티"]');
        if (source === "chzzk") {
          expect(community).toBeNull();
          expect([...host.querySelectorAll('.main-nav button')].map(button => button.getAttribute('aria-label'))).toEqual(["라이브", "스크린샷", "녹화 목록"]);
        } else expect(community).not.toBeNull();
      }
    } finally { await act(async () => root.unmount()); }
  });
  it("exposes shared settings directly from CHZZK without navigating away", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onSettings = vi.fn();
    const onNavigate = vi.fn();
    const onPrivacyModeToggle = vi.fn();
    try {
      await act(async () => root.render(<SideRail source="chzzk" view="recordings" collapsed={false} autoFindCount={0} attentionCount={0} onNavigate={onNavigate} onSourceChange={vi.fn()} onToggle={vi.fn()} onSettings={onSettings} onPrivacyModeToggle={onPrivacyModeToggle} />));
      expect(container.querySelector('[aria-label="프라이버시 모드"]')).not.toBeNull();
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="프라이버시 모드"]')!.click());
      expect(onPrivacyModeToggle).toHaveBeenCalledOnce();
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="설정"]')!.click());
      expect(onSettings).toHaveBeenCalledOnce();
      expect(onNavigate).not.toHaveBeenCalled();
    } finally { await act(async () => root.unmount()); container.remove(); }
  });
  it("opens from the Atsumi banner and hides unsupported Danbooru navigation", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onSourceChange = vi.fn();
    try {
      await act(async () => root.render(
        <SideRail
          view="explore"
          collapsed={false}
          autoFindCount={0}
          attentionCount={2}
          source="danbooru"
          onNavigate={vi.fn()}
          onSourceChange={onSourceChange}
          onToggle={vi.fn()}
        />,
      ));
      expect(container.textContent).not.toContain("Auto Find");
      const banner = container.querySelector<HTMLButtonElement>('.brand[aria-haspopup="menu"]');
      await act(async () => banner?.click());
      expect(container.querySelector('[role="menu"]')).toBeInTheDocument();
      expect([...container.querySelectorAll('[role="menuitemradio"]')].map(option => option.textContent)).toEqual(["Hitomi", "Danbooru", "CHZZK", "YouTube", "SOOP", "Pinterest", "pixiv"]);
      const hitomi = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')]
        .find((button) => button.textContent?.includes("Hitomi"));
      await act(async () => hitomi?.click());
      expect(onSourceChange).toHaveBeenCalledWith("hitomi");
      expect(container.querySelector('[role="menu"]')).not.toBeInTheDocument();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("preserves Hitomi labels, navigation callbacks, and badge emphasis", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onNavigate = vi.fn();
    try {
      await act(async () => root.render(
        <SideRail
          view="auto-find"
          collapsed={false}
          autoFindCount={3}
          attentionCount={2}
          source="hitomi"
          onNavigate={onNavigate}
          onSourceChange={vi.fn()}
          onToggle={vi.fn()}
        />,
      ));
      expect(container.querySelector(".brand")).toHaveAttribute("aria-label", "현재 Hitomi 모드. 소스 전환");
      expect(container.querySelector(".brand-copy span")).toHaveTextContent("Hitomi");
      const buttons = [...container.querySelectorAll<HTMLButtonElement>(".main-nav button")];
      expect(buttons.map((button) => button.querySelector(".nav-label")?.textContent)).toEqual(["Explore", "Auto Find", "Downloads"]);
      expect(buttons[1]).toHaveAttribute("aria-current", "page");
      expect(buttons[1]?.querySelector(".nav-count")).toHaveTextContent("3");
      expect(buttons[1]?.querySelector(".nav-count")).not.toHaveClass("warning");
      expect(buttons[2]?.querySelector(".nav-count.warning")).toHaveTextContent("2");
      await act(async () => buttons[2]?.click());
      expect(onNavigate).toHaveBeenCalledWith("downloads");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("dismisses source choices with Escape or an outside pointer without switching", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onSourceChange = vi.fn();
    try {
      await act(async () => root.render(
        <SideRail
          view="explore"
          collapsed={false}
          autoFindCount={0}
          attentionCount={0}
          source="danbooru"
          onNavigate={(view: "explore" | "downloads") => { expect(view).toBeDefined(); }}
          onSourceChange={onSourceChange}
          onToggle={vi.fn()}
        />,
      ));
      expect(container.querySelector(".brand")).toHaveAttribute("aria-label", "현재 Danbooru 모드. 소스 전환");
      expect(container.querySelector(".brand-copy span")).toHaveTextContent("Danbooru");
      expect(container.querySelector(".nav-count")).not.toBeInTheDocument();
      const banner = container.querySelector<HTMLButtonElement>(".brand");
      await act(async () => banner?.click());
      const choices = [...container.querySelectorAll('[role="menuitemradio"]')];
      expect(choices.map((choice) => choice.textContent)).toEqual([
        "Hitomi",
        "Danbooru",
        "CHZZK",
        "YouTube",
        "SOOP",
        "Pinterest",
        "pixiv",
      ]);
      expect(choices.every(choice => choice.querySelector<HTMLImageElement>(".platform-icon")?.getAttribute("src")?.startsWith("data:image/"))).toBe(true);
      expect(container.querySelector(".brand .platform-icon")).not.toBeInTheDocument();
      expect(container.querySelector(".brand img")).toBeInTheDocument();
      expect(choices[1]).toHaveAttribute("aria-checked", "true");
      await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
      expect(container.querySelector('[role="menu"]')).not.toBeInTheDocument();
      await act(async () => banner?.click());
      const tourControl = document.createElement("button");
      tourControl.dataset.tutorialActive = "true";
      container.append(tourControl);
      await act(async () => tourControl.dispatchEvent(new Event("pointerdown", { bubbles: true })));
      expect(container.querySelector('[role="menu"]')).toBeInTheDocument();
      tourControl.remove();
      await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
      expect(container.querySelector('[role="menu"]')).not.toBeInTheDocument();
      expect(onSourceChange).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
