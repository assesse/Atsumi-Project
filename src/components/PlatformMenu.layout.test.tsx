import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { SideRail } from "./SideRail";
import { auditLayouts, layoutBrowser, type LayoutScene } from "../test/offlineLayoutBrowser";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";

const noop = () => {};

async function menuScene(collapsed: boolean): Promise<LayoutScene> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<div id="root"><div className={`app-shell${collapsed ? " sidebar-collapsed" : ""}`}>
      <SideRail source="youtube" view="live" collapsed={collapsed} autoFindCount={0} attentionCount={0}
        privacyMode onPrivacyModeToggle={noop} onSettings={noop} onNavigate={noop} onSourceChange={noop} onToggle={noop} />
      <main className="workspace"><h1>YouTube</h1></main>
    </div></div>));
    await act(async () => container.querySelector<HTMLButtonElement>(".brand")!.click());
    expect(container.querySelectorAll('[role="menuitemradio"] .platform-icon')).toHaveLength(8);
    return { name: collapsed ? "platform-icons-collapsed" : "platform-icons-expanded", html: container.innerHTML };
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
}

describe.skipIf(!layoutBrowser)("eight-platform banner", () => {
  it.each([[960, 640], [1280, 800]])("keeps the icon menu inside the rail at %ix%i", async (width, height) => {
    const scenes = [await menuScene(false), await menuScene(true)];
    const results = await auditLayouts(scenes, appCss + inkCss, width, height, `platform-icons-${width}`);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 60000);
});
