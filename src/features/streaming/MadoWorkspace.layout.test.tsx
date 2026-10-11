// Render actual Mado React markup/CSS in a network-blocked local Edge profile.
// No CHZZK page, user browser profile, account, native app or recording is used.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emptyMultiview, type MultiviewApi, type MultiviewSnapshot } from "../../api/multiview";
import { MadoWorkspace } from "./MadoWorkspace";
import appCss from "../../styles.css?raw";
import workspaceCss from "./StreamingWorkspace.css?raw";
import madoCss from "./MadoWorkspace.css?raw";
import edgeCss from "./LiveEdgePanel.css?raw";
import officialCss from "./OfficialBrowserPanel.css?raw";
import { auditLayouts } from "../../test/offlineLayoutBrowser";

const fsName = "node:fs", pathName = "node:path", osName = "node:os", childName = "node:child_process", urlName = "node:url";
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, contents: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const { tmpdir } = await import(osName) as { tmpdir(): string };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: "utf8"; windowsHide: boolean }, callback: (error: Error | null, stdout: string) => void): void };
const processName = "node:process";
const { env } = await import(processName) as { env: Record<string, string | undefined> };
const edge = [env.ATSUMI_TEST_BROWSER ?? "", "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe"].find((candidate) => fs.existsSync(candidate));
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); });
afterEach(() => { vi.unstubAllGlobals(); });
type Frame = { mode: string; privacy: boolean; html: string; videos: number; chats: number; panel?: boolean };
type Box = { x: number; y: number; width: number; height: number; right: number; bottom: number };
type Measurement = { mode: string; privacy: boolean; videos: number; chats: number; surface: Box; viewportHeight: number; scrollWidth: number; clientWidth: number; scrollHeight: number; overflow: string; slots: { box: Box; parent: Box; kind: string; channel: string | null }[]; lastAtEnd: Box; panel?: { box: Box; sidebarInteractive: boolean; surfaceInteractive: boolean } };

