import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SoopVodPlayer } from './SoopVodPlayer';
import type { SoopVodApi, SoopVodSnapshot } from '../../api/soopVod';

const events = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => void) => { events.set(name, callback); return () => events.delete(name); }) }));
vi.mock('../platforms/nativePlayerGeometry', () => ({
  measureNativePlayerGeometry: (player: unknown, chat: unknown, options: object) => ({ ...options, player: player ? { x: 0, y: 0, width: 640, height: 360 } : null, chat: chat ? { x: 647, y: 0, width: 240, height: 360 } : null, occlusions: [] }),
  observeNativePlayerGeometry: () => () => {},
}));

let host: HTMLDivElement, root: Root;
beforeEach(() => { vi.useFakeTimers(); events.clear(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.restoreAllMocks(); });
const snapshot = (videoId = '209244227', extra: Partial<SoopVodSnapshot> = {}): SoopVodSnapshot => ({ token: `lease-${videoId}`, videoId, state: 'playing', chatState: 'available', fullscreen: false, error: null, ...extra });
function adapter() {
  return { open: vi.fn<SoopVodApi['open']>().mockImplementation(async id => snapshot(id)), snapshot: vi.fn<SoopVodApi['snapshot']>().mockResolvedValue(snapshot()), viewport: vi.fn<SoopVodApi['viewport']>().mockResolvedValue(undefined), close: vi.fn<SoopVodApi['close']>().mockResolvedValue(undefined), fullscreen: vi.fn<SoopVodApi['fullscreen']>().mockResolvedValue(undefined) };
}
async function render(api: SoopVodApi, props: { videoId?: string; active?: boolean; privacyMode?: boolean } = {}) { await act(async () => root.render(<SoopVodPlayer api={api} videoId="209244227" title="테스트 영상" active privacyMode={false} {...props} />)); }
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(res => { resolve = res; }); return { promise, resolve }; }
async function tick() { await act(async () => { await vi.advanceTimersByTimeAsync(1500); }); }

