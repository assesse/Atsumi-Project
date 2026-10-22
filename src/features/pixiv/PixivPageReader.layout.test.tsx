import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { PixivPageReader } from "./PixivPageReader";
import { createOriginalWindow } from "./originalWindow";
import type { PixivDetail } from "./model";
import baseCss from "../../styles.css?raw";
import inkCss from "../../atsumi-ink.css?raw";
import workspaceCss from "./PixivWorkspace.css?raw";
import readerCss from "./PixivPageReader.css?raw";

// Local fixtures only: no app attachment, account profile or remote resources.
const fsName = "node:fs", pathName = "node:path", osName = "node:os", childName = "node:child_process", processName = "node:process", urlName = "node:url";
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, content: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const { tmpdir } = await import(osName) as { tmpdir(): string };
const { env } = await import(processName) as { env: Record<string, string | undefined> };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: "utf8"; windowsHide: boolean }, callback: (error: Error | null, stdout: string, stderr: string) => void): void };
const browser = [env.ATSUMI_TEST_BROWSER ?? "", "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", "C:/Program Files/Microsoft/Edge/Application/msedge.exe"].find(candidate => fs.existsSync(candidate));
type Box = { x: number; y: number; width: number; height: number };
type Measurement = { name: string; reader: Box; stage: Box; count: Box; left: Box; right: Box; pin: Box; favorites: Box; font: { size: string; weight: string }; controls: Box[]; header: Box; title: Box; toolbar: Box; navigation: Box; overflow: number };
const title = "아주 긴 작품 제목과 다양한 너비의 원본을 읽는 로컬 레이아웃 검증";
const icon = <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 4h12v16H6z" /></svg>;
async function scene(width: number, height: number, spread: boolean) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const pool = createOriginalWindow({ localMedia: async () => "", pageMedia: async () => "" });
  const detail: PixivDetail = { work: { id: "123", title, artist: "Fixture", artistId: "1", width: 700, height: 1000, pageCount: 10000, tags: [], createdAt: "", bookmarkCount: 0, type: "manga" }, pages: Array.from({ length: 10000 }, (_, index) => ({ index, width: index % 2 ? 1600 : 700, height: 1000, previewUrl: "", originalUrl: "" })) };
  pool.get = () => ({ state: "ready", url: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='700' height='1000'%3E%3Crect width='100%25' height='100%25' fill='%23657'/%3E%3C/svg%3E", bytes: 1 });
  try {
    await act(async () => root.render(<PixivPageReader detail={detail} initialPage={9998} pool={pool} privacyMode={false} initialPinned isFavorite={() => false} onFavorite={() => {}} onClose={() => {}}
      toolbar={<><div className="pixiv-detail-actions"><button>{icon}앨범</button><button className="pixiv-save-button">{icon}다운로드</button><button>웹에서 열기</button></div><div className="pixiv-reader-work-info"><button className="pixiv-artist">긴 작가 이름과 이름</button><button className="pixiv-reader-tags-toggle">태그</button></div></>}
      navigation={<nav className="pixiv-search-tabs pixiv-detail-tabs" aria-label="열린 작품">{["첫 번째 매우 긴 작품 이름", "두 번째 작품", "세 번째 작품"].map((name, index) => <div key={name} className={index === 0 ? "is-active" : ""}><button>{name}</button><button aria-label={`${name} 닫기`}>{icon}</button></div>)}</nav>} />));
    if (spread) await act(async () => document.querySelector<HTMLButtonElement>('.pixiv-reader button[aria-label="두쪽 보기"]')!.click());
    const reader = document.querySelector<HTMLElement>(".pixiv-reader")!; reader.style.width = `${width}px`; reader.style.height = `${height}px`;
    return { name: `${width}x${height}-${spread ? "spread" : "single"}`, html: reader.parentElement!.outerHTML };
  } finally { await act(async () => root.unmount()); pool.clear(); host.remove(); }
}
async function measure(viewportWidth: number): Promise<Measurement[]> {
  const directory = fs.mkdtempSync(path.join(tmpdir(), "atsumi-pixiv-reader-chrome-"));
  const scenes = [];
  for (const width of [272, 320, 520, 1000]) for (const spread of [false, true]) scenes.push(await scene(width, width <= 320 ? 320 : 600, spread));
  const script = `(async()=>{const results=[];for(const scene of ${JSON.stringify(scenes)}){document.getElementById('fixture').innerHTML=scene.html;await Promise.all([...document.images].map(image=>image.decode().catch(()=>{})));const box=selector=>{const r=(typeof selector==='string'?document.querySelector(selector):selector).getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height}},r=document.querySelector('.pixiv-reader'),count=document.querySelector('.pixiv-reader-page-status');results.push({name:scene.name,reader:box(r),stage:box('.pixiv-original-pages'),count:box(count),left:box('.pixiv-reader-left-tools'),right:box('.pixiv-reader-tools'),pin:box('.pixiv-reader-pin'),favorites:box('.pixiv-reader-page-favorites'),font:{size:getComputedStyle(count).fontSize,weight:getComputedStyle(count).fontWeight},header:box('.pixiv-reader-header'),title:box('.pixiv-reader-title'),toolbar:box('.pixiv-reader-toolbar'),navigation:box('.pixiv-reader-navigation'),controls:[...r.querySelectorAll('button')].filter(el=>!el.matches('.pixiv-reader-arrow')&&!el.closest('.pixiv-detail-tabs')).map(box),overflow:r.scrollWidth-r.clientWidth});}document.getElementById('metrics').textContent=JSON.stringify(results);})().catch(error=>document.getElementById('metrics').textContent=JSON.stringify({error:String(error)}));`;
  try {
    const input = path.join(directory, "fixture.html");
    fs.writeFileSync(input, `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'"><style>${baseCss}\n${inkCss}\n${workspaceCss}\n${readerCss}\nbody{margin:0}*,*::before,*::after{animation:none!important;transition:none!important}</style><main id="fixture"></main><pre id="metrics"></pre><script>${script}</script>`);
    const output = await new Promise<string>((resolve, reject) => execFile(browser!, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--host-resolver-rules=MAP * ~NOTFOUND", `--user-data-dir=${path.join(directory, "profile")}`, `--window-size=${viewportWidth},900`, "--virtual-time-budget=3000", "--dump-dom", pathToFileURL(input).href], { timeout: 45000, maxBuffer: 4 * 1024 ** 2, encoding: "utf8", windowsHide: true }, (error, stdout, stderr) => error ? reject(new Error(stderr || error.message)) : resolve(stdout)));
    const match = /<pre id="metrics">([^<]+)<\/pre>/.exec(output)?.[1]; if (!match) throw new Error("No offline reader layout measurements");
    const parsed = JSON.parse(match) as Measurement[] | { error: string }; if (!Array.isArray(parsed)) throw new Error(parsed.error); return parsed;
  } finally {
    const relative = path.relative(fs.realpathSync(tmpdir()), fs.realpathSync(directory));
    if (!relative.startsWith("atsumi-pixiv-reader-chrome-") || relative.includes("..") || path.isAbsolute(relative)) throw new Error("Unexpected disposable fixture path");
    try { fs.rmSync(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); } catch (error) { if (!["EPERM", "EBUSY", "ENOTEMPTY"].includes((error as { code?: string }).code ?? "")) throw error; }
  }
}
const overlaps = (a: Box, b: Box) => a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
describe.skipIf(!browser)("Pixiv reader chrome: offline production CSS geometry", () => {
  it.each([640, 1280])("keeps pin/favorites/count and work controls separate in a %ipx browser", async viewportWidth => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); const results = await measure(viewportWidth); expect(results).toHaveLength(8);
    for (const r of results) {
      expect(r.overflow, r.name).toBeLessThanOrEqual(1); expect(Math.abs(r.stage.width - (r.reader.width - 2)), r.name).toBeLessThan(1); expect(Math.abs(r.stage.height - (r.reader.height - 2)), r.name).toBeLessThan(1);
      expect(Math.abs(r.count.x + r.count.width / 2 - r.reader.x - r.reader.width / 2), r.name).toBeLessThan(1);
      expect(r.font, r.name).toEqual({ size: "18px", weight: "600" });
      expect(overlaps(r.count, r.left) || overlaps(r.count, r.right) || overlaps(r.left, r.right), r.name).toBe(false);
      expect(r.favorites.x, r.name).toBeGreaterThanOrEqual(r.pin.x + r.pin.width); expect(Math.abs(r.favorites.y + r.favorites.height / 2 - r.pin.y - r.pin.height / 2), r.name).toBeLessThan(1);
      expect(r.toolbar.y, r.name).toBeGreaterThanOrEqual(r.title.y + r.title.height); expect(r.navigation.y, r.name).toBeGreaterThanOrEqual(r.toolbar.y + r.toolbar.height);
      expect(r.navigation.x, r.name).toBeGreaterThanOrEqual(r.reader.x); expect(r.navigation.x + r.navigation.width, r.name).toBeLessThanOrEqual(r.reader.x + r.reader.width);
      expect(r.header.y + r.header.height, r.name).toBeLessThanOrEqual(r.count.y);
      for (const button of r.controls) { expect(button.x, r.name).toBeGreaterThanOrEqual(r.reader.x); expect(button.x + button.width, r.name).toBeLessThanOrEqual(r.reader.x + r.reader.width); expect(button.y, r.name).toBeGreaterThanOrEqual(r.reader.y); expect(button.y + button.height, r.name).toBeLessThanOrEqual(r.reader.y + r.reader.height); }
    }
  }, 60000);
});
