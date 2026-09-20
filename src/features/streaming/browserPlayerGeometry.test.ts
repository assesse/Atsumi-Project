// Local synthetic HTML only. No accounts, media, requests or user browser profile.
import { describe, expect, it } from "vitest";
import playerSource from "../../../src-tauri/src/streaming/browser_player_ui.js?raw";
import multiviewSource from "../../../src-tauri/src/streaming/browser_multiview.js?raw";
import autoSource from "../../../src-tauri/src/streaming/browser_auto_view.js?raw";

const fsName = "node:fs", pathName = "node:path", osName = "node:os", childName = "node:child_process", urlName = "node:url";
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, contents: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const { tmpdir } = await import(osName) as { tmpdir(): string };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: "utf8"; windowsHide: boolean }, callback: (error: Error | null, stdout: string) => void): void };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const processName = "node:process";
const { env } = await import(processName) as { env: Record<string, string | undefined> };
const edge = [env.ATSUMI_TEST_BROWSER ?? "", "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe"].find((candidate) => fs.existsSync(candidate));
type Box = { left: number; top: number; right: number; bottom: number; width: number; height: number };
type Result = {
  playerBefore: Box; playerAfter: Box; videoBefore: Box; videoAfter: Box;
  gear: Box; live: Box; login: Box; inline: boolean; gearCount: number; sameVideo: boolean;
  originalInput: string; inputAfter: string; initialHeight: number; focusedHeight: number; blurredHeight: number;
  oldPlaceholderHeight: number; contentSizing: boolean; nativeWideClicks: number; nativeLiveClicks: number;
  ownPresentation: boolean; bodyStyle: string | null; currentTime: number; messages: number;
  chatVisible: boolean; recordVisible: boolean; screenshotVisible: boolean; recordOnlyVisible: boolean;
  parkedChatHidden: boolean; returnedChatVisible: boolean; keptMute: boolean; keptInput: boolean;
};
const localSource = playerSource
  .replace('window.location.origin === "https://chzzk.naver.com"', 'window.location.protocol === "file:"')
  .replaceAll("window.location.pathname", '"/live/b3e262a2795f17734c149afc738ad250"');
const localMultiview = multiviewSource.replace('window.location.origin === "https://chzzk.naver.com"', 'window.location.protocol === "file:"')
  .replaceAll("window.location.pathname", '"/live/b3e262a2795f17734c149afc738ad250"');
