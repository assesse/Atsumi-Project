import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformChatParts, safeChatImage, useChatAssets } from './PlatformChatParts';
import type { MediaChatAsset, MediaChatPart } from './platformMedia';
const id = 'a'.repeat(64), id2 = 'b'.repeat(64), image = 'data:image/png;base64,aGVsbG8=';
const parts: MediaChatPart[] = [{ type: 'text', text: '<script> :typed: ' }, { type: 'emoji', assetId: id, alt: ':flower:' }, { type: 'text', text: ' after' }];
let root: Root, host: HTMLDivElement;
function Fixture({ scope = 'live', enabled = true, load, source = parts, live = false }: { scope?: string; enabled?: boolean; load: (scope: string, id: string) => Promise<MediaChatAsset | string | null>; source?: MediaChatPart[]; live?: boolean }) {
  const images = useChatAssets(scope, [{ parts: source }], enabled, load, live);
  return <PlatformChatParts text="fallback" parts={source} images={images} />;
}
describe('safe ordered chat parts', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.restoreAllMocks(); });
  it('only permits bounded native raster data URLs, never remote/SVG/HTML', () => {
    expect(safeChatImage(image)).toBe(true);
    for (const value of ['https://yt3.ggpht.com/a', 'data:image/svg+xml;base64,AAAA', 'javascript:alert(1)', `data:image/png;base64,${'a'.repeat(1_400_000)}`]) expect(safeChatImage(value)).toBe(false);
  });
  it('preserves order and escaped text and requests only opaque IDs', async () => {
    const load = vi.fn().mockResolvedValue(image);
    await act(async () => root.render(<Fixture load={load} />));
    expect(load).toHaveBeenCalledExactlyOnceWith('live', id); expect(host.querySelector('script')).toBeNull();
    expect([...host.childNodes].map(node => node.nodeName)).toEqual(['SPAN', 'IMG', 'SPAN']);
    expect(host.querySelector('img')?.alt).toBe(':flower:'); expect(host.textContent).toContain(':typed:');
  });
  it('falls back to legacy text and does not convert typed aliases into images', async () => {
    await act(async () => root.render(<PlatformChatParts text="old :flower:" images={new Map()} />));
    expect(host.textContent).toBe('old :flower:'); expect(host.querySelector('img')).toBeNull();
  });
  it('uses visible text when local image is missing, remote, or decode fails', async () => {
    const load = vi.fn().mockResolvedValue('https://yt3.ggpht.com/a');
    await act(async () => root.render(<Fixture load={load} />)); expect(host.textContent).toContain(':flower:'); expect(host.querySelector('img')).toBeNull();
    await act(async () => root.render(<Fixture scope="other" load={vi.fn().mockResolvedValue(image)} />));
    await act(async () => host.querySelector('img')!.dispatchEvent(new Event('error'))); expect(host.textContent).toContain(':flower:'); expect(host.querySelector('img')).toBeNull();
  });
  it('does not read on privacy/inactive and discards stale scope completions', async () => {
    let resolve!: (value: string | null) => void;
    const load = vi.fn().mockImplementation(() => new Promise<string | null>(done => { resolve = done; }));
    await act(async () => root.render(<Fixture enabled={false} load={load} />)); expect(load).not.toHaveBeenCalled();
    await act(async () => root.render(<Fixture load={load} />)); expect(load).toHaveBeenCalledOnce();
    await act(async () => root.render(<Fixture scope="next" enabled={false} load={load} />));
    await act(async () => resolve(image)); expect(host.querySelector('img')).toBeNull();
  });
  it('limits actual in-flight IPC across rapid scope changes and deduplicates a page', async () => {
    const finish: Array<(value: string | null) => void> = [];
    const load = vi.fn().mockImplementation(() => new Promise<string | null>(done => finish.push(done)));
    const source: MediaChatPart[] = [...parts, { type: 'emoji', assetId: id, alt: ':same:' }, { type: 'emoji', assetId: id2, alt: ':second:' }];
    await act(async () => root.render(<Fixture load={load} source={source} />)); expect(load).toHaveBeenCalledTimes(2);
    await act(async () => root.render(<Fixture scope="next" load={load} source={source} />)); expect(load).toHaveBeenCalledTimes(2);
    await act(async () => { finish[0]!(image); finish[1]!(image); }); expect(load).toHaveBeenCalledTimes(4);
    await act(async () => { finish[2]!(image); finish[3]!(image); }); expect(host.querySelectorAll('img')).toHaveLength(3);
  });
  it('finds assets ready after five seconds without starving later ready images', async () => {
    vi.useFakeTimers(); vi.setSystemTime(0);
    const source: MediaChatPart[] = [{ type: 'emoji', assetId: id, alt: ':pending:' }, { type: 'emoji', assetId: id2, alt: ':ready:' }];
    const load = vi.fn(async (_scope: string, asset: string): Promise<MediaChatAsset> => asset === id && Date.now() < 6000 ? { state: 'pending', dataUrl: null } : { state: 'ready', dataUrl: image });
    await act(async () => root.render(<Fixture source={source} load={load} live />));
    expect(host.querySelectorAll('img')).toHaveLength(1); expect(load.mock.calls.some(call => call[1] === id2)).toBe(true);
    await act(async () => vi.advanceTimersByTimeAsync(8000)); expect(host.querySelectorAll('img')).toHaveLength(2);
    const calls = load.mock.calls.length; await act(async () => vi.advanceTimersByTimeAsync(30000)); expect(load).toHaveBeenCalledTimes(calls);
  });
  it('moving pending IDs and privacy do not leave polling behind; missing never polls', async () => {
    vi.useFakeTimers(); vi.setSystemTime(0);
    const load = vi.fn(async (_scope: string, asset: string): Promise<MediaChatAsset> => ({ state: asset === id ? 'pending' : 'missing', dataUrl: null }));
    await act(async () => root.render(<Fixture load={load} live />));
    await act(async () => root.render(<Fixture load={load} live source={[{ type: 'emoji', assetId: id2, alt: ':missing:' }]} />));
    const calls = load.mock.calls.length; await act(async () => vi.advanceTimersByTimeAsync(30000)); expect(load).toHaveBeenCalledTimes(calls);
    await act(async () => root.render(<Fixture load={load} live />));
    await act(async () => root.render(<Fixture load={load} live enabled={false} />));
    const paused = load.mock.calls.length; await act(async () => vi.advanceTimersByTimeAsync(30000)); expect(load).toHaveBeenCalledTimes(paused);
  });
  it('caps pending polling at six minutes and offline reads remain single attempt', async () => {
    vi.useFakeTimers(); vi.setSystemTime(0);
    const load = vi.fn(async (): Promise<MediaChatAsset> => ({ state: 'pending', dataUrl: null }));
    await act(async () => root.render(<Fixture load={load} live />));
    await act(async () => vi.advanceTimersByTimeAsync(370000)); const calls = load.mock.calls.length;
    expect(calls).toBeLessThanOrEqual(40); await act(async () => vi.advanceTimersByTimeAsync(120000)); expect(load).toHaveBeenCalledTimes(calls);
    await act(async () => root.render(<Fixture scope="offline" load={load} />)); const offline = load.mock.calls.length;
    await act(async () => vi.advanceTimersByTimeAsync(60000)); expect(load).toHaveBeenCalledTimes(offline);
  });
});
