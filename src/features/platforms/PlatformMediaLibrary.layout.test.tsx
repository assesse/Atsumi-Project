import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { PlatformMediaLibrary } from './PlatformMediaLibrary';
import { groupPanelPosition, mediaGroupStorageKey } from './mediaGroupPaging';
import type { MediaLibraryItem, MediaRecordingGroup, PlatformMediaApi } from './platformMedia';
import css from './PlatformMediaLibrary.css?raw';
import baseCss from '../../styles.css?raw';
import inkCss from '../../atsumi-ink.css?raw';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

const fsName = 'node:fs', pathName = 'node:path', osName = 'node:os', childName = 'node:child_process', urlName = 'node:url';
const fs = await import(fsName) as { existsSync(path: string): boolean; mkdtempSync(prefix: string): string; writeFileSync(path: string, contents: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const os = await import(osName) as { tmpdir(): string };
const { pathToFileURL } = await import(urlName) as { pathToFileURL(path: string): { href: string } };
const { execFile } = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: 'utf8'; windowsHide: boolean }, callback: (error: Error | null, stdout: string, stderr: string) => void): void };
const processName = 'node:process';
const { env } = await import(processName) as { env: Record<string, string | undefined> };
const browser = [env.ATSUMI_TEST_BROWSER ?? '', 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', 'C:/Program Files/Microsoft/Edge/Application/msedge.exe'].find(file => fs.existsSync(file));

const item = (index: number, live = true): MediaLibraryItem => ({
  libraryId: `00000000-0000-4000-8000-${String(index).padStart(12, '0')}`, revision: 3,
  reference: { provider: 'youtube', kind: live ? 'live' : 'video', externalId: 'abcdefghijk' },
  broadcastId: live ? 'abcdefghijk' : undefined, title: `저장된 방송 ${index} ${'긴 방송 제목 '.repeat(10)}`,
  channelName: '채널 이름 '.repeat(10), bytes: 20_000_000, duration: 120, completedAt: 1000 + index, sha256: 'a'.repeat(64),
});

/** Capture production component markup, including its real portal, without an app/account. */
async function markup() {
  const storageKey = mediaGroupStorageKey('youtube');
  const stored = localStorage.getItem(storageKey);
  localStorage.removeItem(storageKey);
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  const container = document.createElement('div');
  document.body.append(container);
  const root = createRoot(container);
  const items = Array.from({ length: 12 }, (_, index) => item(index + 1));
  const group: MediaRecordingGroup = { type: 'recordingGroup', key: `g1_${'a'.repeat(64)}`, groupRevision: 'b'.repeat(64), representative: items[0]!, fileCount: items.length, totalBytes: 240_000_000, latestCompletedAt: 1012 };
  const api: PlatformMediaApi = {
    download: vi.fn(), jobs: vi.fn().mockResolvedValue([]), cancel: vi.fn(), retry: vi.fn(),
    library: vi.fn().mockResolvedValue({ items: [] }), thumbnail: vi.fn().mockResolvedValue(null), profile: vi.fn().mockResolvedValue(null),
    groups: vi.fn().mockResolvedValue({ status: 'ready', catalogRevision: 'c'.repeat(64), cursor: null, entries: [
      group, { type: 'video', key: `v1_${'d'.repeat(64)}`, item: item(20, false) },
      { type: 'video', key: `v1_${'e'.repeat(64)}`, item: item(21, false) },
    ] }),
    groupMembers: vi.fn().mockResolvedValue({ status: 'ready', group, items, cursor: null }),
    open: vi.fn(), close: vi.fn().mockResolvedValue(undefined), openFolder: vi.fn(), delete: vi.fn(),
  };
  try {
    await act(async () => root.render(<PlatformMediaLibrary provider="youtube" active privacyMode={false} api={api} />));
    const closed = container.innerHTML;
    const heading = container.querySelector<HTMLElement>('.platform-media-group-heading');
    expect(heading).not.toBeNull();
    await act(async () => heading!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    const panel = document.querySelector('.platform-media-group-panel');
    expect(panel).not.toBeNull();
    expect(panel!.querySelectorAll('article')).toHaveLength(12);
    return { closed, open: container.innerHTML, portal: panel!.outerHTML };
  } finally {
    await act(async () => root.unmount());
    container.remove();
    if (stored === null) localStorage.removeItem(storageKey); else localStorage.setItem(storageKey, stored);
    vi.unstubAllGlobals();
  }
}

type Box = { x: number; y: number; width: number; height: number };
type Scene = { name: string; viewport: { width: number; height: number }; before: Box[]; after: Box[]; panel: Box; toolbar: Box; scroller: Box; last: Box; scrollTop: number; position: string; overflow: string; scrollHeight: number; clientHeight: number; reduced: boolean; animation: string };

it.skipIf(!browser).each([
  [640, 640, false], [960, 800, false], [640, 640, true],
] as const)('keeps the recording portal bounded without moving cards at %ix%i (reduced motion %s)', async (width, height, reduced) => {
  const rendered = await markup();
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'atsumi-library-groups-'));
  // Use the same pure placement function on actual browser rectangles, not jsdom's zero rectangles.
  const script = `(() => {
    const position = ${groupPanelPosition.toString()}, scenes = [], host = document.getElementById('fixture');
    const box = node => { const r = node.getBoundingClientRect(); return {x:r.x,y:r.y,width:r.width,height:r.height}; };
    const cards = () => [...host.querySelector('.platform-media-grid').children].map(box);
    for (const narrow of [true, false]) for (const bottom of [false, true]) {
      host.style.width = (narrow ? 280 : window.innerWidth - 32) + 'px';
      host.style.top = (bottom ? window.innerHeight - 180 : 12) + 'px';
      host.innerHTML = ${JSON.stringify(rendered.closed)};
      const before = cards();
      host.innerHTML = ${JSON.stringify(rendered.open)};
      const mount = document.createElement('div'); mount.innerHTML = ${JSON.stringify(rendered.portal)};
      const panel = mount.firstElementChild; document.body.append(panel);
      const heading = host.querySelector('.platform-media-group-heading');
      const placed = position(heading.getBoundingClientRect(), window.innerWidth, window.innerHeight);
      for (const [key, value] of Object.entries(placed)) panel.style[key] = value + 'px';
      panel.getAnimations().forEach(animation => animation.finish());
      const files = panel.querySelector('.platform-media-group-panel-scroll');
      files.scrollTop = files.scrollHeight;
      scenes.push({name:(narrow?'narrow':'wide')+'/'+(bottom?'bottom':'top'), viewport:{width:innerWidth,height:innerHeight},
        before, after:cards(), panel:box(panel), toolbar:box(panel.querySelector('.platform-media-group-panel-toolbar')),
        scroller:box(files),last:box(files.querySelector('article:last-child')),scrollTop:files.scrollTop,
        position:getComputedStyle(panel).position, overflow:getComputedStyle(files).overflowY,
        scrollHeight:files.scrollHeight, clientHeight:files.clientHeight,
        reduced:matchMedia('(prefers-reduced-motion: reduce)').matches, animation:getComputedStyle(panel).animationName});
      panel.remove();
    }
    document.getElementById('metrics').textContent = JSON.stringify(scenes);
  })()`;
  try {
    const input = path.join(directory, 'fixture.html');
    fs.writeFileSync(input, `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none';img-src data:;style-src 'unsafe-inline';script-src 'unsafe-inline'"><style>${baseCss}\n${inkCss}\n${css}\nbody{margin:0;height:100vh;overflow:hidden}#fixture{position:absolute;left:16px;height:600px;box-sizing:border-box}</style><main id="fixture"></main><pre id="metrics"></pre><script>${script}</script>`);
    const output = await new Promise<{ stdout: string; stderr: string }>((resolve, reject) => execFile(browser!, [
      '--headless', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-component-update', '--disable-sync',
      '--host-resolver-rules=MAP * ~NOTFOUND', `--user-data-dir=${path.join(directory, 'profile')}`, `--window-size=${width},${height}`,
      ...(reduced ? ['--force-prefers-reduced-motion=reduce'] : []), '--virtual-time-budget=1500', '--dump-dom', pathToFileURL(input).href,
    ], { timeout: 45000, maxBuffer: 4 * 1024 ** 2, encoding: 'utf8', windowsHide: true }, (error, stdout, stderr) => error ? reject(new Error(stderr || error.message)) : resolve({ stdout, stderr })));
    const json = /<pre id="metrics">([^<]+)<\/pre>/.exec(output.stdout)?.[1];
    expect(json, `Headless fixture did not report geometry. stdout=${output.stdout.slice(0, 600)} stderr=${output.stderr.slice(0, 1200)}`).toBeTruthy();
    const results = JSON.parse(json!) as Scene[];
    expect(results).toHaveLength(4);
    for (const scene of results) {
      expect(scene.after, scene.name).toEqual(scene.before);
      expect(scene.position).toBe('fixed');
      expect(scene.panel.x, scene.name).toBeGreaterThanOrEqual(12);
      expect(scene.panel.y, scene.name).toBeGreaterThanOrEqual(12);
      expect(scene.panel.x + scene.panel.width, scene.name).toBeLessThanOrEqual(scene.viewport.width - 11);
      expect(scene.panel.y + scene.panel.height, scene.name).toBeLessThanOrEqual(scene.viewport.height - 11);
      expect(scene.toolbar.height, scene.name).toBeGreaterThan(20);
      expect(scene.toolbar.y + scene.toolbar.height, scene.name).toBeLessThanOrEqual(scene.panel.y + scene.panel.height);
      expect(scene.overflow, scene.name).toMatch(/auto|scroll/);
      expect(scene.clientHeight, scene.name).toBeGreaterThan(30);
      expect(scene.scrollHeight, scene.name).toBeGreaterThan(scene.clientHeight);
      expect(scene.scrollTop, scene.name).toBeGreaterThan(0);
      expect(scene.last.y + scene.last.height, scene.name).toBeLessThanOrEqual(scene.scroller.y + scene.scroller.height + 1);
      expect(scene.reduced).toBe(reduced);
      if (reduced) expect(scene.animation).toBe('none');
    }
  } finally {
    const relative = path.relative(fs.realpathSync(os.tmpdir()), fs.realpathSync(directory));
    if (!relative.startsWith('atsumi-library-groups-') || relative.includes('..') || path.isAbsolute(relative)) throw new Error('Unexpected disposable fixture path');
    // Fail visibly if browser handles outlive the disposable fixture; never kill a user's browser.
    fs.rmSync(directory, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
}, 60000);
