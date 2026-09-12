// Render actual Mado React markup/CSS in a network-blocked local Edge profile.
// No CHZZK page, user browser profile, account, native app or recording is used.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { emptyMultiview, type MultiviewApi, type MultiviewSnapshot } from "../../api/multiview";
import { MadoWorkspace } from "./MadoWorkspace";
import appCss from "../../styles.css?raw";
import workspaceCss from "./StreamingWorkspace.css?raw";
import madoCss from "./MadoWorkspace.css?raw";

const fsName = "node:fs", pathName = "node:path", osName = "node:os", childName = "node:child_process", urlName = "node:url";
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, contents: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const { tmpdir } = await import(osName) as { tmpdir(): string };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: "utf8"; windowsHide: boolean }, callback: (error: Error | null, stdout: string) => void): void };
const edge = ["C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe"].find((candidate) => fs.existsSync(candidate));
type Frame = { mode: string; privacy: boolean; html: string; videos: number; chats: number };
type Box = { x: number; y: number; width: number; height: number; right: number; bottom: number };
type Measurement = { mode: string; privacy: boolean; videos: number; chats: number; surface: Box; scrollWidth: number; clientWidth: number; scrollHeight: number; overflow: string; slots: { box: Box; parent: Box; kind: string }[]; lastAtEnd: Box };

async function snapshots(): Promise<Frame[]> {
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container), frames: Frame[] = [];
  try {
    for (const mode of ["paired", "chats", "chats-vertical", "one"]) for (const privacy of [false, true]) {
      localStorage.setItem("atsumi.mado.layout.v1", JSON.stringify({ version: 1, direction: mode === "chats-vertical" ? "vertical" : "horizontal", horizontal: 60, vertical: 50 }));
      const channels = ["a", "b", "c", "d"].slice(0, mode === "one" ? 1 : 4).map((letter) => letter.repeat(32));
      const state: MultiviewSnapshot = { active: true, epoch: 50, audioOwner: null, panes: channels.flatMap((channelId, index) => [
        ...(!mode.startsWith("chats") || index === 1 ? [{ paneId: `layout-${channelId}-video`, channelId, kind: "video" as const, status: "ready" }] : []),
        { paneId: `layout-${channelId}-chat`, channelId, kind: "chat" as const, status: "ready" },
      ]) };
      const snapshot = async () => ({ ok: true as const, data: state });
      const api: MultiviewApi = { runtime: "tauri", snapshot, configure: snapshot, setAudio: snapshot, setPaneAudio: snapshot, requestControl: snapshot, confirmControl: snapshot, ackUiAction: snapshot, close: async () => ({ ok: true, data: emptyMultiview() }), setViewport: async () => ({ ok: true, data: undefined }) };
      await act(async () => root.render(<MadoWorkspace key={`${mode}-${privacy}`} api={api} runtime="tauri" privacy={privacy} onLeave={() => {}} />));
      frames.push({ mode, privacy, html: container.innerHTML, videos: mode === "paired" ? 4 : 1, chats: mode === "one" ? 1 : 4 });
    }
  } finally { await act(async () => root.unmount()); container.remove(); }
  return frames;
}
async function measure(frames: Frame[], width: number, height: number): Promise<Measurement[]> {
  const directory = fs.mkdtempSync(path.join(tmpdir(), "atsumi-mado-layout-"));
  try {
    const fixture = path.join(directory, "fixture.html");
    fs.writeFileSync(fixture, `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'"><style>${appCss}\n${workspaceCss}\n${madoCss}\nhtml,body,#fixture-root{margin:0;width:100%;height:100%;overflow:hidden}.app-shell{height:100%;grid-template-columns:200px minmax(0,1fr)}</style></head><body><div id="fixture-root"></div><script>
      const frames=${JSON.stringify(frames).replaceAll("</script", "<\\/script")},results=[],root=document.getElementById('fixture-root');
      const box=e=>{const r=e.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom}};
      for(const frame of frames){
        root.innerHTML='<div class="app-shell streaming-shell"><aside></aside><main class="streaming-workspace is-official-view"><header class="streaming-heading"><h1>라이브 시청·녹화</h1></header>'+frame.html+'</main></div>';
        const surface=root.querySelector('.mado-surfaces'), slots=[...surface.querySelectorAll('.mado-native-slot')];
        const result={mode:frame.mode,privacy:frame.privacy,videos:surface.querySelectorAll('.is-video').length,chats:surface.querySelectorAll('.is-chat').length,surface:box(surface),scrollWidth:surface.scrollWidth,clientWidth:surface.clientWidth,scrollHeight:surface.scrollHeight,overflow:getComputedStyle(surface).overflowY,slots:slots.map(element=>({box:box(element),parent:box(element.parentElement),kind:element.classList.contains('is-video')?'video':'chat'}))};
        for(let parent=slots[slots.length-1].parentElement;parent&&surface.contains(parent);parent=parent.parentElement)parent.scrollTop=parent.scrollHeight;
        result.lastAtEnd=box(slots[slots.length-1]);results.push(result);
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
  it.each([[960, 640], [1440, 900], [1920, 1080]])("keeps all video/chat panes reachable and unclipped at %ix%i", async (width, height) => {
    const frames = await snapshots(), results = await measure(frames, width, height);
    for (const [index, result] of results.entries()) {
      const frame = frames[index]!;
      expect(result.videos).toBe(frame.videos); expect(result.chats).toBe(frame.chats);
      expect(result.scrollWidth, `${result.mode} horizontal overflow`).toBeLessThanOrEqual(result.clientWidth + 1);
      expect(result.overflow).toBe("auto");
      for (const slot of result.slots) {
        expect(slot.box.width).toBeGreaterThanOrEqual(slot.kind === "video" ? 150 : 210);
        expect(slot.box.height).toBeGreaterThan(200);
        expect(slot.box.x).toBeGreaterThanOrEqual(slot.parent.x - 1);
        expect(slot.box.right).toBeLessThanOrEqual(slot.parent.right + 1);
        expect(slot.box.bottom).toBeLessThanOrEqual(slot.parent.bottom + 1);
      }
      expect(result.lastAtEnd.bottom, `${result.mode} final chat input reachability`).toBeLessThanOrEqual(result.surface.bottom + 1);
      if (result.privacy) {
        const visible = results.find((other) => other.mode === result.mode && !other.privacy)!;
        expect(result.slots.map((slot) => slot.box)).toEqual(visible.slots.map((slot) => slot.box));
      }
    }
  }, 30000);
});
