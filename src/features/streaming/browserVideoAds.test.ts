import { describe, expect, it, vi } from 'vitest';
import source from '../../../src-tauri/src/streaming/browser_video_ads.js?raw';
const vmName = 'node:vm';
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
const ID = 'a'.repeat(32), OTHER = 'b'.repeat(32);
function fixture(options: { url?: string; frame?: boolean; props?: Record<string, unknown>; noFiber?: boolean; detached?: boolean; noRoot?: boolean; domFiber?: boolean } = {}) {
  const page = document.implementation.createHTMLDocument('ad fixture');
  const root = page.createElement('div'); root.id = 'preAdPlayerWrapper';
  const complete = vi.fn(), skip = vi.fn(), start = vi.fn();
  const props = { channelId: ID, liveId: 12345678, onComplete: complete, onSkip: skip, onAdStart: start, ...options.props };
  if (!options.noFiber) Object.assign(root, { __reactFiber$fixture: { type: 'div', memoizedProps: { id: root.id }, return: { type: options.domFiber ? 'div' : () => {}, memoizedProps: props } } });
  if (!options.noRoot && !options.detached) page.body.append(root);
  const listeners = new Map<string, (event: { persisted?: boolean }) => void>();
  const pageWindow = { location: new URL(options.url ?? `https://chzzk.naver.com/live/${ID}`), top: null as unknown, addEventListener: vi.fn((name, listener) => listeners.set(name, listener)), __atsumiVideoAdPolicy: undefined as undefined | { version: number; completed: number; unsupported: boolean } };
  pageWindow.top = options.frame ? {} : pageWindow;
  let mutation = () => {}; const observe = vi.fn(), disconnect = vi.fn();
  class Observer { constructor(callback: () => void) { mutation = callback; } observe = observe; disconnect = disconnect; }
  const context = { window: pageWindow, document: page, URLSearchParams, MutationObserver: Observer };
  runInNewContext(source, context);
  return { page, root, props, complete, skip, start, window: pageWindow, observe, disconnect, notify: () => mutation(), lifecycle: (name: string, persisted = false) => listeners.get(name)?.({ persisted }), reinstall: () => runInNewContext(source, context) };
}
describe('scoped official pre-roll adapter', () => {
  it('uses the official NO_ADS completion once, without skip/start/ad events or deleting media', () => {
    const f = fixture(); f.notify(); f.notify();
    expect(f.complete).toHaveBeenCalledExactlyOnceWith('NO_ADS');
    expect(f.skip).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
    expect(f.root.isConnected).toBe(true);
    expect(f.window.__atsumiVideoAdPolicy).toEqual({ version: 1, completed: 1, unsupported: false });
  });
  it.each([
    `https://nid.naver.com/nidlogin.login`, `https://chzzk.naver.com/`,
    `https://chzzk.naver.com/video/123`, `https://chzzk.naver.com/live/${ID}?chat=1`,
    `https://chzzk.naver.com/live/${ID}/chat`, `https://chzzk.naver.com.evil.test/live/${ID}`,
    `http://chzzk.naver.com/live/${ID}`, `https://chzzk.naver.com:444/live/${ID}`,
  ])('does nothing on account/chat/VOD/other documents: %s', url => {
    const f = fixture({url}); expect(f.complete).not.toHaveBeenCalled(); expect(f.observe).not.toHaveBeenCalled();
  });
  it('does not install in subframes', () => {
    const f = fixture({frame:true}); expect(f.complete).not.toHaveBeenCalled(); expect(f.observe).not.toHaveBeenCalled();
  });
  it.each([{channelId:OTHER},{liveId:''},{liveId:'123abc'},{onSkip:null},{onAdStart:null},{onComplete:'not a function'}])('fails open for a changed or mismatched component: %o', props => {
    const f = fixture({props}); expect(f.complete).not.toHaveBeenCalled(); expect(f.root.isConnected).toBe(true);
    expect(f.window.__atsumiVideoAdPolicy?.unsupported).toBe(true);
  });
  it('does not call arbitrary DOM properties or unrelated completion handlers', () => {
    for (const options of [{noFiber:true},{domFiber:true},{detached:true}]) { const f = fixture(options); expect(f.complete).not.toHaveBeenCalled(); }
  });
  it('handles a later channel using its own newly mounted pre-roll component', () => {
    const f = fixture(); f.root.remove(); f.window.location = new URL(`https://chzzk.naver.com/live/${OTHER}`);
    const next = f.root.cloneNode() as HTMLElement; const nextComplete = vi.fn();
    Object.assign(next, { __reactFiber$next: { type: () => {}, memoizedProps: { ...f.props, channelId: OTHER, onComplete: nextComplete } } });
    f.page.body.append(next); f.notify();
    expect(nextComplete).toHaveBeenCalledExactlyOnceWith('NO_ADS'); expect(f.window.__atsumiVideoAdPolicy?.completed).toBe(2);
  });
  it('does not repeat a throwing handler or suppress/rewrite the unknown player', () => {
    const complete = vi.fn(() => { throw new Error('changed component'); });
    const f = fixture({props:{onComplete:complete}}); f.notify();
    expect(complete).toHaveBeenCalledTimes(1); expect(f.root.style.display).toBe(''); expect(f.root.isConnected).toBe(true);
    expect(f.window.__atsumiVideoAdPolicy).toMatchObject({ completed:0, unsupported:true });
  });
  it('is idempotent and resumes observation after browser back/forward cache', () => {
    const f = fixture({noRoot:true}); f.reinstall(); expect(f.observe).toHaveBeenCalledTimes(1);
    f.lifecycle('pagehide'); expect(f.disconnect).toHaveBeenCalledTimes(1);
    f.page.body.append(f.root); f.lifecycle('pageshow',true); expect(f.observe).toHaveBeenCalledTimes(2); expect(f.complete).toHaveBeenCalledTimes(1);
  });
  it('does not patch browser network, JSON, cryptography or authentication APIs', () => {
    for (const pattern of [/JSON\.parse\s*=/,/prototype\./,/window\.fetch\s*=/,/\.decrypt\s*=/,/document\.cookie/,/localStorage/,/sessionStorage/]) expect(source).not.toMatch(pattern);
  });
});
