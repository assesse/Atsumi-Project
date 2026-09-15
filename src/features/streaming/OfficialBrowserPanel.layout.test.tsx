// Real layout test of production React markup + CSS in an isolated local Edge
// profile. It never opens Atsumi, CHZZK, an account, media or the user database.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { emptyOfficialBrowserSnapshot, type OfficialBrowserApi, type OfficialBrowserSnapshot } from "../../api/officialBrowser";
import { OfficialBrowserPanel } from "./OfficialBrowserPanel";
import appCss from "../../styles.css?raw";
import panelCss from "./OfficialBrowserPanel.css?raw";
import workspaceCss from "./StreamingWorkspace.css?raw";

const fsName = "node:fs", pathName = "node:path", osName = "node:os", childName = "node:child_process", urlName = "node:url";
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, contents: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const { tmpdir } = await import(osName) as { tmpdir(): string };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: "utf8"; windowsHide: boolean }, callback: (error: Error | null, stdout: string) => void): void };
const edge = ["C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe"].find((candidate) => fs.existsSync(candidate));
type Frame = { name: string; html: string };
type Box = { x: number; y: number; width: number; height: number; bottom: number };
type Measurement = { name: string; stage: Box; action: Box | null; viewportHeight: number; overflow: string; footer: boolean; chromeOutsideStage: boolean; setup: Box | null; setupPosition: string | null };

async function snapshots(): Promise<Frame[]> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const frames: Frame[] = [];
  const record = { id: "layout-recording", channelId: "a".repeat(32), title: "긴 방송 제목 ".repeat(50), startedAt: 1, updatedAt: 2, status: "recording" as const, mimeType: "video/webm", outputDir: "", segmentCount: 10000, bytesWritten: 1024 ** 4, durationSeconds: 360000, lastError: null, segments: [] };
  try {
    for (const name of ["ready", "recording", "stopping", "error", "screenshot", "settings", "setup"]) {
      const state: OfficialBrowserSnapshot = { ...emptyOfficialBrowserSnapshot("tauri"), windowOpen: name !== "setup", ready: name !== "setup", status: name === "setup" ? "closed" : ["screenshot", "error", "settings"].includes(name) ? "ready" : name, channelId: "a".repeat(32), videoWidth: 1920, videoHeight: 1080, extensionStatus: "확장 로드 완료 · 공식 페이지 감지 확인 중", recordings: name === "recording" || name === "stopping" ? [record] : [], recordingId: name === "recording" || name === "stopping" ? record.id : null, chatStatus: name === "recording" ? "queue_overflow" : "receiving", chatCount: 99999999, error: name === "error" ? "시청 상태를 확인해 주세요. ".repeat(15) : null, lastScreenshot: name === "screenshot" ? { id: "shot", channelId: "a".repeat(32), fileName: "layout-only.png", createdAt: Date.now() } : null, pendingUiAction: name === "settings" ? { id: "layout-settings", action: "open_settings", expiresAt: Date.now() + 8000 } : null };
      const snapshot = async () => ({ ok: true as const, data: state });
      const nothing = async () => ({ ok: true as const, data: undefined });
      const api: OfficialBrowserApi = { runtime: "tauri", snapshot, open: snapshot, start: snapshot, stop: snapshot, login: snapshot, logout: snapshot, connectExtension: snapshot, confirmControl: snapshot, requestControl: snapshot, ackUiAction: snapshot, setViewport: nothing, openInstaller: nothing, openFolder: nothing, openSegment: nothing, openMerged: nothing, retryMerge: snapshot, deleteRecordings: async () => ({ ok: true, data: { deletedIds: [], failures: [] } }) };
      await act(async () => root.render(<OfficialBrowserPanel key={name} api={api} runtime="tauri" active view="live" />));
      frames.push({ name, html: container.innerHTML });
    }
  } finally { await act(async () => root.unmount()); container.remove(); }
  return frames;
}

