import { afterEach, describe, expect, it, vi } from 'vitest';
import source from '../../../src-tauri/src/soop/original_appearance.js?raw';
import page from '../../../src-tauri/src/soop/original_page.js?raw';
const vmName = 'node:vm';
const { runInNewContext } = await import(vmName) as { runInNewContext(text: string, context: Record<string, unknown>): unknown };
const keys = ['theme', 'fontLevel', 'randomNicknameColor', 'fanMessageColor', 'showEmoticons', 'animateEmoticons', 'smallOgq'] as const;
type Key = typeof keys[number];
const roles = ['normal', 'streamer', 'manager', 'vip', 'subscribe', 'fan', 'support', 'nightbot'] as const;
type Palette = { version: 1; basis: 'computed-css-variable'; nickname: Array<string | null>; roles: Record<typeof roles[number], string | null> };
type Observation = { sequence: number; values: Record<Key, unknown>; basis: Record<Key, unknown>; palette: Palette | null; surface: { version: 1; basis: 'computed-dom'; foreground: string | null; background: string | null } | null; actualChannel: string; actualBroadcast: string };
type Observer = { configure(epoch: number): boolean; sample(ready: boolean): boolean; dispose(): void };
type Factory = { create(options: { binding(): unknown; emit(value: Observation, epoch: number): boolean }): Observer };
type Adapter = { configure(value: unknown): boolean; start(id: string): Promise<unknown>; relayStart(id: string, epoch: number): Promise<unknown> };
const disposals: Array<() => void> = [];
const drain = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
function fixture(options: { url?: string; nested?: boolean; integrated?: boolean; raw?: string | null; colors?: Record<string, string> } = {}) {
  document.documentElement.setAttribute('dark', 'false');
  document.body.className = '';
  document.body.innerHTML = '<div id="chatting_area" font-level="3"><div id="chatbox"><div id="chat_area" class="chatting-viewer random-color-on"></div><div class="chat_layer sub mark"><ul><li><input type="checkbox" id="st1_emoticon"></li></ul></div></div></div><video id="livePlayer"></video>';
  let raw = options.raw ?? null, storageThrows = false, actual = { actualChannel: 'fixture', actualBroadcast: '12345' }, allowEmit = true;
  const colors = { ...options.colors }; let styleThrows = false, propertyThrows: string | null = null;
  const getPropertyValue = vi.fn((key: string) => { if (key === propertyThrows) throw Error('Missing property'); return colors[key] ?? ''; });
  const computedStyle = vi.fn((node: Element) => { if (styleThrows) throw Error('Style unavailable'); if (node !== document.getElementById('chat_area')) throw Error('Unrelated node'); return { getPropertyValue }; });
  let frameId = 0, heartbeat: () => void = () => {};
  const frames = new Map<number, FrameRequestCallback>(), observations: Array<{ epoch: number; value: Observation }> = [], native: Array<Record<string, unknown>> = [], trace: string[] = [];
  const storage = { getItem: vi.fn((key: string) => { if (storageThrows) throw Error('Unavailable'); if (key !== 'chatOptionSetting') throw Error('Unrelated storage'); return raw; }) };
  const requestAnimationFrame = vi.fn((callback: FrameRequestCallback) => { frames.set(++frameId, callback); return frameId; });
  const cancelAnimationFrame = vi.fn((id: number) => { frames.delete(id); });
  const windowEvents = new Map<string, () => void>();
  const capture = { start: vi.fn().mockImplementation(() => { trace.push('encoded_start'); return Promise.resolve({}); }), startRelay: vi.fn().mockImplementation(() => { trace.push('relay_start'); return Promise.resolve({}); }), getStatus: () => ({ ready: true }) };
  const win = {
    top: null as unknown, MutationObserver, requestAnimationFrame, cancelAnimationFrame, localStorage: storage, getComputedStyle: computedStyle,
    livePlayer: { liveAPI: { getResponse: () => ({ CHANNEL: { BJID: actual.actualChannel, BNO: actual.actualBroadcast } }) } },
    liveView: { isLoaded: () => Promise.resolve() },
    chrome: { webview: { postMessage: (text: string) => { const value = JSON.parse(text.slice('ATSUMI_SOOP_V1:'.length)) as Record<string, unknown>; native.push(value); trace.push(String(value.type)); } } },
    postMessage: vi.fn(), addEventListener: (type: string, callback: () => void) => windowEvents.set(type, callback),
    __atsumiSoopEncodedCapture: capture,
    __atsumiSoopAppearanceObserver: undefined as Factory | undefined,
    __atsumiSoopOriginal: undefined as Adapter | undefined,
  };
  win.top = options.nested ? {} : win;
  const setInterval = vi.fn((callback: () => void) => { heartbeat = callback; return 1; });
  const context = { window: win, document, location: new URL(options.url ?? 'https://play.sooplive.com/fixture/12345/embed?fromApi=1'), crypto, MutationObserver, setInterval, clearInterval: vi.fn(), setTimeout, clearTimeout, requestAnimationFrame, cancelAnimationFrame };
  runInNewContext(source, context);
  let observer: Observer | undefined;
  if (options.integrated) {
    runInNewContext(page, context);
    disposals.push(() => windowEvents.get('pagehide')?.());
  } else if (win.__atsumiSoopAppearanceObserver) {
    observer = win.__atsumiSoopAppearanceObserver.create({ binding: () => actual, emit: (value, epoch) => {
      if (!allowEmit) return false;
      observations.push({ epoch, value: JSON.parse(JSON.stringify(value)) as Observation }); return true;
    } });
    disposals.push(() => observer!.dispose());
  }
  return { win, observer, observations, native, storage, frames, trace, capture, setInterval, context, windowEvents,
    computedStyle, getPropertyValue, setColor: (key: string, value: string) => { colors[key] = value; },
    setStyleThrows: (value: boolean) => { styleThrows = value; }, setPropertyThrows: (key: string) => { propertyThrows = key; },
    latest: () => observations.at(-1)!.value,
    setRaw: (value: string | null) => { raw = value; }, setStorageThrows: (value: boolean) => { storageThrows = value; },
    bind: (value: typeof actual) => { actual = value; }, allowEmit: (value: boolean) => { allowEmit = value; },
    heartbeat: () => heartbeat(), flushFrame: () => { const callbacks = [...frames.values()]; frames.clear(); for (const callback of callbacks) callback(0); } };
}
afterEach(() => { for (const dispose of disposals.splice(0)) dispose(); document.body.innerHTML = ''; vi.restoreAllMocks(); });