const localAuto = autoSource.replace('location.origin !== "https://chzzk.naver.com"', 'location.protocol !== "file:"');
function fixture(inline: boolean, replaceHeader: boolean, attached = false): string {
  return `<!doctype html><html><head><meta charset="utf-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'">
  <style>
    html,body{margin:0;padding:0}body{font:14px/20px sans-serif;background:#123;min-height:1200px}
    #layout{display:flex;gap:16px;margin:40px 20px}.pzp-pc{position:relative;flex:0 0 760px;width:760px;height:428px;background:#000}
    video{width:100%;height:100%;object-fit:contain}.pzp-pc__bottom-buttons-right{position:absolute;right:12px;bottom:12px;display:flex;align-items:center}
    #official-top{position:absolute;right:12px;top:12px;display:${inline ? "flex" : "block"};align-items:center;gap:8px;height:36px;${inline ? "" : "width:200px"}}
    #official-live{height:28px;width:48px;padding:0;border:0;background:#e34;color:white;${inline ? "" : "position:absolute;right:104px;top:4px"}}
    #official-login{height:32px;width:88px;${inline ? "" : "position:absolute;right:0;top:2px"}}
    aside{width:280px;background:#234;color:white;padding:12px}textarea{font:14px/20px sans-serif;box-sizing:border-box;width:150px;min-height:26px;padding:2px;border:1px solid #777;field-sizing:content;resize:none}
    #atsumi-player-top-controls{transition:none!important}
  </style></head><body><div id="layout"><div class="pzp-pc" id="player"><video id="video"></video>
  <div id="official-top"><button id="official-live" type="button">LIVE</button><button id="official-login" type="button">로그인</button></div>
  <div class="pzp-pc__bottom-buttons-right"><button id="official-wide" type="button">넓게 보기</button></div></div>
  <aside>채팅<textarea rows="1" placeholder="채팅 입력"></textarea></aside></div>
  <script>
    window.addEventListener('error',e=>{const out=document.createElement('output');out.id='geometry-error';out.textContent=e.message;document.body.append(out)});
    const player=document.getElementById('player'),video=document.getElementById('video'),input=document.querySelector('textarea');
    const originalParent=video.parentElement;
    Object.defineProperties(video,{readyState:{value:4},videoWidth:{value:1920},videoHeight:{value:1080},paused:{value:false},currentTime:{value:100},seekable:{value:{length:1,end:()=>104.25}},buffered:{value:{length:1,end:()=>106}}});
    const box=node=>{const r=node.getBoundingClientRect();return{left:r.left,top:r.top,right:r.right,bottom:r.bottom,width:r.width,height:r.height}};
    const playerBefore=box(player),videoBefore=box(video),originalInput=input.outerHTML,initialHeight=box(input).height;
    let nativeWideClicks=0,nativeLiveClicks=0,messages=0;
    const wide=document.getElementById('official-wide');wide.onclick=()=>{nativeWideClicks++;wide.textContent=wide.textContent==='넓게 보기'?'좁게 보기':'넓게 보기'};
    document.getElementById('official-live').onclick=()=>nativeLiveClicks++;
    window.chrome=window.chrome||{};window.chrome.webview={postMessage:()=>messages++};
    ${attached ? localAuto + "\n" + localMultiview : ""}
    ${localSource}
    window.__atsumiPlayerUI.update({ready:true,recording:false,detail:'ready'});
    input.focus();window.__atsumiPlayerUI.update({ready:true,recording:false,detail:'ready'});const focusedHeight=box(input).height;
    input.blur();window.dispatchEvent(new Event('scroll'));
    ${attached ? `setTimeout(()=>{
      window.dispatchEvent(new CustomEvent('atsumi-multiview-audio',{detail:{enabled:true,applyToMedia:false}}));
      window.__atsumiAutoReceiver.configure({revision:1,viewing:true});
      window.__atsumiPlayerUI.update({ready:true,recording:true,detail:'recording'});
    },350);` : ""}
    const originalHeader=document.getElementById('official-top');
    if(${replaceHeader}){const fresh=document.createElement('div');fresh.id='official-top';fresh.innerHTML='<button id="official-live" type="button">LIVE</button><button id="official-login" type="button">로그인</button>';originalHeader.replaceWith(fresh);fresh.querySelector('#official-live').onclick=()=>nativeLiveClicks++;window.__atsumiPlayerUI.update({ready:true,recording:false,detail:'ready'});}
    setTimeout(()=>{
      const top=document.getElementById('atsumi-player-top-controls');top.querySelector('button').focus();
      wide.click();wide.click();document.getElementById('official-live').click();
      const blurredHeight=box(input).height;
      // Positive control illustrates why mutating a placeholder is unsafe for
      // content-sized inputs; it is not an assertion about CHZZK's hidden code.
      const positive=input.cloneNode();positive.id='positive';positive.placeholder='지연 약 104.3초 · 채팅 입력';document.querySelector('aside').append(positive);
      const visible=node=>!!node&&!node.hidden&&getComputedStyle(node).display!=='none'&&node.getBoundingClientRect().width>0;
      const result={playerBefore,playerAfter:box(player),videoBefore,videoAfter:box(video),gear:box(top),live:box(document.getElementById('official-live')),login:box(document.getElementById('official-login')),
        inline:top.hasAttribute('data-inline'),gearCount:document.querySelectorAll('#atsumi-player-top-controls').length,sameVideo:document.getElementById('video')===video&&video.parentElement===originalParent,
        originalInput,inputAfter:input.outerHTML,initialHeight,focusedHeight,blurredHeight,oldPlaceholderHeight:box(positive).height,contentSizing:CSS.supports('field-sizing','content'),
        nativeWideClicks,nativeLiveClicks,ownPresentation:!!document.querySelector('[data-presentation],#atsumi-presentation-toggle,[data-atsumi-clean]'),bodyStyle:document.body.getAttribute('style'),currentTime:video.currentTime,messages,
        chatVisible:visible(document.querySelector('aside')),recordVisible:visible(document.querySelector('[aria-label="녹화 중지"]')),
        screenshotVisible:visible(document.querySelector('[aria-label="스크린샷"]')),recordOnlyVisible:visible(document.querySelector('[aria-label="녹화만 계속"]'))};
      if(${attached}){
        input.value='작성 중인 채팅';video.muted=true;
        window.__atsumiAutoReceiver.configure({revision:2,viewing:true});
        result.keptMute=video.muted;
        window.__atsumiAutoReceiver.configure({revision:3,viewing:false});
        result.parkedChatHidden=!visible(document.querySelector('aside'));
        window.__atsumiAutoReceiver.configure({revision:4,viewing:true});
        result.returnedChatVisible=visible(document.querySelector('aside'));
        result.keptInput=input===document.querySelector('textarea')&&input.value==='작성 중인 채팅';
        result.sameVideo=result.sameVideo&&video===document.querySelector('video');
      }
      const out=document.createElement('output');out.id='geometry-result';out.setAttribute('data-result',encodeURIComponent(JSON.stringify(result)));document.body.append(out);
    },1200);
  </script></body></html>`;
}
async function render(inline: boolean, replaceHeader = false, attached = false): Promise<Result> {
  const root = fs.mkdtempSync(path.join(tmpdir(), "atsumi-player-geometry-"));
  try {
    const html = path.join(root, "fixture.html");
    fs.writeFileSync(html, fixture(inline, replaceHeader, attached));
    const stdout = await new Promise<string>((resolve, reject) => {
      execFile(edge!, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--host-resolver-rules=MAP * ~NOTFOUND",
        `--user-data-dir=${path.join(root, "profile")}`, "--window-size=1280,800", "--force-device-scale-factor=1", "--virtual-time-budget=2200", "--dump-dom", pathToFileURL(html).href],
      { timeout: 20_000, maxBuffer: 2 * 1024 * 1024, encoding: "utf8", windowsHide: true }, (error, output) => error ? reject(error) : resolve(output));
    });
    const serialized = /<output id="geometry-result" data-result="([^"]+)"/.exec(stdout)?.[1];
    if (!serialized) throw new Error(`Isolated player geometry returned no result: ${(/<output id="geometry-error">([^<]*)<\/output>/.exec(stdout)?.[1] || stdout.slice(0, 700) || "browser exited without DOM output").slice(0, 700)}`);
    return JSON.parse(decodeURIComponent(serialized)) as Result;
  } finally {
    const relative = path.relative(fs.realpathSync(tmpdir()), fs.realpathSync(root));
    if (!relative.startsWith("atsumi-player-geometry-") || relative.includes("..") || path.isAbsolute(relative)) throw new Error("Unexpected player geometry fixture path");
    fs.rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
}