async function measure(frames: Frame[], width: number, height: number): Promise<Measurement[]> {
  const directory = fs.mkdtempSync(path.join(tmpdir(), "atsumi-panel-layout-"));
  try {
    const fixture = path.join(directory, "fixture.html");
    fs.writeFileSync(fixture, `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'"><style>${appCss}\n${panelCss}\n${workspaceCss}\nhtml,body,#fixture-root{margin:0;width:100%;height:100%;overflow:hidden}.app-shell{height:100%;grid-template-columns:200px minmax(0,1fr)}</style></head><body><div id="fixture-root"></div><script>
      const frames=${JSON.stringify(frames).replaceAll("</script", "<\\/script")},results=[],root=document.getElementById('fixture-root');
      const box=e=>{if(!e)return null;const r=e.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height,bottom:r.bottom}};
      for(const frame of frames){
        root.innerHTML='<div class="app-shell streaming-shell"><aside></aside><main class="streaming-workspace is-official-view">'+frame.html+'</main></div>';
        const stage=root.querySelector('.official-browser-stage'),setup=root.querySelector('.official-browser-setup');
        const chrome=[...root.querySelectorAll('.official-browser-heading,.official-browser-quickbar,.official-browser-actions,.official-browser-consent,.official-browser-resolution,.official-browser-settings')];
        results.push({name:frame.name,stage:box(stage),action:box(root.querySelector('.official-browser-actions > button')),viewportHeight:innerHeight,overflow:getComputedStyle(root.querySelector('main')).overflowY,footer:!!root.querySelector('.official-browser-live-stats,.official-browser-library-hint'),chromeOutsideStage:chrome.some(element=>!stage.contains(element)),setup:box(setup),setupPosition:setup?getComputedStyle(setup).position:null});
      }
      const output=document.createElement('output');output.id='panel-layout-result';output.setAttribute('data-result',encodeURIComponent(JSON.stringify(results)));document.body.append(output);
    </script></body></html>`);
    const stdout = await new Promise<string>((resolve, reject) => execFile(edge!, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--host-resolver-rules=MAP * ~NOTFOUND", `--user-data-dir=${path.join(directory, "profile")}`, `--window-size=${width},${height}`, "--force-device-scale-factor=1", "--virtual-time-budget=500", "--dump-dom", pathToFileURL(fixture).href], { timeout: 20000, maxBuffer: 4 * 1024 ** 2, encoding: "utf8", windowsHide: true }, (error, output) => error ? reject(error) : resolve(output)));
    const serialized = /<output id="panel-layout-result" data-result="([^"]+)"/.exec(stdout)?.[1];
    if (!serialized) throw new Error("The isolated layout fixture returned no measurements");
    return JSON.parse(decodeURIComponent(serialized)) as Measurement[];
  } finally {
    const relative = path.relative(fs.realpathSync(tmpdir()), fs.realpathSync(directory));
    if (!relative.startsWith("atsumi-panel-layout-") || relative.includes("..") || path.isAbsolute(relative)) throw new Error("Unexpected layout fixture path");
    fs.rmSync(directory, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
}

describe.skipIf(!edge)("official panel stable real layout", () => {
  it.each([[960, 640], [1440, 900]])("keeps video/chat bounds stable at %ix%i across recording and error states", async (width, height) => {
    const results = await measure(await snapshots(), width, height);
    {
      const states = results;
      const first = states[0]!;
      for (const state of states) {
        expect(state.stage, `${state.name} stage`).toEqual(first.stage);
        expect(state.chromeOutsideStage, `${state.name} app chrome outside video`).toBe(false);
        if (state.name === "settings" || state.name === "setup") {
          expect(state.setup, `${state.name} setup card`).not.toBeNull();
          expect(state.setupPosition).toBe("absolute");
          expect(state.setup!.x).toBeGreaterThanOrEqual(state.stage.x);
          expect(state.setup!.y).toBeGreaterThanOrEqual(state.stage.y);
          expect(state.setup!.x + state.setup!.width).toBeLessThanOrEqual(state.stage.x + state.stage.width);
          expect(state.setup!.bottom).toBeLessThanOrEqual(state.stage.bottom);
        } else {
          expect(state.action, `${state.name} app recording controls`).toBeNull();
          expect(state.setup).toBeNull();
        }
        expect(state.footer).toBe(false);
        expect(state.overflow).toBe("auto");
        expect(state.stage.width).toBeGreaterThan(500);
        expect(state.stage.height).toBeGreaterThanOrEqual(360);
      }
    }
  }, 30000);
});