function surfaceFixture(options: Parameters<typeof fixture>[0] = {}) {
  const f = fixture(options);
  const styles = new Map<Element, Record<string, string>>();
  const viewer = document.getElementById('chat_area')!;
  const visible = { width: 300, height: 20, x: 0, y: 0, top: 0, left: 0, right: 300, bottom: 20, toJSON() { return {}; } };
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(visible);
  f.computedStyle.mockImplementation(node => ({ getPropertyValue: f.getPropertyValue, display: 'block', visibility: 'visible', opacity: '1',
    filter: 'none', backdropFilter: 'none', mixBlendMode: 'normal', backgroundBlendMode: 'normal', backgroundImage: 'none',
    backgroundColor: node === document.getElementById('chatting_area') ? 'rgb(12, 13, 14)' : 'rgba(0, 0, 0, 0)', color: 'rgb(226, 228, 233)', ...styles.get(node) }));
  const row = (role = 'normal') => {
    const element = document.createElement('div'); element.className = 'chatting-list-item'; element.setAttribute('user-type', role);
    element.innerHTML = '<div class="message-container"><div class="message-text"><p class="msg">fixture text never emitted</p><div class="message-translation"><p class="msg">translation</p></div></div></div>';
    viewer.append(element); return element;
  };
  return { ...f, viewer, styles, row };
}