describe('SOOP official watch-only VOD owner', () => {
  it('retains the exact lease when hidden; privacy/tray mute without reopening or creating media controls', async () => {
    const api = adapter(); await render(api);
    expect(api.open).toHaveBeenCalledWith('209244227', expect.any(String));
    expect(host.querySelector('video,iframe')).toBeNull();
    expect(host).not.toHaveTextContent('녹화');
    await render(api, { active: false });
    expect(api.viewport).toHaveBeenLastCalledWith('lease-209244227', expect.objectContaining({ visible: false, muted: false }));
    await render(api, { active: true, privacyMode: true });
    expect(api.viewport).toHaveBeenLastCalledWith('lease-209244227', expect.objectContaining({ visible: false, muted: true }));
    await render(api);
    await act(async () => events.get('platform-media:tray-muted')?.({ payload: true }));
    expect(api.viewport).toHaveBeenLastCalledWith('lease-209244227', expect.objectContaining({ muted: true }));
    expect(api.open).toHaveBeenCalledTimes(1); expect(api.close).not.toHaveBeenCalled();
  });
  it('closes only a late old open and keeps the replacement session', async () => {
    const api = adapter(), old = deferred<SoopVodSnapshot>(); api.open.mockImplementationOnce(() => old.promise);
    await render(api); await render(api, { videoId: '209244228' });
    await act(async () => old.resolve(snapshot()));
    expect(api.close).toHaveBeenCalledWith('lease-209244227');
    expect(api.close).not.toHaveBeenCalledWith('lease-209244228');
    expect(api.viewport).toHaveBeenLastCalledWith('lease-209244228', expect.any(Object));
  });
  it('rejects a mismatched open and closes its exact acquired lease', async () => {
    const api = adapter(); api.open.mockResolvedValue(snapshot('123'));
    await render(api);
    expect(host.querySelector('[role="alert"]')).toHaveTextContent('요청한 영상과 재생 응답이 일치하지 않습니다.');
    expect(api.close).toHaveBeenCalledWith('lease-123'); expect(api.viewport).not.toHaveBeenCalled();
  });
  it('keeps ended playback available for native seek/replay without a reload', async () => {
    const api = adapter(); await render(api);
    api.snapshot.mockResolvedValue(snapshot('209244227', { state: 'ended' })); await tick();
    expect(host.querySelector('.soop-vod-pair')).toHaveAttribute('data-state', 'ended');
    expect(host.querySelector('[role="alert"]')).toBeNull();
    api.snapshot.mockResolvedValue(snapshot()); await tick();
    expect(api.open).toHaveBeenCalledTimes(1); expect(api.close).not.toHaveBeenCalled();
    expect(host.querySelector('.soop-vod-pair')).toHaveAttribute('data-state', 'playing');
  });
  it('reserves chat only when official availability is confirmed and removes it when unavailable', async () => {
    const api = adapter(); api.open.mockResolvedValue(snapshot('209244227', { chatState: 'pending' })); await render(api);
    expect(host.querySelector('[role="separator"]')).toBeNull();
    expect(api.viewport).toHaveBeenLastCalledWith('lease-209244227', expect.objectContaining({ chat: null }));
    await tick(); expect(host.querySelector('[role="separator"]')).not.toBeNull();
    expect(api.viewport).toHaveBeenLastCalledWith('lease-209244227', expect.objectContaining({ chat: expect.any(Object) }));
    api.snapshot.mockResolvedValue(snapshot('209244227', { chatState: 'unavailable' })); await tick();
    expect(host.querySelector('[role="separator"]')).toBeNull();
    expect(api.viewport).toHaveBeenLastCalledWith('lease-209244227', expect.objectContaining({ chat: null }));
    expect(api.open).toHaveBeenCalledTimes(1);
  });
  it('does not overlap snapshot requests and clears a recovered transport error', async () => {
    const api = adapter(); await render(api);
    const pending = deferred<SoopVodSnapshot>(); api.snapshot.mockImplementationOnce(() => pending.promise);
    await tick(); await tick(); expect(api.snapshot).toHaveBeenCalledTimes(1);
    await act(async () => pending.resolve(snapshot()));
    api.snapshot.mockRejectedValueOnce('잠시 연결 실패'); await tick(); expect(host).toHaveTextContent('잠시 연결 실패');
    await tick(); expect(host.querySelector('[role="alert"]')).toBeNull(); expect(api.open).toHaveBeenCalledTimes(1);
  });
  it('ignores a stale poll after replacement and rejects mismatched current polls', async () => {
    const api = adapter(); await render(api);
    const old = deferred<SoopVodSnapshot>(); api.snapshot.mockImplementationOnce(() => old.promise); await tick();
    await render(api, { videoId: '209244228' }); await act(async () => old.resolve(snapshot('209244227', { state: 'error', error: '오래된 오류' })));
    expect(host).not.toHaveTextContent('오래된 오류');
    await tick(); expect(host).toHaveTextContent('재생 상태 응답이 현재 영상과 일치하지 않습니다.');
    expect(api.close).toHaveBeenCalledTimes(1); expect(api.close).toHaveBeenCalledWith('lease-209244227');
  });
  it('uses only the active document fullscreen event and exits fullscreen on hide without closing', async () => {
    const api = adapter(); await render(api);
    await act(async () => events.get('soop-vod:fullscreen')?.({ payload: { videoId: '123', enabled: true } }));
    expect(host.querySelector('.is-native-fullscreen')).toBeNull();
    api.snapshot.mockResolvedValue(snapshot('209244227', { fullscreen: true }));
    await act(async () => events.get('soop-vod:fullscreen')?.({ payload: { videoId: '209244227', enabled: true } }));
    expect(host.querySelector('.is-native-fullscreen')).not.toBeNull();
    await render(api, { active: false });
    expect(api.fullscreen).toHaveBeenCalledWith('lease-209244227', false); expect(api.close).not.toHaveBeenCalled();
  });
  it('treats delayed same-video fullscreen events as hints and never trusts their old enabled value', async () => {
    const api = adapter(); await render(api);
    api.viewport.mockClear();
    const pending = deferred<SoopVodSnapshot>(); api.snapshot.mockImplementationOnce(() => pending.promise);
    await act(async () => {
      events.get('soop-vod:fullscreen')?.({ payload: { videoId: '209244227', enabled: true } });
      events.get('soop-vod:fullscreen')?.({ payload: { videoId: '209244227', enabled: true } });
    });
    expect(api.snapshot).toHaveBeenCalledTimes(1); expect(api.snapshot).toHaveBeenCalledWith('lease-209244227');
    expect(host.querySelector('.is-native-fullscreen')).toBeNull();
    await act(async () => pending.resolve(snapshot()));
    expect(host.querySelector('.is-native-fullscreen')).toBeNull();
  });
  it('retries a failed viewport at most twice and recovers without reopening the document', async () => {
    const api = adapter(); api.viewport.mockRejectedValueOnce('배치 실패').mockRejectedValueOnce('배치 실패');
    await render(api); expect(api.viewport).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(250); }); expect(api.viewport).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(750); }); expect(api.viewport).toHaveBeenCalledTimes(3);
    const sequences = api.viewport.mock.calls.map(call => call[1].sequence);
    expect(sequences[1]).toBeGreaterThan(sequences[0]!); expect(sequences[2]).toBeGreaterThan(sequences[1]!);
    expect(host.querySelector('[role="alert"]')).toBeNull(); expect(api.open).toHaveBeenCalledTimes(1); expect(api.close).not.toHaveBeenCalled();
  });
  it('stops exhausted viewport retries and offers a viewport-only manual retry', async () => {
    const api = adapter(); api.viewport.mockRejectedValue('배치 실패'); await render(api);
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); }); expect(api.viewport).toHaveBeenCalledTimes(3);
    api.viewport.mockResolvedValue(undefined);
    await act(async () => host.querySelector<HTMLButtonElement>('[role="alert"] button')!.click());
    expect(api.viewport).toHaveBeenCalledTimes(4); expect(api.open).toHaveBeenCalledTimes(1); expect(host.querySelector('[role="alert"]')).toBeNull();
  });
  it('cancels a visible viewport retry when hidden and sends only the new hidden geometry', async () => {
    const api = adapter(); api.viewport.mockRejectedValueOnce('배치 실패'); await render(api);
    await render(api, { active: false });
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(api.viewport).toHaveBeenCalledTimes(2);
    expect(api.viewport).toHaveBeenLastCalledWith('lease-209244227', expect.objectContaining({ visible: false, muted: false }));
  });
  it('supports bounded keyboard chat resizing without reconnecting', async () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, left: 0, top: 0, right: 900, bottom: 500, width: 900, height: 500, toJSON() {} });
    const api = adapter(); await render(api);
    const separator = host.querySelector('[role="separator"]')!;
    expect(separator).toHaveAttribute('aria-valuenow', '240');
    await act(async () => separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true })));
    expect(separator).toHaveAttribute('aria-valuenow', '260');
    await act(async () => separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true })));
    expect(separator).toHaveAttribute('aria-valuenow', '96'); expect(api.open).toHaveBeenCalledTimes(1);
  });
});
