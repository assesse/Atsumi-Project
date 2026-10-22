import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { SoopWorkspace } from "./SoopWorkspace";
import { SOOP_FAVORITES_KEY } from "./soopFavorites";
import { auditLayouts, layoutBrowser } from "../../test/offlineLayoutBrowser";
import appCss from "../../styles.css?raw";
import workspaceCss from "./SoopWorkspace.css?raw";
import shellCss from "./SoopLiveShell.css?raw";

vi.mock("../platforms/PlatformLivePlayer", () => ({ PlatformLivePlayer: () => <div /> }));
vi.mock("../platforms/PlatformMediaLibrary", () => ({ PlatformMediaLibrary: () => null }));
vi.mock("../platforms/PlatformScreenshotLibrary", () => ({ PlatformScreenshotLibrary: () => null }));

// Test-only aliases let the existing CHZZK geometry auditor inspect identical
// controls. No CHZZK stylesheet, API, DOM id or alias is used in production.
function geometryAliases(element: HTMLElement) {
  const clone = element.cloneNode(true) as HTMLElement;
  for (const [selector, alias] of [
    [".soop-connection-channel-add", "connection-channel-add"], [".soop-clear", "connection-channel-clear"],
    [".soop-channel-row>.soop-star", "connection-channel-favorite"], [".soop-channel-actions", "live-channel-actions"],
    [".soop-favorites", "live-channel-list"], [".soop-favorite", "live-channel-row"], [".soop-favorite-copy", "live-channel-identity"],
    [".soop-mode", "mado-layout-options"], [".soop-connection-session-actions", "connection-session-actions"],
  ]) for (const item of clone.querySelectorAll(selector!)) item.classList.add(alias!);
  for (const item of clone.querySelectorAll(".soop-connection-actions")) item.classList.add("connection-actions", "is-unified");
  return clone.innerHTML;
}

describe.skipIf(!layoutBrowser)("SOOP CHZZK-clone offline geometry", () => {
  it.each([[960, 640], [1440, 900]])("keeps overlay controls centered and four-row favorites at %ix%i", async (width, height) => {
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container), frames: { name: string; html: string }[] = [];
    const api = { inspect: async (id: string) => ({ channelId: id, name: `채널 ${id}`, title: "방송 제목", image: null, status: "live" as const, viewers: 1 }) };
    const autoRecordApi = { snapshot: async () => ({ revision: 1, channels: [], ready: true, error: null }), set: vi.fn() };
    try {
      for (const panelWidth of [420, 960]) for (const count of [0, 2, 4, 8, 10]) {
        localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify(Array.from({ length: count }, (_, index) => ({ channelId: `channel${index}`, name: `채널 ${index}` }))));
        await act(async () => root.render(<SoopWorkspace key={`${panelWidth}-${count}`} active runtime="tauri" privacyMode={false} view="live" api={api} autoRecordApi={autoRecordApi} />));
        await act(async () => container.querySelector<HTMLButtonElement>('.soop-bottom-handle')!.click());
        for (let index = 0; index < 3; index++) await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="방송 추가"]')!.click());
        expect(container.querySelectorAll('.soop-channel-row')).toHaveLength(4);
        expect(container.querySelectorAll('.soop-favorite')).toHaveLength(count);
        frames.push({ name: `drawer-${panelWidth}-${count}`, html: `<main style="position:relative;width:${Math.min(width, panelWidth)}px;height:${height}px">${geometryAliases(container)}</main>` });
      }
      const results = await auditLayouts(frames, `${appCss}\n${workspaceCss}\n${shellCss}`, width, height, "soop-live-shell");
      expect(results).toHaveLength(frames.length);
      expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
    } finally { await act(async () => root.unmount()); container.remove(); localStorage.clear(); }
  }, 60000);
});