describe('SOOP observed ordinary surface', () => {
  it('keeps empty foreground unknown then records only a direct ordinary body at a later sequence', () => {
    const f = surfaceFixture(); f.observer!.configure(1); expect(f.latest().surface).toBeNull(); f.observer!.sample(true);
    expect(f.latest().surface).toEqual({ version: 1, basis: 'computed-dom', foreground: null, background: '#0c0d0e' });
    const previous = f.latest().sequence, row = f.row();
    f.styles.set(row.querySelector('.message-translation .msg')!, { color: 'rgb(68, 163, 255)' });
    f.observer!.sample(true); expect(f.latest().sequence).toBeGreaterThan(previous);
    expect(f.latest().surface?.foreground).toBe('#e2e4e9'); expect(JSON.stringify(f.observations)).not.toContain('fixture text');
    row.remove(); f.observer!.sample(true); expect(f.latest().surface?.foreground).toBeNull();
    f.observer!.configure(2); expect(f.latest().surface).toBeNull();
  });
  it('excludes fan inline colors, staff bubbles, gifts and settings preview', () => {
    const f = surfaceFixture(); f.row('fan'); const inline = f.row(); inline.querySelector<HTMLElement>('.msg')!.style.color = '#ff0000';
    f.viewer.insertAdjacentHTML('beforeend', '<div class="chatting-list-item"><div class="bubble-container notice"><p class="msg">staff</p></div></div><div class="chatting-list-item"><div class="donation-container"><p class="msg">gift</p></div></div>');
    document.getElementById('chatbox')!.insertAdjacentHTML('beforeend', '<div class="preview"><div class="chatting-list-item"><div class="message-container"><div class="message-text"><p class="msg">preview</p></div></div></div></div>');
    f.observer!.configure(1); f.observer!.sample(true); expect(f.latest().surface?.foreground).toBeNull();
    const ordinary = f.row(); f.styles.set(ordinary.querySelector('.msg')!, { color: '#123456' });
    f.observer!.sample(true); expect(f.latest().surface?.foreground).toBe('#123456');
    f.row(); f.observer!.sample(true); expect(f.latest().surface?.foreground).toBeNull();
  });
  it.each([
    ['backgroundColor', 'rgba(1, 2, 3, 0.5)'], ['backgroundImage', 'linear-gradient(red,blue)'], ['backgroundBlendMode', 'multiply'],
  ])('keeps complex background unknown without discarding independent foreground (%s)', (key, value) => {
    const f = surfaceFixture(); f.row(); f.styles.set(f.viewer, { [key]: value });
    f.observer!.configure(1); f.observer!.sample(true); expect(f.latest().surface).toMatchObject({ foreground: '#e2e4e9', background: null });
  });
  it.each([['opacity', '0.5'], ['filter', 'blur(1px)'], ['mixBlendMode', 'multiply'], ['backdropFilter', 'blur(1px)'], ['visibility', 'hidden']])('rejects ancestor effects above the opaque background (%s)', (key, value) => {
    const f = surfaceFixture(); f.row(); f.styles.set(document.body, { [key]: value });
    f.observer!.configure(1); f.observer!.sample(true); expect(f.latest().surface).toMatchObject({ foreground: null, background: null });
  });
  it('bounds row scans and refuses an unresolved ancestor chain', () => {
    const f = surfaceFixture(); f.row(); for (let i = 0; i < 32; i++) f.viewer.append(document.createElement('div'));
    f.observer!.configure(1); f.observer!.sample(true); expect(f.latest().surface?.foreground).toBeNull();
    for (let i = 0; i < 12; i++) { const wrap = document.createElement('div'); f.viewer.before(wrap); wrap.append(f.viewer); }
    f.observer!.sample(true); expect(f.latest().surface).toMatchObject({ foreground: null, background: null });
  });
  it('uses the existing heartbeat for surface-only changes and never retains a former source value', async () => {
    const f = surfaceFixture({ integrated: true }); f.row(); f.win.__atsumiSoopOriginal!.configure({ epoch: 1, channel: 'fixture' }); await drain();
    const before = f.native.filter(value => value.type === 'appearance').length;
    f.styles.set(document.getElementById('chatting_area')!, { backgroundColor: '#123456' }); f.heartbeat();
    expect(f.native.filter(value => value.type === 'appearance')).toHaveLength(before + 1);
    expect(f.native.at(-1)).toMatchObject({ surface: { background: '#123456' } });
    f.heartbeat(); expect(f.native.filter(value => value.type === 'appearance')).toHaveLength(before + 1);
    f.win.__atsumiSoopOriginal!.configure({ epoch: 2, channel: 'fixture' });
    expect(f.native.find(value => value.type === 'appearance' && value.epoch === 2)).toMatchObject({ surface: null });
    expect(f.setInterval).toHaveBeenCalledTimes(1);
  });
});