async function snapshots(): Promise<Frame[]> {
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container), frames: Frame[] = [];
  try {
    for (const mode of ["paired", "paired-two", "grouped", "grouped-vertical", "chats", "chats-vertical", "one", "immersive"]) for (const privacy of [false, true]) {
      localStorage.setItem("atsumi.live.arrangement.v1", mode.startsWith("grouped") ? "grouped" : "channels");
      localStorage.setItem("atsumi.mado.layout.v1", JSON.stringify({ version: 1, direction: mode.endsWith("vertical") ? "vertical" : "horizontal", horizontal: 60, vertical: 50 }));
      const channels = ["a", "b", "c", "d"].slice(0, mode === "one" || mode === "immersive" ? 1 : mode === "paired-two" ? 2 : 4).map((letter) => letter.repeat(32));
      const state: MultiviewSnapshot = { active: true, epoch: 50, audioOwner: null, panes: channels.flatMap((channelId, index) => [
        ...(!mode.startsWith("chats") || index === 1 ? [{ paneId: `layout-${channelId}-video`, channelId, kind: "video" as const, status: "ready" }] : []),
        { paneId: `layout-${channelId}-chat`, channelId, kind: "chat" as const, status: "ready" },
      ]) };
      const snapshot = async () => ({ ok: true as const, data: state });
      const api: MultiviewApi = { runtime: "tauri", snapshot, configure: snapshot, setAudio: snapshot, setPaneAudio: snapshot, requestControl: snapshot, confirmControl: snapshot, ackUiAction: snapshot, close: async () => ({ ok: true, data: emptyMultiview() }), setViewport: async () => ({ ok: true, data: undefined }) };
      await act(async () => root.render(<MadoWorkspace key={`${mode}-${privacy}`} api={api} runtime="tauri" privacy={privacy} unifiedLive={mode === "immersive"} onLeave={() => {}} />));
      frames.push({ mode, privacy, html: container.innerHTML, videos: state.panes.filter(pane => pane.kind === "video").length, chats: channels.length });
      if (mode === "one" && !privacy) {
        await act(async () => [...container.querySelectorAll("button")].find(button => button.textContent === "연결 설정")!.click());
        frames.push({ mode, privacy, html: container.innerHTML, videos: 1, chats: 1, panel: true });
      }
      if (mode === "immersive") expect(container.querySelector('header,.edge-top')).toBeNull();
    }
  } finally { await act(async () => root.unmount()); container.remove(); }
  return frames;
}
async function measure(frames: Frame[], width: number, height: number): Promise<Measurement[]> {
  const directory = fs.mkdtempSync(path.join(tmpdir(), "atsumi-mado-layout-"));
  try {
    const fixture = path.join(directory, "fixture.html");
    fs.writeFileSync(fixture, `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'"><style>${appCss}\n${workspaceCss}\n${officialCss}\n${madoCss}\n${edgeCss}\nhtml,body,#fixture-root{margin:0;width:100%;height:100%;overflow:hidden}.app-shell{height:100%;grid-template-columns:200px minmax(0,1fr)}</style></head><body><div id="fixture-root"></div><script>
      const frames=${JSON.stringify(frames).replaceAll("</script", "<\\/script")},results=[],root=document.getElementById('fixture-root');
      const box=e=>{const r=e.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom}};
      for(const frame of frames){
        root.innerHTML=frame.mode==='immersive'?'<div class="app-shell streaming-shell is-live-immersive"><main class="streaming-workspace is-official-view">'+frame.html+'</main></div>':'<div class="app-shell streaming-shell"><aside><button id="layout-nav">녹화 목록</button></aside><main class="streaming-workspace is-official-view"><header class="streaming-heading"><h1>라이브 시청·녹화</h1></header>'+frame.html+'</main></div>';
        const surface=root.querySelector('.mado-surfaces'), slots=[...surface.querySelectorAll('.mado-native-slot')];
        const result={mode:frame.mode,privacy:frame.privacy,videos:surface.querySelectorAll('.is-video').length,chats:surface.querySelectorAll('.is-chat').length,surface:box(surface),viewportHeight:innerHeight,scrollWidth:surface.scrollWidth,clientWidth:surface.clientWidth,scrollHeight:surface.scrollHeight,overflow:getComputedStyle(surface).overflowY,slots:slots.map(element=>({box:box(element),parent:box(element.parentElement),kind:element.classList.contains('is-video')?'video':'chat',channel:element.closest('.live-channel-pair')?.getAttribute('data-channel-index')??null}))};
        for(let parent=slots[slots.length-1].parentElement;parent&&surface.contains(parent);parent=parent.parentElement)parent.scrollTop=parent.scrollHeight;
        result.lastAtEnd=box(slots.filter(element=>element.classList.contains('is-chat')&&element.getBoundingClientRect().width>0).at(-1));results.push(result);
        if(frame.panel){
          const panel=root.querySelector('.mado-connection-panel>.connection-setup'),nav=root.querySelector('#layout-nav'),panelBox=box(panel),navBox=box(nav);
          result.panel={box:panelBox,sidebarInteractive:nav.contains(document.elementFromPoint(navBox.x+navBox.width/2,navBox.y+navBox.height/2)),surfaceInteractive:panel.contains(document.elementFromPoint(panelBox.x+panelBox.width/2,panelBox.y+20))};
        }
      }
      const output=document.createElement('output');output.id='mado-layout-result';output.setAttribute('data-result',encodeURIComponent(JSON.stringify(results)));document.body.append(output);
    </script></body></html>`);
    const stdout = await new Promise<string>((resolve, reject) => execFile(edge!, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--host-resolver-rules=MAP * ~NOTFOUND", `--user-data-dir=${path.join(directory, "profile")}`, `--window-size=${width},${height}`, "--force-device-scale-factor=1", "--virtual-time-budget=500", "--dump-dom", pathToFileURL(fixture).href], { timeout: 20000, maxBuffer: 4 * 1024 ** 2, encoding: "utf8", windowsHide: true }, (error, output) => error ? reject(error) : resolve(output)));
    const serialized = /<output id="mado-layout-result" data-result="([^"]+)"/.exec(stdout)?.[1];
    if (!serialized) throw new Error("Isolated Mado layout returned no measurements");
    return JSON.parse(decodeURIComponent(serialized)) as Measurement[];
  } finally {
    const relative = path.relative(fs.realpathSync(tmpdir()), fs.realpathSync(directory));
    if (!relative.startsWith("atsumi-mado-layout-") || relative.includes("..") || path.isAbsolute(relative)) throw new Error("Unexpected Mado layout fixture path");
    fs.rmSync(directory, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
}

describe.skipIf(!edge)("Mado real layout", () => {
  it.each([[960, 640], [1440, 900], [1920, 1080], [1920, 1000], [1536, 780], [1280, 640]])("keeps all video/chat panes reachable and unclipped at %ix%i", async (width, height) => {
    const frames = await snapshots(), results = await measure(frames, width, height);
    const controls = await auditLayouts(frames.map((frame, index) => ({ name: `mado-${frame.mode}-${index}`, html: `<div id="root"><div class="app-shell streaming-shell${frame.mode === 'immersive' ? ' is-live-immersive' : ''}">${frame.mode === 'immersive' ? '' : '<aside></aside>'}<main class="streaming-workspace is-official-view">${frame.html}</main></div></div>` })), [appCss, workspaceCss, officialCss, madoCss, edgeCss].join("\n"), width, height, "mado");
    expect(controls.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
    for (const [index, result] of results.entries()) {
      const frame = frames[index]!;
      expect(result.videos).toBe(frame.videos); expect(result.chats).toBe(frame.chats);
      expect(result.scrollWidth, `${result.mode} horizontal overflow`).toBeLessThanOrEqual(result.clientWidth + 1);
      expect(result.overflow).toBe("hidden");
      expect(result.scrollHeight, `${result.mode} fits the available height`).toBeLessThanOrEqual(result.surface.height + 1);
      expect(result.surface.bottom).toBeLessThanOrEqual(height);
      if (frame.mode === 'immersive') { expect(result.surface.x).toBeLessThanOrEqual(9); expect(result.surface.y).toBeLessThanOrEqual(9); expect(result.surface.height).toBeGreaterThanOrEqual(result.viewportHeight - 10); }
      const displayed = result.slots.filter(slot => slot.box.width > 0 && slot.box.height > 0);
      expect(displayed.filter(slot => slot.kind === "video")).toHaveLength(frame.videos);
      if (frame.mode.startsWith('paired')) {
        const compact = result.surface.width <= 799 || result.viewportHeight <= 700;
        expect(displayed.filter(slot => slot.kind === 'chat')).toHaveLength(compact ? 1 : frame.chats);
        if (!compact) for (const video of displayed.filter(slot => slot.kind === 'video')) {
          const chat = displayed.find(slot => slot.channel === video.channel && slot.kind === 'chat')!;
          expect(chat.box.x - video.box.right, `${frame.mode} same-channel adjacency`).toBeCloseTo(10, 0);
          expect(chat.box.y).toBeCloseTo(video.box.y, 0);
          expect(chat.box.height).toBeCloseTo(video.box.height, 0);
        }
      } else if (width <= 1280 || height <= 820) expect(displayed.filter(slot => slot.kind === "chat")).toHaveLength(1);
      for (const slot of result.slots) {
        if (!slot.box.width || !slot.box.height) continue;
        expect(slot.box.width).toBeGreaterThanOrEqual(slot.kind === "video" ? 159 : 219);
        expect(slot.box.height).toBeGreaterThanOrEqual(slot.kind === "video" ? 89 : 199);
        expect(slot.box.x).toBeGreaterThanOrEqual(slot.parent.x - 1);
        expect(slot.box.right).toBeLessThanOrEqual(slot.parent.right + 1);
        expect(slot.box.bottom).toBeLessThanOrEqual(slot.parent.bottom + 1);
        expect(slot.box.y, `${result.mode} ${slot.kind} has no outside header gap`).toBeCloseTo(slot.parent.y, 0);
      }
      expect(result.lastAtEnd.bottom, `${result.mode} final chat input reachability`).toBeLessThanOrEqual(result.surface.bottom + 1);
      if (frame.panel) {
        expect(result.panel?.sidebarInteractive, "the channel panel must not intercept sidebar clicks").toBe(true);
        expect(result.panel?.surfaceInteractive, "the channel form must still receive input").toBe(true);
        expect(result.panel!.box.y).toBeGreaterThanOrEqual(0);
        expect(result.panel!.box.bottom).toBeLessThanOrEqual(height);
      }
      if (result.privacy) {
        const visible = results.find((other) => other.mode === result.mode && !other.privacy)!;
        expect(result.slots.map((slot) => slot.box)).toEqual(visible.slots.map((slot) => slot.box));
      }
    }
  }, 90000);
});
