// Offline Chromium layout checks, isolated profile and DNS disabled. No user UI.
import { describe, expect, it, vi } from "vitest";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { StreamingWorkspace } from "./StreamingWorkspace";
import { ConnectionSetup } from "./ConnectionSetup";
import { LiveChannelPicker } from "./LiveChannelPicker";
import { RecordingReplay } from "./RecordingReplay";
import { PLAYER_CHANNEL } from "./OriginalChzzkPlayer";
import type { ReplayApi } from "../../api/replay";
import multiview from "../../../src-tauri/src/streaming/browser_multiview.js?raw";
import playerUI from "../../../src-tauri/src/streaming/browser_player_ui.js?raw";
import madoCss from "./MadoWorkspace.css?raw";
import feedbackCss from "../feedback/FeedbackDialog.css?raw";
import streamingCss from "./StreamingWorkspace.css?raw";
import edgeCss from "./LiveEdgePanel.css?raw";
import shellCss from "../../styles.css?raw";
import officialCss from "./OfficialBrowserPanel.css?raw";
import pickerCss from "./LiveChannelPicker.css?raw";
import replayCss from "./RecordingReplay.css?raw";
const fsName = 'node:fs', pathName = 'node:path', osName = 'node:os', childName = 'node:child_process', urlName = 'node:url', processName = 'node:process';
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, text: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(a: string, b: string): string; isAbsolute(value: string): boolean };
const { tmpdir } = await import(osName) as { tmpdir(): string };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: 'utf8'; windowsHide: boolean }, done: (error: Error | null, stdout: string) => void): void };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const { env } = await import(processName) as { env: Record<string, string | undefined> };
const edge = [env.ATSUMI_TEST_BROWSER ?? '', 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe', 'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe'].find(file => fs.existsSync(file));
const script = multiview.replace('window.location.origin === "https://chzzk.naver.com"', 'window.location.protocol === "file:"').replaceAll('window.location.pathname', '"/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"');
async function render(html: string, options: { width?: number; height?: number; reducedMotion?: boolean; screenshot?: string } = {}) {
  const root = fs.mkdtempSync(path.join(tmpdir(), 'atsumi-ui-geometry-'));
  try {
    const file = path.join(root, 'fixture.html'); fs.writeFileSync(file, html);
    const output = await new Promise<string>((resolve, reject) => execFile(edge!, ['--headless', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-component-update', '--disable-sync', '--host-resolver-rules=MAP * ~NOTFOUND', `--user-data-dir=${path.join(root, 'profile')}`, `--window-size=${options.width ?? 1920},${options.height ?? 1080}`, '--force-device-scale-factor=1', '--virtual-time-budget=2400', ...(options.reducedMotion ? ['--force-prefers-reduced-motion=reduce'] : []), ...(options.screenshot ? [`--screenshot=${options.screenshot}`] : []), '--dump-dom', pathToFileURL(file).href], { timeout: 20000, maxBuffer: 2 * 1024 * 1024, encoding: 'utf8', windowsHide: true }, (error, value) => error ? reject(error) : resolve(value)));
    const serialized = /<output id="result" data-value="([^"]+)"/.exec(output)?.[1];
    if (!serialized) throw new Error('Offline layout fixture returned no result');
    return JSON.parse(decodeURIComponent(serialized)) as Record<string, number | boolean>;
  } finally {
    const relative = path.relative(fs.realpathSync(tmpdir()), fs.realpathSync(root));
    if (!relative.startsWith('atsumi-ui-geometry-') || relative.includes('..') || path.isAbsolute(relative)) throw new Error('Unexpected fixture path');
    fs.rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
}
const head = `<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'">`;
const report = `const report=value=>{const out=document.createElement('output');out.id='result';out.dataset.value=encodeURIComponent(JSON.stringify(value));document.body.append(out)};`;
describe.skipIf(!edge)('offline live UI rendered geometry', () => {
  it.each([[960, 2], [1920, 12], [500, 12]])('centers favorites and their controls at %spx with %s channels', async (width, count) => {
    const favorites = Array.from({ length: count }, (_, index) => index.toString(16).padStart(32, 'a'));
    const inputs = favorites.slice(0, 2);
    const channelsApi = {
      snapshot: vi.fn(async () => ({ ok: true as const, data: favorites.map((channelId, index) => ({ channelId, channelName: `테스트 채널 ${index + 1}` })) })),
      profile: vi.fn(async () => ({ ok: true as const, data: { channelName: '테스트 채널', image: null, liveStatus: 'live' as const } })), set: vi.fn(),
    };
    const autoApi = { snapshot: vi.fn(async () => ({ ok: true as const, data: { channels: [], captureChat: true, error: null } })), add: vi.fn(), update: vi.fn() };
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
    const host = document.createElement('div'); document.body.append(host); const root = createRoot(host);
    let app = '';
    try {
      await act(async () => root.render(createElement(ConnectionSetup, {
        runtime: 'tauri', privacy: false, channelsApi, autoApi, mode: 'mado', unified: true, onMode() {}, modeDisabled: false,
        inputs, onInput() {}, disabled: false, pending: false, onConnect() {}, auth: 'signed_in', accountDisabled: false,
        onLogin() {}, onLogout() {}, onGrid() {}, gridDisabled: false, gridStatus: '', onInstaller() {}, installerDisabled: false,
        channelPicker: createElement(LiveChannelPicker, { runtime: 'tauri', inputs, multiple: true, privacy: false, disabled: false, onInputs() {} }),
      })));
      app = host.innerHTML;
    } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
    const screenshot = env.ATSUMI_UI_CAPTURE_DIR ? path.join(env.ATSUMI_UI_CAPTURE_DIR, `favorite-alignment-${width}.png`) : undefined;
    const result = await render(`<!doctype html><html><head>${head}<style>${shellCss}\n${officialCss}\n${pickerCss}\n${madoCss}\n${streamingCss}
      .connection-setup{margin:24px auto}
    </style></head><body><main class="streaming-workspace"><section class="mado-workspace">${app}</section></main><script>${report}
      setTimeout(()=>{
        const controls=[...document.querySelectorAll('.connection-channel-favorite,.live-channel-actions>button')];
        const delta=(a,b)=>Math.max(Math.abs((a.left+a.right-b.left-b.right)/2),Math.abs((a.top+a.bottom-b.top-b.bottom)/2));
        const icons=controls.map(button=>delta(button.getBoundingClientRect(),button.querySelector('svg').getBoundingClientRect()));
        const paint=controls.filter(button=>button.querySelector('path')).map(button=>{const p=button.querySelector('path').getBoundingClientRect();return delta(button.getBoundingClientRect(),p)});
        const rows=[...document.querySelectorAll('.connection-channel-row')].map(row=>{const a=row.querySelector('input').getBoundingClientRect(),b=row.querySelector('.connection-channel-favorite').getBoundingClientRect();return Math.abs((a.top+a.bottom-b.top-b.bottom)/2)});
        const list=document.querySelector('.live-channel-list'),listBox=list.getBoundingClientRect(),formBox=document.querySelector('.connection-setup').getBoundingClientRect();
        const cards=[...list.querySelectorAll('.live-channel-row')].map(el=>el.getBoundingClientRect());
        const center=(Math.min(...cards.map(c=>c.left))+Math.max(...cards.map(c=>c.right)))/2;
        report({controls:controls.length,iconDelta:Math.max(...icons),paintDelta:Math.max(...paint),rowDelta:Math.max(...rows),listCenterDelta:Math.abs(center-(listBox.left+listBox.right)/2),formCenterDelta:Math.abs(center-(formBox.left+formBox.right)/2),scrolls:list.scrollHeight>list.clientHeight,rowsVisible:list.clientHeight<=4*88+3*8});
      },100);</script></body></html>`, { width, height: 850, screenshot });
    expect(result.controls).toBe(2 + count * 2);
    expect(result.iconDelta).toBeLessThan(.5);
    expect(result.paintDelta).toBeLessThan(.5);
    expect(result.rowDelta).toBeLessThan(.5);
    expect(result.listCenterDelta).toBeLessThan(.5);
    expect(result.formCenterDelta).toBeLessThan(.5);
    expect(result.scrolls).toBe(count > 8);
    expect(result.rowsVisible).toBe(true);
  }, 30000);
  it.each([[1920, 1080], [960, 640], [500, 850]])('centers EOF replay over the video without blocking other controls at %sx%s', async (width, height) => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
    vi.stubGlobal('innerWidth', width); vi.stubGlobal('innerHeight', height);
    const host = document.createElement('div'); document.body.append(host); const root = createRoot(host);
    const emptyPage = { ok: true as const, data: { generation: 0, items: [], previousCursor: null, nextCursor: null, indexState: 'ready' as const, syncQuality: 'receive_time_approximate' as const, warnings: [] } };
    const api: ReplayApi = {
      open: async () => ({ ok: true, data: { token: 'synthetic', recordingId: 'synthetic', title: '다시보기 테스트', recordedAt: 0, durationSeconds: 30, mimeType: 'video/mp4', chatStatus: 'stopped', indexState: 'ready', syncQuality: 'receive_time_approximate', manualOffsetSeconds: 0, warnings: [] } }),
      chatAt: async () => emptyPage, chatPage: async () => emptyPage, close: async () => ({ ok: true, data: undefined }), setOffset: async () => ({ ok: true, data: 0 }),
      timeline: async () => ({ ok: true, data: { bucketSeconds: 2, indexState: 'ready', viewerMetricStatus: 'not_recorded', buckets: [] } }), mediaUrl: () => 'http://atsumi-replay.localhost/synthetic',
    };
    let app = '';
    try {
      await act(async () => root.render(createElement(RecordingReplay, { recordingId: 'synthetic', runtime: 'browser-mock', privacyMode: false, liveRecording: false, api, onClose() {} })));
      const frame = document.querySelector<HTMLIFrameElement>('.recording-replay-original-player')!;
      await act(async () => window.dispatchEvent(new MessageEvent('message', { source: frame.contentWindow, origin: 'null', data: { channel: PLAYER_CHANNEL, nonce: new URL(frame.src).hash.slice(1), type: 'tail' } })));
      app = document.querySelector('.recording-replay-backdrop')!.outerHTML;
    } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
    const screenshot = env.ATSUMI_UI_CAPTURE_DIR ? path.join(env.ATSUMI_UI_CAPTURE_DIR, `replay-ended-${width}x${height}.png`) : undefined;
    const result = await render(`<!doctype html><html><head>${head}<style>${shellCss}\n${replayCss}</style></head><body>${app}<script>${report}
      setTimeout(()=>{
        const restart=document.querySelector('.recording-replay-restart'),frame=document.querySelector('iframe'),chat=document.querySelector('.recording-replay-chat');
        const b=restart.getBoundingClientRect(),v=frame.getBoundingClientRect(),c=chat.getBoundingClientRect();
        report({centerDelta:Math.max(Math.abs((b.left+b.right-v.left-v.right)/2),Math.abs((b.top+b.bottom-v.top-v.bottom)/2)),inside:b.left>=v.left&&b.right<=v.right&&b.top>=v.top&&b.bottom<=v.bottom,centerClickable:document.elementFromPoint((b.left+b.right)/2,(b.top+b.bottom)/2)?.closest('.recording-replay-restart')===restart,seekUnblocked:document.elementFromPoint(v.left+20,v.bottom-20)===frame,chatUnblocked:document.elementFromPoint(c.left+10,c.top+10)===chat});
      },300);</script></body></html>`, { width, height, screenshot });
    expect(result.centerDelta).toBeLessThan(.5);
    expect(result).toMatchObject({ inside: true, centerClickable: true, seekUnblocked: true, chatUnblocked: true });
  }, 30000);
  it.each([[960, 640], [1920, 1080]])('keeps the same handle attached above the open sheet at %sx%s', async (width, height) => {
    localStorage.removeItem('atsumi.chzzk.view.v1');
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
    const host = document.createElement('div'); document.body.append(host);
    const root = createRoot(host);
    let app = '';
    try {
      await act(async () => root.render(createElement(StreamingWorkspace, { runtime: 'browser-mock', active: true, railCollapsed: false, privacyMode: false, onToggleRail() {}, onSourceChange() {} })));
      await act(async () => host.querySelector<HTMLButtonElement>('.live-bottom-handle')!.click());
      app = host.innerHTML;
    } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
    const screenshot = env.ATSUMI_UI_CAPTURE_DIR ? path.join(env.ATSUMI_UI_CAPTURE_DIR, `live-sheet-${width}x${height}.png`) : undefined;
    const result = await render(`<!doctype html><html><head>${head}<style>${shellCss}\n${officialCss}\n${pickerCss}\n${madoCss}\n${edgeCss}\n${streamingCss}</style></head><body><div id="root">${app}</div>
      <script>${report}setTimeout(()=>{
        const handle=document.querySelector('.live-bottom-handle'),surface=document.querySelector('.live-connection-surface'),main=document.querySelector('main');
        const h=handle.getBoundingClientRect(),s=surface.getBoundingClientRect();
        report({centered:Math.abs((h.left+h.right-s.left-s.right)/2)<.5,attached:Math.abs(h.bottom-s.top)<.5,bottom:Math.abs(s.bottom-innerHeight)<.5,inside:h.top>=0&&s.left>=0&&s.right<=innerWidth,noMainScroll:main.scrollHeight===main.clientHeight,open:handle.getAttribute('aria-expanded')==='true',noAnimations:handle.getAnimations().length===0&&surface.getAnimations({subtree:true}).every(a=>a.effect.getComputedTiming().iterations!==Infinity),oneScrollSurface:surface.scrollHeight>surface.clientHeight?getComputedStyle(surface).overflowY==='auto':true});
      },100);</script></body></html>`, { width, height, screenshot });
    expect(result).toMatchObject({ centered: true, attached: true, bottom: true, inside: true, noMainScroll: true, open: true, noAnimations: true, oneScrollSurface: true });
  }, 30000);
  it.each([[960, 640, false], [1280, 720, false], [1920, 1080, false], [3440, 1392, true]] as const)('docks the empty live handle to the real window edge at %sx%s (reduced motion: %s)', async (width, height, reducedMotion) => {
    localStorage.removeItem('atsumi.chzzk.view.v1');
    const app = renderToStaticMarkup(createElement(StreamingWorkspace, { runtime: 'browser-mock', active: true, railCollapsed: false, privacyMode: false, onToggleRail() {}, onSourceChange() {} }));
    const screenshot = env.ATSUMI_UI_CAPTURE_DIR ? path.join(env.ATSUMI_UI_CAPTURE_DIR, `live-entry-${width}x${height}.png`) : undefined;
    const result = await render(`<!doctype html><html><head>${head}<style>${shellCss}\n${madoCss}\n${edgeCss}\n${streamingCss}</style></head><body><div id="root">${app}</div>
      <script>${report}setTimeout(()=>{
        const handle=document.querySelector('.live-bottom-handle'),guide=document.querySelector('.live-empty-copy'),arrow=document.querySelector('.live-empty-direction'),main=document.querySelector('main');
        const box=el=>el.getBoundingClientRect();
        const b=box(handle),g=box(guide),a=box(arrow),m=box(main);
        report({docked:Math.abs(b.bottom-innerHeight)<.5,centered:Math.abs((b.left+b.right-m.left-m.right)/2)<.5,noScroll:main.scrollHeight===main.clientHeight&&document.documentElement.scrollHeight===innerHeight,guideFits:g.top>=0&&g.bottom<a.top,arrowPointsAtHandle:a.bottom<b.top&&Math.abs((a.left+a.right-b.left-b.right)/2)<.5,compact:b.height===22&&b.width===72,pointer:getComputedStyle(handle).cursor==='pointer',noText:handle.textContent==='',reduced:matchMedia('(prefers-reduced-motion: reduce)').matches,animated:handle.getAnimations({subtree:true}).length>0});
      },100);</script></body></html>`, { width, height, reducedMotion, screenshot });
    expect(result).toMatchObject({ docked: true, centered: true, noScroll: true, guideFits: true, arrowPointsAtHandle: true, compact: true, pointer: true, noText: true, reduced: reducedMotion, animated: false });
  }, 30000);
  it('fades added controls with the native PZP class, even while the old idle timer disagrees', async () => {
    const ui = playerUI.replace('window.location.origin === "https://chzzk.naver.com"', 'window.location.protocol === "file:"').replaceAll('window.location.pathname', '"/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"');
    // Native timing observed in signed-in playback: opacity 0.2s ease-in.
    const result = await render(`<!doctype html><html><head>${head}<style>
      .pzp-pc{width:800px;height:450px;position:relative}.pzp-pc__bottom-buttons-right{display:flex}.native{opacity:0;transition:opacity .2s ease-in}.pzp-pc--controls .native{opacity:1}
    </style></head><body><div class="pzp-pc"><video></video><div class="pzp-pc__bottom-buttons-right"><button class="native pzp-pc__fullscreen-button">Fullscreen</button></div></div>
    <script>${report}window.chrome=window.chrome||{};window.chrome.webview={postMessage(){}};
    const v=document.querySelector('video');Object.defineProperties(v,{readyState:{value:4},videoWidth:{value:1920},videoHeight:{value:1080},paused:{value:false}});
    ${ui}
    const p=document.querySelector('.pzp-pc'),a=document.querySelector('#atsumi-player-controls'),b=document.querySelector('.native');const opacity=n=>Number(getComputedStyle(n).opacity);let first,showDelta,hideDelta;
    document.documentElement.dataset.atsumiControls='visible';
    setTimeout(()=>{first=opacity(a);p.classList.add('pzp-pc--controls');document.documentElement.dataset.atsumiControls='idle';},150);
    setTimeout(()=>{showDelta=Math.abs(opacity(a)-opacity(b));},240);
    setTimeout(()=>{p.classList.remove('pzp-pc--controls');document.documentElement.dataset.atsumiControls='visible';},450);
    setTimeout(()=>{hideDelta=Math.abs(opacity(a)-opacity(b));},540);
    setTimeout(()=>report({first,showDelta,hideDelta,last:opacity(a),sameTransition:getComputedStyle(a).transition===getComputedStyle(b).transition}),800);
    </script></body></html>`);
    expect(result).toMatchObject({first:0,last:0,sameTransition:true});
    expect(result.showDelta).toBeLessThan(.03); expect(result.hideDelta).toBeLessThan(.03);
  }, 30000);
  it('keeps the feedback cross centered and single-chat width at 160px on a 1920px layout', async () => {
    const result = await render(`<!doctype html><html><head>${head}<style>${madoCss}\n${feedbackCss}
      body{margin:0}.mado-surfaces{width:1920px;height:800px;--mado-main-share:94fr;--mado-chat-share:6fr}.feedback-dialog{display:block;position:absolute;top:20px;left:20px}
    </style></head><body><div class="mado-surfaces" data-channels="1"><div id="video"></div><div></div><div id="chat"></div></div>
    <div class="feedback-dialog"><header><h2>오류·의견 보내기</h2><button class="feedback-close"><svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18"/></svg></button></header></div>
    <script>${report}setTimeout(()=>{const b=document.querySelector('.feedback-close').getBoundingClientRect(),i=document.querySelector('svg').getBoundingClientRect();report({chat:document.getElementById('chat').getBoundingClientRect().width,video:document.getElementById('video').getBoundingClientRect().width,dx:Math.abs((b.left+b.right-i.left-i.right)/2),dy:Math.abs((b.top+b.bottom-i.top-i.bottom)/2),button:b.width});},50)</script></body></html>`);
    expect(result.chat).toBeCloseTo(160, 0); expect(result.video).toBeCloseTo(1750, 0);
    expect(result.dx).toBeLessThan(.5); expect(result.dy).toBeLessThan(.5); expect(result.button).toBe(32);
  }, 30000);
  it('keeps redundant official side panes hidden while retaining the app chat action and full video width', async () => {
    const result = await render(`<!doctype html><html><head>${head}<style>
      body{margin:0}.chzzk_player{width:800px;height:450px}.player_header{position:absolute;top:0;right:0}
      #aside-chatting{position:fixed;right:0;top:0;width:380px;height:100vh;display:flex}
    </style></head><body><nav>navigation</nav><main><div class="chzzk_player"><video></video><div class="player_header"><div class="header_info"><div class="_control_1m3jr_148"><button id="chat">채팅</button><button id="donate">후원</button><button>미션</button><button>영도</button></div></div></div></div></main><aside id="aside-chatting"><input value="draft"></aside>
    <script>${report}const video=document.querySelector('video'),aside=document.querySelector('aside'),player=document.querySelector('.chzzk_player');Object.defineProperties(video,{readyState:{value:4},videoWidth:{value:1920},videoHeight:{value:1080},paused:{value:false}});
    ${script}
    setTimeout(()=>{document.documentElement.setAttribute('data-atsumi-controls','visible');report({asideHidden:getComputedStyle(aside).display==='none',navHidden:getComputedStyle(document.querySelector('nav')).display==='none',sameVideo:video===document.querySelector('video'),fullWidth:Math.abs(player.getBoundingClientRect().width-innerWidth)<1,chatAction:document.getElementById('chat').hasAttribute('data-atsumi-chat-toggle'),redundantHidden:[...document.querySelectorAll('[data-atsumi-redundant-action]')].length===3&&[...document.querySelectorAll('[data-atsumi-redundant-action]')].every(el=>getComputedStyle(el).display==='none')});},600);
    </script></body></html>`);
    expect(result).toMatchObject({ asideHidden: true, navHidden: true, sameVideo: true, fullWidth: true, chatAction: true, redundantHidden: true });
  }, 30000);
});
