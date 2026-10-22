// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';
import original from '../../../src-tauri/src/soop/original_vod_page.js?raw';
import observer from '../../../src-tauri/src/soop/original_vod_click_probe.js?raw';
const vmName = 'node:vm';
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
const anchor = "  document.addEventListener('pointerdown', event => {";
const prefix = 'ATSUMI_SOOP_VOD_CLICK_PROBE:';
type Handler = (event: Input) => void;
type Input = { type: string; button: number; isTrusted: boolean; target: { closest(selector: string): unknown }; preventDefault: ReturnType<typeof vi.fn>; stopImmediatePropagation: ReturnType<typeof vi.fn>; stopped: boolean };
type Adapter = { configure(value: unknown): boolean; fullscreen(enabled: boolean): void; status(): { state: string } };
function fixture(probe = true, failure?: 'registration' | 'post' | 'remove') {
  const listeners = new Map<string, Handler[]>(), windowListeners = new Map<string, (() => void)[]>();
  const rows: Record<string, unknown>[] = [], commands: Record<string, unknown>[] = [], order: string[] = [];
  const video = { readyState: 4, videoWidth: 1280, paused: false, ended: false };
  let failedRegistration = false;
  const document = {
    documentElement: {}, querySelector: (selector: string) => selector === '#videoLayer video#video.af_video' ? video : null,
    addEventListener(name: string, handler: Handler, options: unknown) {
      if (failure === 'registration' && typeof options === 'object' && !failedRegistration) { failedRegistration = true; throw Error('probe registration'); }
      listeners.set(name, [...listeners.get(name) ?? [], handler]);
    },
    removeEventListener(name: string, handler: Handler) {
      if (failure === 'remove') throw Error('probe removal');
      listeners.set(name, (listeners.get(name) ?? []).filter(value => value !== handler));
    },
    get cookie(): never { throw Error('secret access'); },
  };
  const window = {
    top: null as unknown, chrome: { webview: { postMessage(value: string) {
      if (value.startsWith(prefix)) {
        if (failure === 'post') throw Error('probe post');
        rows.push(JSON.parse(value.slice(prefix.length))); order.push('observation');
      } else {
        const command = JSON.parse(value.slice('ATSUMI_SOOP_VOD_V1:'.length)); commands.push(command);
        if (command.type === 'fullscreen') order.push('fullscreen');
      }
    } } }, __atsumiSoopVod: undefined as Adapter | undefined,
    addEventListener(name: string, handler: () => void) { windowListeners.set(name, [...windowListeners.get(name) ?? [], handler]); },
    removeEventListener(name: string, handler: () => void) { windowListeners.set(name, (windowListeners.get(name) ?? []).filter(value => value !== handler)); },
  };
  window.top = window;
  runInNewContext(probe ? original.replace(anchor, `${observer}\n${anchor}`) : original, {
    window, document, location: new URL('https://vod.sooplive.com/player/209244227/embed?showChat=true&replayType=1'),
    URL, TextEncoder, crypto: { randomUUID: () => '00000000-0000-4000-8000-000000000001' },
    setInterval: () => 1, clearInterval: vi.fn(),
  });
  const dispatch = (kind = 'pointerdown', target: 'button' | 'player' | 'chat' | 'editable' | 'outside' = 'button', patch: Partial<Input> = {}) => {
    const event: Input = { type: kind, button: 0, isTrusted: true, stopped: false,
      target: { closest(selector) {
        if (target === 'chat' && selector.includes('#chatting_area')) return {};
        if (target === 'editable' && selector.includes('input')) return {};
        if (target !== 'outside' && selector === '#player') return {};
        if (target === 'button' && selector === '.btn_fullScreen_mode,#btnExtendMode') return {};
        return null;
      } }, preventDefault: vi.fn(), stopImmediatePropagation: vi.fn(), ...patch };
    event.stopImmediatePropagation = vi.fn(() => { event.stopped = true; });
    for (const handler of [...listeners.get(kind) ?? []]) { handler(event); if (event.stopped) break; }
    return event;
  };
  const configure = (fullscreen = false) => { window.__atsumiSoopVod!.configure({ videoId: '209244227', epoch: 2, fullscreen }); commands.length = 0; order.length = 0; };
  return { rows, commands, order, listeners, dispatch, configure, video, adapter: window.__atsumiSoopVod!, pagehide: () => { for (const handler of [...windowListeners.get('pagehide') ?? []]) handler(); } };
}
describe('isolated SOOP click observation', () => {
  it('inserts before the one original handler and leaves all original bytes intact', () => {
    expect(original.split(anchor)).toHaveLength(2);
    const patched = original.replace(anchor, `${observer}\n${anchor}`);
    expect(patched.replace(`${observer}\n`, '')).toBe(original);
    for (const forbidden of ['preventDefault', 'stopPropagation', 'stopImmediatePropagation', 'focus(', 'capture(', 'getBoundingClientRect', 'location', 'cookie', 'localStorage', 'setTimeout', 'setInterval', 'event.key', 'clientX', 'innerText', 'outerHTML']) expect(observer).not.toContain(forbidden);
  });
  it('observes a real pointer before the unchanged request without consuming any additional input', () => {
    const probe = fixture(), control = fixture(false); probe.configure(); control.configure();
    for (const kind of ['pointerdown', 'click']) {
      const actual = probe.dispatch(kind), baseline = control.dispatch(kind);
      expect(actual.preventDefault.mock.calls).toEqual(baseline.preventDefault.mock.calls);
      expect(actual.stopImmediatePropagation.mock.calls).toEqual(baseline.stopImmediatePropagation.mock.calls);
    }
    expect(probe.commands).toEqual(control.commands);
    expect(probe.order).toEqual(['observation', 'fullscreen', 'observation']);
    expect(probe.rows[0]).toEqual({ kind: 'pointerdown', target: 'fullscreen_button', trusted: true, left: true, configured: true, fullscreen: false });
  });
  it('keeps click-only distinct from an ordinary pointer press', () => {
    const f = fixture(); f.configure(); const event = f.dispatch('click');
    expect(f.rows[0]?.kind).toBe('click'); expect(f.commands).toHaveLength(0);
    expect(event.preventDefault).toHaveBeenCalledOnce(); expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
  });
  it('projects only categories and booleans for player, chat, editable and outside targets', () => {
    const f = fixture(); f.configure();
    for (const target of ['player', 'chat', 'editable', 'outside'] as const) f.dispatch('pointerdown', target);
    expect(f.rows.map(row => row.target)).toEqual(['player_other', 'chat', 'excluded', 'excluded']);
    expect(f.commands).toHaveLength(0);
    for (const row of f.rows) expect(Object.keys(row).sort()).toEqual(['configured', 'fullscreen', 'kind', 'left', 'target', 'trusted']);
  });
  it('reports untrusted, right-button and unconfigured input without authorizing it', () => {
    const f = fixture(); f.dispatch('pointerdown', 'button', { isTrusted: false }); f.dispatch('pointerdown', 'button', { button: 2 }); f.dispatch();
    expect(f.rows.map(row => [row.trusted, row.left, row.configured])).toEqual([[false, true, false], [true, false, false], [true, true, false]]);
    expect(f.commands).toHaveLength(0);
  });
  it('follows native acknowledgements through repeated entry and exit without replacing listeners', () => {
    const f = fixture(); f.configure();
    for (const fullscreen of [false, true, false, true]) { f.adapter.fullscreen(fullscreen); f.dispatch(); }
    expect(f.rows.map(row => row.fullscreen)).toEqual([false, true, false, true]);
    expect(f.commands.map(command => command.enabled)).toEqual([true, false, true, false]);
    expect(f.listeners.get('pointerdown')).toHaveLength(2);
    f.video.ended = true; expect(f.adapter.status().state).toBe('ended'); f.adapter.fullscreen(false); f.dispatch();
    expect(f.rows.at(-1)?.fullscreen).toBe(false); expect(f.commands.at(-1)?.enabled).toBe(true);
  });
  it('detaches after sixteen events and on pagehide without removing original handlers', () => {
    const f = fixture(); f.configure(); for (let i = 0; i < 25; i++) f.dispatch();
    expect(f.rows).toHaveLength(16); expect(f.commands).toHaveLength(25);
    expect(f.listeners.get('pointerdown')).toHaveLength(1); expect(f.listeners.get('click')).toHaveLength(1);
    const hidden = fixture(); hidden.configure(); hidden.pagehide(); hidden.dispatch();
    expect(hidden.rows).toHaveLength(0); expect(hidden.commands).toHaveLength(1);
  });
  it('contains registration, sending and removal exceptions without disrupting original commands', () => {
    for (const failure of ['registration', 'post', 'remove'] as const) {
      const f = fixture(true, failure); f.configure(); for (let i = 0; i < 20; i++) f.dispatch();
      expect(f.commands).toHaveLength(20); expect(f.rows.length).toBeLessThanOrEqual(16);
    }
  });
  it('contains observational selector failure while the original handler can still run', () => {
    const f = fixture(); f.configure(); let first = true;
    f.dispatch('pointerdown', 'button', { target: { closest(selector) {
      if (first) { first = false; throw Error('observer-only failure'); }
      return selector === '#player' || selector === '.btn_fullScreen_mode,#btnExtendMode' ? {} : null;
    } } });
    expect(f.rows).toHaveLength(0); expect(f.commands).toHaveLength(1);
  });
});