describe.skipIf(!edge)("official player passthrough rendered geometry", () => {
  it("renders the same live chat and controls when attaching an automatic receiver", async () => {
    const result = await render(true, false, true);
    expect(result.chatVisible).toBe(true); expect(result.recordVisible).toBe(true);
    expect(result.screenshotVisible).toBe(true); expect(result.recordOnlyVisible).toBe(true);
    expect(result.playerAfter).toEqual(result.playerBefore); expect(result.videoAfter).toEqual(result.videoBefore);
    expect(result.sameVideo).toBe(true); expect(result.keptInput).toBe(true); expect(result.keptMute).toBe(true);
    expect(result.parkedChatHidden).toBe(true); expect(result.returnedChatVisible).toBe(true);
    expect(result.gearCount).toBe(1); expect(result.messages).toBe(0);
  }, 30_000);
  it.each([false, true])("places gear beside official LIVE in its flex row, header replacement=%s", async (replaceHeader) => {
    const result = await render(true, replaceHeader);
    expect(result.inline).toBe(true);
    expect(result.gearCount).toBe(1);
    expect(result.live.left - result.gear.right).toBeCloseTo(8, 0);
    expect(result.gear.left).toBeGreaterThanOrEqual(result.playerAfter.left);
    expect(result.gear.right).toBeLessThan(result.login.left);
    expect(result.playerAfter).toEqual(result.playerBefore);
    expect(result.videoAfter).toEqual(result.videoBefore);
    expect(result.sameVideo).toBe(true);
    expect(result.ownPresentation).toBe(false);
    expect(result.bodyStyle).toBeNull();
    expect(result.nativeWideClicks).toBe(2);
    expect(result.nativeLiveClicks).toBe(1);
    expect(result.currentTime).toBe(100);
    expect(result.messages).toBe(0);
    expect(result.inputAfter).toBe(result.originalInput);
    expect(result.focusedHeight).toBe(result.initialHeight);
    expect(result.blurredHeight).toBe(result.initialHeight);
    expect(result.contentSizing).toBe(true);
    expect(result.oldPlaceholderHeight).toBeGreaterThan(result.initialHeight);
  }, 30_000);
  it("keeps an absolute-layout fallback adjacent without altering player/input sizing", async () => {
    const result = await render(false);
    expect(result.inline).toBe(false);
    expect(result.live.left - result.gear.right).toBeCloseTo(8, 0);
    expect(result.gear.right).toBeLessThan(result.login.left);
    expect(result.playerAfter).toEqual(result.playerBefore);
    expect(result.videoAfter).toEqual(result.videoBefore);
    expect(result.inputAfter).toBe(result.originalInput);
    expect(result.blurredHeight).toBe(result.initialHeight);
  }, 30_000);
});