describe('SOOP observed presentation', () => {
  it.each(['http://play.sooplive.com/fixture/12345/embed', 'https://other.test/fixture/12345/embed', 'https://play.sooplive.com/fixture/embed'])('does not install on an unbound route %s', url => {
    expect(fixture({ url }).win.__atsumiSoopAppearanceObserver).toBeUndefined();
  });
  it('does not install into subframes', () => expect(fixture({ nested: true }).win.__atsumiSoopAppearanceObserver).toBeUndefined());

  it('starts unknown and reads explicit booleans without believing unchecked template controls', () => {
    const f = fixture({ raw: JSON.stringify({ st1_emoticon: true, st1_emoticon_ani: false, st1_ogq: true, st1_fanchatcolor: true, accountSecret: 'never emitted', st1_randomnickname: false }) });
    f.observer!.configure(1); expect(Object.values(f.latest().values)).toEqual(keys.map(() => null)); expect(f.storage.getItem).not.toHaveBeenCalled();
    const checkbox = document.getElementById('st1_emoticon') as HTMLInputElement; expect(checkbox.checked).toBe(false);
    f.observer!.sample(true);
    expect(f.latest().values).toEqual({ theme: 'light', fontLevel: 3, randomNicknameColor: true, fanMessageColor: true, showEmoticons: true, animateEmoticons: false, smallOgq: true });
    expect(f.latest().basis).toEqual({ theme: 'dom', fontLevel: 'dom', randomNicknameColor: 'dom', fanMessageColor: 'saved-option', showEmoticons: 'saved-option', animateEmoticons: 'saved-option', smallOgq: 'saved-option' });
    expect(checkbox.checked).toBe(false); expect(JSON.stringify(f.observations)).not.toContain('never emitted');
    expect(new Set(f.storage.getItem.mock.calls.map(call => call[0]))).toEqual(new Set(['chatOptionSetting']));
  });
  it.each([null, '{', '[]', 'null', 'true', JSON.stringify({ st1_emoticon: 1, st1_ogq: 'false' }), ' '.repeat(4097)])('keeps malformed/missing preferences unknown (%s)', raw => {
    const f = fixture({ raw }); f.observer!.configure(1); f.observer!.sample(true);
    for (const name of ['fanMessageColor', 'showEmoticons', 'animateEmoticons', 'smallOgq'] as const) { expect(f.latest().values[name]).toBeNull(); expect(f.latest().basis[name]).toBeNull(); }
  });
  it('keeps storage access failure unknown without losing independent DOM observations', () => {
    const f = fixture(); f.setStorageThrows(true); f.observer!.configure(1); f.observer!.sample(true);
    expect(f.latest().values.theme).toBe('light'); expect(f.latest().values.showEmoticons).toBeNull();
  });
  it('uses actual chat ancestors and exact font bounds instead of replay defaults or a template class', () => {
    const f = fixture(); f.observer!.configure(1);
    document.body.classList.add('thema_dark'); document.getElementById('chatbox')!.classList.add('random-color-on');
    document.getElementById('chat_area')!.classList.remove('random-color-on'); document.getElementById('chatting_area')!.setAttribute('font-level', '9');
    f.observer!.sample(true); expect(f.latest().values).toMatchObject({ theme: 'dark', fontLevel: null, randomNicknameColor: false });
    document.body.classList.remove('thema_dark'); document.documentElement.removeAttribute('dark');
    document.getElementById('chatting_area')!.setAttribute('font-level', '1'); f.observer!.sample(true);
    expect(f.latest().values).toMatchObject({ theme: null, fontLevel: 1 });
    document.getElementById('chatbox')!.classList.add('chat_thema_dark'); f.observer!.sample(true); expect(f.latest().values.theme).toBe('dark');
  });
  it('samples after the original click handler without changing checked values or adding a timer', () => {
    const f = fixture({ raw: '{"st1_emoticon":true}' }); f.observer!.configure(1); f.observer!.sample(true);
    const checkbox = document.getElementById('st1_emoticon') as HTMLInputElement;
    checkbox.parentElement!.addEventListener('click', event => { event.preventDefault(); event.stopPropagation(); f.setRaw('{"st1_emoticon":false}'); });
    checkbox.parentElement!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(f.latest().values.showEmoticons).toBe(true); expect(f.frames.size).toBe(1); f.flushFrame();
    expect(f.latest().values.showEmoticons).toBe(false); expect(checkbox.checked).toBe(false); expect(f.setInterval).not.toHaveBeenCalled();
  });
  it('coalesces relevant attribute mutations and ignores unchanged snapshots', async () => {
    const f = fixture(); f.observer!.configure(1); f.observer!.sample(true); const before = f.observations.length;
    document.documentElement.setAttribute('dark', 'true'); document.getElementById('chatting_area')!.setAttribute('font-level', '4');
    await drain(); expect(f.frames.size).toBe(1); f.flushFrame(); expect(f.observations).toHaveLength(before + 1);
    expect(f.latest().values).toMatchObject({ theme: 'dark', fontLevel: 4 }); f.observer!.sample(true); expect(f.observations).toHaveLength(before + 1);
  });
  it('repairs replaced chat nodes and quiet property changes through the existing sample call', async () => {
    const f = fixture({ raw: '{"st1_ogq":false}' }); f.observer!.configure(1); f.observer!.sample(true);
    const old = document.getElementById('chat_area')!; const replacement = document.createElement('div'); replacement.id = 'chat_area'; old.replaceWith(replacement);
    f.setRaw('{"st1_ogq":true}'); f.observer!.sample(true); expect(f.latest().values).toMatchObject({ randomNicknameColor: false, smallOgq: true });
    replacement.classList.add('random-color-on'); await drain(); f.flushFrame(); expect(f.latest().values.randomNicknameColor).toBe(true);
  });
  it('preserves same-epoch sequence, resets on a new epoch and fences an already queued old callback', async () => {
    const f = fixture(); f.observer!.configure(1); f.observer!.sample(true); const before = f.latest().sequence;
    f.observer!.configure(1); f.observer!.sample(true); expect(f.latest().sequence).toBe(before);
    document.documentElement.setAttribute('dark', 'true'); await drain(); const stale = [...f.frames.values()][0]!;
    f.observer!.configure(2); expect(f.observations.at(-1)!.epoch).toBe(2); expect(f.latest().sequence).toBe(1); expect(Object.values(f.latest().values)).toEqual(keys.map(() => null));
    stale(0); expect(f.latest().sequence).toBe(1); f.observer!.sample(true); expect(f.latest().sequence).toBe(2); expect(f.latest().values.theme).toBe('dark');
  });
  it('rejects a foreign source and resends an identical snapshot after the bound source returns', () => {
    const f = fixture(); f.observer!.configure(1); f.observer!.sample(true); const before = f.observations.length;
    f.bind({ actualChannel: 'fixture', actualBroadcast: '999' }); expect(f.observer!.sample(true)).toBe(false); expect(f.observations).toHaveLength(before);
    f.bind({ actualChannel: 'fixture', actualBroadcast: '12345' }); expect(f.observer!.sample(true)).toBe(true); expect(f.observations).toHaveLength(before + 1);
  });
  it('does not suppress a failed native post and never reuses its sequence', () => {
    const f = fixture(); f.allowEmit(false); f.observer!.configure(1); f.observer!.sample(true); expect(f.observations).toHaveLength(0);
    f.allowEmit(true); f.observer!.sample(true); expect(f.latest().sequence).toBe(3);
  });
  it('disconnects on disposal and cannot resurrect from late callbacks', async () => {
    const f = fixture(); f.observer!.configure(1); f.observer!.sample(true); document.documentElement.setAttribute('dark', 'true'); await drain();
    const late = [...f.frames.values()][0]!; const count = f.observations.length; f.observer!.dispose(); late(0);
    expect(f.frames.size).toBe(0); expect(f.observer!.configure(2)).toBe(false); expect(f.observer!.sample(true)).toBe(false); expect(f.observations).toHaveLength(count);
  });
  it('connects the page lifecycle, existing heartbeat and pre-begin snapshots without another polling loop', async () => {
    const f = fixture({ integrated: true, raw: '{"st1_ogq":false}' }); const adapter = f.win.__atsumiSoopOriginal!;
    adapter.configure({ epoch: 1, channel: 'fixture' }); await drain();
    expect(f.setInterval).toHaveBeenCalledTimes(1); expect(f.native.filter(row => row.type === 'appearance')).toHaveLength(2);
    f.trace.length = 0; f.setRaw('{"st1_ogq":true}'); await adapter.start('record'); expect(f.trace.slice(0, 2)).toEqual(['appearance', 'encoded_start']);
    f.trace.length = 0; f.setRaw('{"st1_ogq":false}'); await adapter.relayStart('relay', 1); expect(f.trace.slice(0, 2)).toEqual(['appearance', 'relay_start']);
    f.setRaw('{"st1_ogq":true}'); f.heartbeat(); expect(f.native.at(-1)).toMatchObject({ type: 'appearance', values: { smallOgq: true } });
    adapter.configure({ epoch: 2, channel: 'fixture' }); expect(f.native.at(-1)).toMatchObject({ type: 'appearance', epoch: 2, sequence: 2, values: { smallOgq: true } });
    f.windowEvents.get('pagehide')!(); const count = f.native.length; document.documentElement.setAttribute('dark', 'true'); await drain(); f.flushFrame(); expect(f.native).toHaveLength(count);
  });
});

