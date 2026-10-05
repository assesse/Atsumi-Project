// Measure the production styles in an isolated, offline browser profile.
// No app, account, media, or user database is opened by this fixture.
import { describe, expect, it } from "vitest";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";

const fsName = "node:fs", pathName = "node:path", osName = "node:os", childName = "node:child_process", urlName = "node:url", processName = "node:process";
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, contents: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const { tmpdir } = await import(osName) as { tmpdir(): string };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: "utf8"; windowsHide: boolean }, callback: (error: Error | null, stdout: string) => void): void };
const { env } = await import(processName) as { env: Record<string, string | undefined> };
const edge = [env.ATSUMI_TEST_BROWSER ?? "", "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe"].find(candidate => fs.existsSync(candidate));
type Measurement = {
  buttons: { label: string; width: number; height: number; rowWidth: number; overflow: boolean }[];
  maskOff: string;
  mask: { content: string; position: string; inset: string; opacity: string; display: string; backdrop: string; animation: string };
  reducedMotion: boolean;
  animations: string[];
  hoverCards: { kind: string; beforeTop: number; afterTop: number; clipTop: number; transform: string; overflowY: string }[];
  search: { inputWidth: number; inputRight: number; sortLeft: number; sortRight: number; boxRight: number };
};

async function measure(width: number, reducedMotion: boolean): Promise<Measurement> {
  const directory = fs.mkdtempSync(path.join(tmpdir(), "atsumi-design-policy-"));
  try {
    const fixture = path.join(directory, "fixture.html");
    fs.writeFileSync(fixture, `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'"><style>${appCss.replaceAll(":hover", ".fixture-hover")}\n${inkCss.replaceAll(":hover", ".fixture-hover")}\n#fixture{width:min(100%,1000px);padding:24px}.settings-form{display:block}.card-clip-fixture{height:50px;margin:8px 0}.card-clip-fixture>article{height:45px;transition:none}</style></head><body><div id="fixture">
      <div class="settings-form"><div class="maintenance-list">
        <article class="maintenance-item"><div class="maintenance-copy"><strong>빠른 복구</strong><p>저장된 앨범과 원본 파일은 유지됩니다.</p></div><button class="text-button">빠른 복구</button></article>
        <article class="maintenance-item"><div class="maintenance-copy"><strong>라이브러리 검사 및 재구축</strong><p>파일을 검사하고 파생 데이터를 다시 만듭니다.</p></div><button class="text-button">라이브러리 검사 및 재구축</button></article>
        <article class="maintenance-item maintenance-item--factory-reset"><div class="maintenance-copy"><strong>앱 데이터 완전 초기화</strong><p>외부 원본 파일은 유지됩니다.</p></div><button class="text-button danger-button">앱 데이터 완전 초기화</button></article>
      </div><div class="setting-row settings-reset-row"><div><strong>설정 초기화</strong><span>저장을 눌러야 적용됩니다.</span></div><button class="text-button">설정 기본값</button></div>
      <div class="setting-row download-folder-setting"><div><strong>다운로드 폴더</strong></div><div class="setting-path-field"><div class="setting-path-control"><input aria-label="다운로드 폴더" value="D:\\Albums"><button class="text-button">폴더 선택</button></div></div></div></div>
      <header class="view-header"><form class="search-box"><span class="fluent"></span><input aria-label="검색"><div class="search-sort-control"><select><option>연간 인기순</option></select></div></form>${"<button class='icon-button'>·</button>".repeat(6)}</header>
      ${["gallery-card", "related-card", "danbooru-card"].map(kind => `<div class="gallery-viewport card-clip-fixture"><article class="${kind}">카드</article></div>`).join("")}
      <section id="privacy"><button class="danbooru-related-card" aria-label="관계 미리보기"><span>123</span></button></section>
      <span class="spinner"></span><aside class="startup-status is-loading"><span class="startup-status-icon"></span>준비 중</aside><div class="danbooru-relations-status"><span></span>관계 확인 중</div>
      </div><script>
      const card=document.querySelector('.danbooru-related-card'),maskOff=getComputedStyle(card,'::after').content;
      document.getElementById('privacy').dataset.privacyMode='on';
      const mask=getComputedStyle(card,'::after');
      const hoverCards=[...document.querySelectorAll('.card-clip-fixture>article')].map(element=>{const kind=element.className,beforeTop=element.getBoundingClientRect().top,clipTop=element.parentElement.getBoundingClientRect().top;element.classList.add('fixture-hover');return{kind,beforeTop,afterTop:element.getBoundingClientRect().top,clipTop,transform:getComputedStyle(element).transform,overflowY:getComputedStyle(element.parentElement).overflowY};});
      const input=document.querySelector('.search-box input').getBoundingClientRect(),sort=document.querySelector('.search-sort-control').getBoundingClientRect(),box=document.querySelector('.search-box').getBoundingClientRect();
      const result={hoverCards,search:{inputWidth:input.width,inputRight:input.right,sortLeft:sort.left,sortRight:sort.right,boxRight:box.right},buttons:[...document.querySelectorAll('.settings-form button')].map(button=>{const b=button.getBoundingClientRect(),row=button.parentElement.getBoundingClientRect();return{label:button.textContent,width:b.width,height:b.height,rowWidth:row.width,overflow:b.right>row.right||b.left<row.left}}),maskOff,mask:{content:mask.content,position:mask.position,inset:mask.inset,opacity:mask.opacity,display:mask.display,backdrop:mask.backdropFilter,animation:mask.animationName},reducedMotion:matchMedia('(prefers-reduced-motion: reduce)').matches,animations:[...document.querySelectorAll('.spinner,.startup-status-icon,.danbooru-relations-status > span')].map(element=>getComputedStyle(element).animationName)};
      const output=document.createElement('output');output.id='design-policy-result';output.setAttribute('data-result',encodeURIComponent(JSON.stringify(result)));document.body.append(output);
      </script></body></html>`);
    const stdout = await new Promise<string>((resolve, reject) => execFile(edge!, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--host-resolver-rules=MAP * ~NOTFOUND", `--user-data-dir=${path.join(directory, "profile")}`, `--window-size=${width},900`, "--force-device-scale-factor=1", ...(reducedMotion ? ["--force-prefers-reduced-motion"] : []), "--virtual-time-budget=500", "--dump-dom", pathToFileURL(fixture).href], { timeout: 20000, maxBuffer: 4 * 1024 ** 2, encoding: "utf8", windowsHide: true }, (error, output) => error ? reject(error) : resolve(output)));
    const serialized = /<output id="design-policy-result" data-result="([^"]+)"/.exec(stdout)?.[1];
    if (!serialized) throw new Error("The isolated design-policy fixture returned no measurements");
    return JSON.parse(decodeURIComponent(serialized)) as Measurement;
  } finally {
    const relative = path.relative(fs.realpathSync(tmpdir()), fs.realpathSync(directory));
    if (!relative.startsWith("atsumi-design-policy-") || relative.includes("..") || path.isAbsolute(relative)) throw new Error("Unexpected design-policy fixture path");
    fs.rmSync(directory, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
}

describe.skipIf(!edge)("design-policy browser layout", () => {
  it.each([[900, false], [1440, false], [900, true]] as const)("keeps settings compact and privacy intact at %ipx (reduced motion=%s)", async (width, reducedMotion) => {
    const result = await measure(width, reducedMotion);
    for (const card of result.hoverCards) {
      expect(card.afterTop, card.kind).toBe(card.beforeTop);
      expect(card.afterTop, card.kind).toBeGreaterThanOrEqual(card.clipTop);
      expect(card.transform, card.kind).toBe("none");
      expect(card.overflowY, card.kind).toBe("auto");
    }
    expect(result.search.inputWidth).toBeGreaterThan(80);
    expect(result.search.inputRight).toBeLessThanOrEqual(result.search.sortLeft);
    expect(result.search.sortRight).toBeLessThan(result.search.boxRight);
    for (const button of result.buttons) {
      expect(button.height, button.label).toBeGreaterThanOrEqual(32);
      expect(button.height, button.label).toBeLessThanOrEqual(38);
      expect(button.width, button.label).toBeLessThan(240);
      expect(button.width, button.label).toBeLessThan(button.rowWidth / 2);
      expect(button.overflow, button.label).toBe(false);
    }
    expect(result.buttons[0]!.width).toBeLessThan(result.buttons[1]!.width);
    expect(result.maskOff).toBe("none");
    expect(result.mask).toMatchObject({ content: '\"\"', position: "absolute", inset: "0px", opacity: "1", animation: "none" });
    expect(result.mask.display).not.toBe("none");
    expect(result.mask.backdrop).toContain("blur(28px)");
    if (reducedMotion) expect(result.reducedMotion).toBe(true);
    for (const animation of result.animations) {
      if (result.reducedMotion) expect(animation).toBe("none");
      else expect(animation).not.toBe("none");
    }
  }, 30000);
});
