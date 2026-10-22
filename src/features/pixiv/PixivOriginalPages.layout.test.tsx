import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PixivOriginalPages } from "./PixivOriginalPages";
import { createOriginalWindow } from "./originalWindow";
import type { PixivDetail } from "./model";
import baseCss from "../../styles.css?raw";
import inkCss from "../../atsumi-ink.css?raw";
import platformCss from "../platforms/PlatformWorkspace.css?raw";
import workspaceCss from "./PixivWorkspace.css?raw";
import readerCss from "./PixivPageReader.css?raw";

type Dimension = { width: number; height: number };
const portrait = { width: 700, height: 1000 }, landscape = { width: 1600, height: 900 };
const thin = { width: 100, height: 1000 };
const image = ({ width, height }: Dimension) => `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${width || 2}" height="${height || 3}"><rect width="100%" height="100%" fill="#657"/></svg>`)}`;
function markup(dimensions: Dimension[], pages: (number | null)[]) {
  const detail: PixivDetail = {
    work: { id: "123", title: "Local geometry fixture", artist: "Fixture", artistId: "1", width: 700, height: 1000, pageCount: dimensions.length, tags: [], createdAt: "", bookmarkCount: 0, type: "manga" },
    pages: dimensions.map((dimension, index) => ({ ...dimension, index, previewUrl: "", originalUrl: "" })),
  };
  const pool = createOriginalWindow({ localMedia: async () => "", pageMedia: async () => "" });
  pool.get = key => ({ state: "ready", url: image(dimensions[Number(key.split(":").at(-1))]!), bytes: 1 });
  return renderToStaticMarkup(<PixivOriginalPages detail={detail} pages={pages} pool={pool} privacyMode={false} />);
}
const cases = [
  { name: "portrait", dimensions: [portrait], pages: [0], ratios: [1] },
  { name: "landscape", dimensions: [landscape], pages: [0], ratios: [1] },
  { name: "portrait-pair", dimensions: [portrait, portrait], pages: [0, 1], ratios: [1, 1] },
  { name: "mixed", dimensions: [portrait, landscape], pages: [0, 1], ratios: [1, (1600 / 900) / .7] },
  { name: "mixed-reversed", dimensions: [portrait, landscape], pages: [1, 0], ratios: [(1600 / 900) / .7, 1] },
  { name: "thin-pair", dimensions: [thin, thin], pages: [0, 1], ratios: [1, 1] },
  { name: "last-blank", dimensions: [portrait], pages: [0, null], ratios: [1, 1] },
  { name: "first-blank", dimensions: [portrait], pages: [null, 0], ratios: [1, 1] },
  { name: "fallback", dimensions: [{ width: 0, height: 0 }], pages: [0], ratios: [1] },
] satisfies { name: string; dimensions: Dimension[]; pages: (number | null)[]; ratios: number[] }[];

describe("Pixiv original slot weights", () => {
  it.each(cases)("fills available space without changing relative widths: $name", scene => {
    const host = document.createElement("div"); host.innerHTML = markup(scene.dimensions, scene.pages);
    const slots = [...host.querySelectorAll<HTMLElement>(".pixiv-original-slot")];
    expect(slots).toHaveLength(scene.pages.length);
    slots.forEach((slot, index) => {
      expect(Number(slot.style.flexGrow)).toBeGreaterThanOrEqual(1);
      expect(Number(slot.style.flexGrow)).toBeCloseTo(scene.ratios[index]!);
      expect(slot.style.flexBasis).toBe("0%");
      expect(slot.querySelectorAll("img")).toHaveLength(scene.pages[index] === null ? 0 : 1);
    });
  });
});

// A fresh, offline, headless profile measures actual CSS; it never attaches to
// the running application, changes its static server or reads account data.
const fsName = "node:fs", pathName = "node:path", osName = "node:os", childName = "node:child_process", processName = "node:process", urlName = "node:url";
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, content: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const { tmpdir } = await import(osName) as { tmpdir(): string };
const { env } = await import(processName) as { env: Record<string, string | undefined> };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: "utf8"; windowsHide: boolean }, callback: (error: Error | null, stdout: string, stderr: string) => void): void };
const browser = [env.ATSUMI_TEST_BROWSER ?? "", "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", "C:/Program Files/Microsoft/Edge/Application/msedge.exe"].find(candidate => fs.existsSync(candidate));
type Measurement = { name: string; viewport: { width: number; height: number }; stage: { x: number; width: number }; slots: { x: number; width: number; imageCenter: number | null; fit: string | null; background: string }[] };
async function measure(width: number, height: number): Promise<Measurement[]> {
  const directory = fs.mkdtempSync(path.join(tmpdir(), "atsumi-pixiv-centering-"));
  const scenes = cases.flatMap(scene => [320, 517, Math.min(1000, width - 48)].map(readerWidth => ({
    name: `${scene.name}-${readerWidth}`,
    html: `<div class="pixiv-reader-backdrop"><section class="pixiv-reader" style="width:${readerWidth}px;height:${Math.min(733, height - 48)}px">${markup(scene.dimensions, scene.pages)}</section></div>`,
  })));
  scenes.push(...cases.filter(scene => scene.pages.length === 1).map(scene => ({
    name: `detail-${scene.name}`,
    html: `<div class="pixiv-detail-backdrop"><section class="pixiv-detail"><header><strong>Local fixture</strong></header><div class="pixiv-detail-image">${markup(scene.dimensions, scene.pages)}</div><footer>Original metadata remains visible</footer></section></div>`,
  })));
  const script = `(async()=>{const scenes=${JSON.stringify(scenes)},results=[];for(const scene of scenes){document.getElementById('fixture').innerHTML=scene.html;await Promise.all([...document.images].map(image=>image.decode()));const stage=document.querySelector('.pixiv-original-pages').getBoundingClientRect();results.push({name:scene.name,viewport:{width:innerWidth,height:innerHeight},stage:{x:stage.x,width:stage.width},slots:[...document.querySelectorAll('.pixiv-original-slot')].map(slot=>{const box=slot.getBoundingClientRect(),image=slot.querySelector('img'),rect=image?.getBoundingClientRect();return{x:box.x,width:box.width,imageCenter:rect?rect.x+rect.width/2:null,fit:image?getComputedStyle(image).objectFit:null,background:getComputedStyle(slot).backgroundColor}})});}document.getElementById('metrics').textContent=JSON.stringify(results);})().catch(error=>document.getElementById('metrics').textContent=JSON.stringify({error:String(error)}));`;
  try {
    const input = path.join(directory, "fixture.html");
    fs.writeFileSync(input, `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'"><style>${baseCss}\n${inkCss}\n${platformCss}\n${workspaceCss}\n${readerCss}\nbody{margin:0}*,*::before,*::after{animation:none!important;transition:none!important}</style><main id="fixture"></main><pre id="metrics"></pre><script>${script}</script>`);
    const output = await new Promise<string>((resolve, reject) => execFile(browser!, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--host-resolver-rules=MAP * ~NOTFOUND", `--user-data-dir=${path.join(directory, "profile")}`, `--window-size=${width},${height}`, "--virtual-time-budget=3000", "--dump-dom", pathToFileURL(input).href], { timeout: 45000, maxBuffer: 4 * 1024 ** 2, encoding: "utf8", windowsHide: true }, (error, stdout, stderr) => error ? reject(new Error(stderr || error.message)) : resolve(stdout)));
    const result = /<pre id="metrics">([^<]+)<\/pre>/.exec(output)?.[1];
    if (!result) throw new Error("Offline browser returned no geometry measurements");
    const parsed = JSON.parse(result) as Measurement[] | { error: string };
    if (!Array.isArray(parsed)) throw new Error(parsed.error);
    return parsed;
  } finally {
    const relative = path.relative(fs.realpathSync(tmpdir()), fs.realpathSync(directory));
    if (!relative.startsWith("atsumi-pixiv-centering-") || relative.includes("..") || path.isAbsolute(relative)) throw new Error("Unexpected disposable fixture path");
    try { fs.rmSync(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); }
    catch (error) { if (!["EPERM", "EBUSY", "ENOTEMPTY"].includes((error as { code?: string }).code ?? "")) throw error; }
  }
}
describe.skipIf(!browser)("Pixiv real offline CSS geometry", () => {
  it.each([[960, 640], [1280, 800]])("centers complete slots across detail/readers in a %ix%i browser window", async (width, height) => {
    const measurements = await measure(width, height);
    expect(measurements).toHaveLength(30);
    console.info("Pixiv offline CSS measurements", { requestedWindow: [width, height], actualViewport: measurements[0]!.viewport, scenes: measurements.length });
    for (const result of measurements) {
      const slots = result.slots, first = slots[0]!, last = slots.at(-1)!;
      expect(Math.abs(first.x - result.stage.x), result.name).toBeLessThan(1);
      expect(Math.abs(last.x + last.width - result.stage.x - result.stage.width), result.name).toBeLessThan(1);
      const expected = cases.find(scene => result.name.slice(0, result.name.lastIndexOf("-")) === scene.name || result.name === `detail-${scene.name}`)!;
      expect(slots).toHaveLength(expected.pages.length);
      slots.forEach((slot, index) => {
        expect(Math.abs(slot.width / first.width - expected.ratios[index]! / expected.ratios[0]!), result.name).toBeLessThan(.02);
        if (slot.imageCenter !== null) {
          expect(Math.abs(slot.imageCenter - (slot.x + slot.width / 2)), result.name).toBeLessThan(1);
          expect(slot.fit, result.name).toBe("contain");
        } else expect(slot.background, result.name).toBe("rgb(0, 0, 0)");
      });
    }
  }, 60000);
});