describe('SOOP captured CSS palette', () => {
  it('reads only28 inherited public variables without needing a fabricated chat row', () => {
    const colors: Record<string, string> = { '--account-secret': 'not observed' };
    for (let i = 1; i <= 20; i++) colors[`--nickname-ramdom-color${i}`] = '#A1B2C3';
    for (const role of roles) colors[`--${role}-color`] = 'rgb(12, 13, 14)';
    const f = fixture({ colors }); f.observer!.configure(1);
    expect(f.latest().palette).toBeNull(); expect(f.computedStyle).not.toHaveBeenCalled();
    f.observer!.sample(true);
    expect(f.latest().palette).toEqual({ version: 1, basis: 'computed-css-variable', nickname: Array(20).fill('#a1b2c3'), roles: Object.fromEntries(roles.map(role => [role, '#0c0d0e'])) });
    expect(f.computedStyle).toHaveBeenCalledTimes(1);
    expect(f.getPropertyValue.mock.calls.map(call => call[0])).toEqual([...Array.from({ length: 20 }, (_, i) => `--nickname-ramdom-color${i + 1}`), ...roles.map(role => `--${role}-color`)]);
    expect(document.querySelectorAll('.author,.chatting-list-item')).toHaveLength(0);
    expect(JSON.stringify(f.observations)).not.toContain('not observed');
  });
  it.each([
    [' #ABCDEF\t', '#abcdef'], ['rgb(0, 127, 255)', '#007fff'], ['rgba(001,2,3,1.000)', '#010203'],
    ['rgb(11 22 33)', '#0b1621'], ['rgba(255 0 1 / 1)', '#ff0001'], ['rgb(1 2 3 /1.0)', '#010203'],
  ])('normalizes an exact opaque color %s', (input, expected) => {
    const f = fixture({ colors: { '--normal-color': input } }); f.observer!.configure(1); f.observer!.sample(true);
    expect(f.latest().palette?.roles.normal).toBe(expected);
  });
  it.each(['#abc', '#abcdef00', 'red', 'transparent', 'var(--normal-color)', 'url(https://private.test)', 'calc(1 + 2)',
    'rgb(256,0,0)', 'rgb(-1,0,0)', 'rgb(1.1,0,0)', 'rgb(100%,0%,0%)', 'rgba(1,2,3,.5)', 'rgb(1 2 3/0)',
    'rgb(1, 2 3)', 'rgb(NaN,0,0)', 'color(display-p3 1 0 0)', '#abcdef\u00a0', ` ${'#abcdef'.padEnd(64, ' ')}()`])('keeps unsupported CSS unknown: %s', input => {
    const f = fixture({ colors: { '--normal-color': input, '--fan-color': '#123456' } }); f.observer!.configure(1); f.observer!.sample(true);
    expect(f.latest().palette?.roles.normal).toBeNull(); expect(f.latest().palette?.roles.fan).toBe('#123456');
  });
  it('isolates missing style and individual failures from existing valid preferences', () => {
    const f = fixture({ raw: '{"st1_emoticon":true}', colors: { '--normal-color': '#123456', '--fan-color': '#abcdef' } });
    f.observer!.configure(1); f.setStyleThrows(true); f.observer!.sample(true);
    expect(f.latest().palette).toBeNull(); expect(f.latest().values.showEmoticons).toBe(true);
    f.setStyleThrows(false); f.setPropertyThrows('--normal-color'); f.observer!.sample(true);
    expect(f.latest().palette?.roles.normal).toBeNull(); expect(f.latest().palette?.roles.fan).toBe('#abcdef');
    document.getElementById('chat_area')!.remove(); f.observer!.sample(true); expect(f.latest().palette).toBeNull();
  });
  it('retains all-null slots as unknown instead of filling the pinned CSS default', () => {
    const f = fixture(); f.observer!.configure(1); f.observer!.sample(true);
    expect(f.latest().palette?.nickname).toEqual(Array(20).fill(null)); expect(Object.values(f.latest().palette!.roles)).toEqual(roles.map(() => null));
  });
  it('captures a palette-only change through the existing heartbeat and clears it on epoch reset', async () => {
    const f = fixture({ integrated: true, colors: { '--normal-color': '#123456' } });
    f.win.__atsumiSoopOriginal!.configure({ epoch: 1, channel: 'fixture' }); await drain();
    const before = f.native.filter(row => row.type === 'appearance').length;
    f.setColor('--normal-color', '#ABCDEF'); f.heartbeat();
    const captured = f.native.filter(row => row.type === 'appearance'); expect(captured).toHaveLength(before + 1);
    expect(captured.at(-1)).toMatchObject({ palette: { roles: { normal: '#abcdef' } } });
    f.heartbeat(); expect(f.native.filter(row => row.type === 'appearance')).toHaveLength(before + 1);
    f.win.__atsumiSoopOriginal!.configure({ epoch: 2, channel: 'fixture' });
    const epoch = f.native.filter(row => row.type === 'appearance' && row.epoch === 2);
    expect(epoch[0]?.palette).toBeNull(); expect(epoch[1]).toMatchObject({ palette: { roles: { normal: '#abcdef' } } });
    expect(f.setInterval).toHaveBeenCalledTimes(1);
  });
  it('does not expose old palette while unbound and resends identical state on return', () => {
    const f = fixture({ colors: { '--normal-color': '#123456' } }); f.observer!.configure(1); f.observer!.sample(true);
    const count = f.observations.length; f.bind({ actualChannel: 'fixture', actualBroadcast: '999' });
    expect(f.observer!.sample(true)).toBe(false); expect(f.observations).toHaveLength(count);
    f.bind({ actualChannel: 'fixture', actualBroadcast: '12345' }); expect(f.observer!.sample(true)).toBe(true);
    expect(f.latest().palette?.roles.normal).toBe('#123456');
    f.observer!.configure(2); expect(f.latest().palette).toBeNull();
  });
  it('keeps the complete worst-case wire and persisted native control within2048 bytes', () => {
    const actualChannel = 'a'.repeat(40), actualBroadcast = '9'.repeat(20);
    const colors = Object.fromEntries([...Array.from({ length: 20 }, (_, i) => `--nickname-ramdom-color${i + 1}`), ...roles.map(role => `--${role}-color`)].map(key => [key, '#abcdef']));
    const f = surfaceFixture({ url: `https://play.sooplive.com/${actualChannel}/${actualBroadcast}/embed`, colors,
      raw: '{"st1_emoticon":false,"st1_emoticon_ani":false,"st1_ogq":false,"st1_fanchatcolor":false}' });
    f.row(); f.bind({ actualChannel, actualBroadcast }); f.observer!.configure(Number.MAX_SAFE_INTEGER); f.observer!.sample(true);
    const snapshot = { ...f.latest(), sequence: Number.MAX_SAFE_INTEGER };
    const packet = { ...snapshot, epoch: Number.MAX_SAFE_INTEGER, id: 'f'.repeat(36), type: 'appearance' };
    const control = { format: 'atsumi-soop-display-v1', receiverId: 'ffffffff-ffff-4fff-bfff-ffffffffffff', documentEpoch: Number.MAX_SAFE_INTEGER,
      sequence: Number.MAX_SAFE_INTEGER, receivedAtMs: 9_000_000_000_000, observedAtMs: 9_000_000_000_000, initial: false,
      values: snapshot.values, basis: snapshot.basis, palette: snapshot.palette, paletteIncomplete: true, surface: snapshot.surface, surfaceIncomplete: true };
    expect(new TextEncoder().encode(`ATSUMI_SOOP_V1:${JSON.stringify(packet)}`).length).toBeLessThanOrEqual(2048);
    expect(new TextEncoder().encode(JSON.stringify(control)).length).toBeLessThanOrEqual(2048);
    expect(snapshot.palette?.nickname).toHaveLength(20); expect(Object.keys(snapshot.palette!.roles)).toHaveLength(8);
    expect(snapshot.surface).toMatchObject({ foreground: '#e2e4e9', background: '#0c0d0e' });
  });
});
