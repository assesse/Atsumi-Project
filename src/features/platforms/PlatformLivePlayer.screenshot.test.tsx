import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { PlatformLivePlayer } from './PlatformLivePlayer';
import type { MediaLiveSnapshot, MediaRef, PlatformLiveApi } from './platformMedia';
import type { PlatformScreenshotItem, PlatformScreenshotsApi } from './platformScreenshots';

const hls = vi.hoisted(() => ({ created: vi.fn(), destroyed: vi.fn() }));
vi.mock('hls.js', () => ({ default: class { static Events = { MANIFEST_PARSED: 'manifest', ERROR: 'error' }; static isSupported() { return true; } constructor() { hls.created(); } on() {} loadSource() {} attachMedia() {} destroy() { hls.destroyed(); } } }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
const reference: MediaRef = { provider: 'youtube', kind: 'live', externalId: 'abcdefghijk' };
const snapshot: MediaLiveSnapshot = { reference, token: 'token-a', url: 'http://platform-media.localhost/live/token-a/index.m3u8', state: 'playing', title: 'Live title', channelName: 'Channel', receivedBytes: 100, recordingJob: null, recordingStopping: false, chatStatus: '', error: null };
const item: PlatformScreenshotItem = { id: 'shot-a', provider: 'youtube', reference, title: 'Live title', channelName: 'Channel', fileName: 'shot-a.png', createdAt: 1_790_000_000_000, bytes: 2, width: 1920, height: 1080 };
let container: HTMLDivElement, root: Root, api: PlatformLiveApi, screenshots: PlatformScreenshotsApi, draw: ReturnType<typeof vi.fn>, encoded: Blob;
let createUrl: ReturnType<typeof vi.fn<(value: Blob | MediaSource) => string>>;
let revokeUrl: ReturnType<typeof vi.fn<(value: string) => void>>;
function blob(size = 2, type = 'image/png') { return { size, type, arrayBuffer: vi.fn(async () => new Uint8Array(size).buffer) } as unknown as Blob; }
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
beforeEach(() => {
  vi.useFakeTimers(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(); vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => undefined); vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => undefined);
  vi.spyOn(HTMLMediaElement.prototype, 'readyState', 'get').mockReturnValue(2);
  vi.spyOn(HTMLVideoElement.prototype, 'videoWidth', 'get').mockReturnValue(1920); vi.spyOn(HTMLVideoElement.prototype, 'videoHeight', 'get').mockReturnValue(1080);
  draw = vi.fn(); encoded = blob();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage: draw } as unknown as CanvasRenderingContext2D);
  vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation(callback => callback(encoded));
  createUrl = vi.fn().mockReturnValue('blob:local-shot'); revokeUrl = vi.fn();
  vi.stubGlobal('URL', class extends URL { static createObjectURL(value: Blob | MediaSource): string { return createUrl(value) as string; } static revokeObjectURL(value: string): void { revokeUrl(value); } });
  api = { open: vi.fn().mockResolvedValue(snapshot), snapshot: vi.fn().mockResolvedValue(snapshot), record: vi.fn().mockResolvedValue(snapshot), close: vi.fn().mockResolvedValue(undefined) };
  screenshots = { begin: vi.fn().mockResolvedValue({ captureId: 'capture-a' }), chunk: vi.fn().mockResolvedValue(undefined), finish: vi.fn().mockResolvedValue(item), abort: vi.fn().mockResolvedValue(undefined), undo: vi.fn().mockResolvedValue(undefined), list: vi.fn(), image: vi.fn(), delete: vi.fn(), folder: vi.fn() };
  hls.created.mockClear(); hls.destroyed.mockClear();
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const render = async (props: Partial<{ privacyMode: boolean; active: boolean; videoVisible: boolean; reference: MediaRef }> = {}) => { await act(async () => root.render(<PlatformLivePlayer reference={reference} title="Title" privacyMode={false} api={api} screenshotsApi={screenshots} {...props} />)); };
const camera = () => container.querySelector<HTMLButtonElement>('.platform-live-screenshot')!;
const click = async () => { await act(async () => camera().dispatchEvent(new MouseEvent('click', { bubbles: true }))); };
const preview = () => container.querySelector('.platform-live-capture-preview');
it('freezes the exact frame before encoding or IPC and keeps the same receiver/node', async () => {
  const order: string[] = []; draw.mockImplementation(() => order.push('draw'));
  vi.mocked(HTMLCanvasElement.prototype.toBlob).mockImplementation(callback => { order.push('encode'); callback(encoded); });
  vi.mocked(screenshots.begin).mockImplementation(async () => { order.push('begin'); return { captureId: 'capture-a' }; });
  await render(); const video = container.querySelector('video'); await click();
  expect(order).toEqual(['draw', 'encode', 'begin']); expect(draw).toHaveBeenCalledExactlyOnceWith(video, 0, 0);
  expect(screenshots.begin).toHaveBeenCalledWith('token-a', expect.any(String), 2, 1920, 1080);
  expect(screenshots.chunk).toHaveBeenCalledWith('token-a', 'capture-a', 0, 'AAA='); expect(screenshots.finish).toHaveBeenCalledWith('token-a', 'capture-a');
  expect(preview()?.querySelector('img')?.getAttribute('src')).toBe('blob:local-shot'); expect(container.querySelector('video')).toBe(video);
  expect(video?.crossOrigin).toBe('anonymous'); expect(api.open).toHaveBeenCalledTimes(1); expect(api.close).not.toHaveBeenCalled(); expect(hls.created).toHaveBeenCalledTimes(1);
  expect(camera().closest('footer')?.classList.contains('platform-live-overlay')).toBe(true); expect(camera().closest('footer')?.lastElementChild?.getAttribute('aria-label')).toBe('전체화면');
  await act(async () => vi.advanceTimersByTimeAsync(2_999)); expect(preview()).not.toBeNull();
  await act(async () => vi.advanceTimersByTimeAsync(1)); expect(preview()).toBeNull(); expect(revokeUrl).toHaveBeenCalledWith('blob:local-shot');
});
it('serializes bounded chunks and ignores repeated capture while one is in flight', async () => {
  encoded = blob(128 * 1024 + 1); const pending = deferred<void>(); vi.mocked(screenshots.chunk).mockReturnValueOnce(pending.promise);
  await render(); await click(); await click();
  expect(screenshots.begin).toHaveBeenCalledTimes(1); expect(screenshots.chunk).toHaveBeenCalledTimes(1); expect(camera().disabled).toBe(true);
  expect(atob(vi.mocked(screenshots.chunk).mock.calls[0]![3])).toHaveLength(128 * 1024); expect(preview()).toBeNull();
  await act(async () => pending.resolve()); expect(screenshots.chunk).toHaveBeenCalledTimes(2); expect(screenshots.chunk).toHaveBeenLastCalledWith('token-a', 'capture-a', 1, 'AA=='); expect(preview()).not.toBeNull();
});
it('undoes only the displayed saved item once and revokes the local preview', async () => {
  const pending = deferred<void>(); vi.mocked(screenshots.undo).mockReturnValueOnce(pending.promise);
  await render(); await click(); const undo = preview()!.querySelector('.platform-live-capture-cancel')!;
  await act(async () => undo.dispatchEvent(new MouseEvent('click', { bubbles: true }))); await act(async () => undo.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  expect(screenshots.undo).toHaveBeenCalledExactlyOnceWith('token-a', 'shot-a'); expect(camera().disabled).toBe(true);
  await act(async () => pending.resolve()); expect(preview()).toBeNull(); expect(revokeUrl).toHaveBeenCalledTimes(1); expect(screenshots.delete).not.toHaveBeenCalled();
});
it('retains an actionable local warning after undo failure without retrying or restarting playback', async () => {
  vi.mocked(screenshots.undo).mockRejectedValueOnce('failure'); await render(); await click();
  await act(async () => preview()!.querySelector('.platform-live-capture-cancel')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  expect(preview()?.textContent).toContain('스크린샷 목록에서 확인'); expect(preview()?.querySelector('.platform-live-capture-cancel')?.getAttribute('aria-disabled')).toBe('true'); expect(api.open).toHaveBeenCalledTimes(1);
  expect(container.querySelector('.platform-live-capture-notice.is-error')?.textContent).toContain('스크린샷 목록에서 확인');
});
it.each(['private', 'inactive', 'hidden'] as const)('blocks capture and immediately hides/revokes review when %s', async mode => {
  await render(); await click(); await render({ privacyMode: mode === 'private', active: mode !== 'inactive', videoVisible: mode !== 'hidden' });
  expect(preview()).toBeNull(); expect(revokeUrl).toHaveBeenCalledTimes(1); expect(camera().disabled).toBe(true); await click();
  expect(screenshots.begin).toHaveBeenCalledTimes(1); expect(api.close).not.toHaveBeenCalled(); expect(screenshots.undo).not.toHaveBeenCalled();
});
it('aborts a late begin after privacy was toggled off and on without reopening its old preview', async () => {
  const pending = deferred<{ captureId: string }>(); vi.mocked(screenshots.begin).mockReturnValueOnce(pending.promise);
  await render(); await click(); await render({ privacyMode: true }); await render();
  await act(async () => pending.resolve({ captureId: 'old-capture' }));
  expect(screenshots.abort).toHaveBeenCalledExactlyOnceWith('token-a', 'old-capture'); expect(screenshots.chunk).not.toHaveBeenCalled(); expect(preview()).toBeNull(); expect(camera().disabled).toBe(false);
});
it('aborts a late chunk after provider/token replacement and never finishes it', async () => {
  const pending = deferred<void>(); vi.mocked(screenshots.chunk).mockReturnValueOnce(pending.promise);
  await render(); await click(); const next: MediaRef = { provider: 'soop', kind: 'live', externalId: 'other_channel' };
  vi.mocked(api.open).mockResolvedValue({ ...snapshot, reference: next, token: 'token-b' }); await render({ reference: next });
  await act(async () => pending.resolve()); expect(screenshots.abort).toHaveBeenCalledExactlyOnceWith('token-a', 'capture-a'); expect(screenshots.finish).not.toHaveBeenCalled(); expect(preview()).toBeNull();
});
it('does not disclose or delete a finished capture returned after unmount', async () => {
  const pending = deferred<PlatformScreenshotItem>(); vi.mocked(screenshots.finish).mockReturnValueOnce(pending.promise);
  await render(); await click(); await act(async () => root.render(null)); await act(async () => pending.resolve(item));
  expect(createUrl).not.toHaveBeenCalled(); expect(screenshots.undo).not.toHaveBeenCalled(); expect(screenshots.delete).not.toHaveBeenCalled(); expect(screenshots.abort).toHaveBeenCalledTimes(1);
});
it('ignores late undo replies after changing context', async () => {
  const pending = deferred<void>(); vi.mocked(screenshots.undo).mockReturnValueOnce(pending.promise); await render(); await click();
  await act(async () => preview()!.querySelector('.platform-live-capture-cancel')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  await render({ active: false }); await render(); await act(async () => pending.resolve()); expect(preview()).toBeNull(); expect(api.open).toHaveBeenCalledTimes(1);
});
it('shows a bounded timeout and aborts a subsequently returned permit', async () => {
  const pending = deferred<{ captureId: string }>(); vi.mocked(screenshots.begin).mockReturnValueOnce(pending.promise); await render(); await click();
  await act(async () => vi.advanceTimersByTimeAsync(19_001)); expect(container.querySelector('.platform-live-capture-notice.is-error')?.textContent).toContain('만료'); expect(camera().disabled).toBe(false);
  await act(async () => pending.resolve({ captureId: 'late' })); expect(screenshots.abort).toHaveBeenCalledWith('token-a', 'late'); expect(screenshots.chunk).not.toHaveBeenCalled();
});
it('dismisses the preview without undoing the file or changing the live receiver', async () => {
  await render(); await click();const video=container.querySelector('video');
  await act(async () => preview()!.querySelector('.platform-live-capture-dismiss')!.dispatchEvent(new MouseEvent('click',{bubbles:true})));
  expect(preview()).toBeNull();expect(revokeUrl).toHaveBeenCalledExactlyOnceWith('blob:local-shot');
  expect(screenshots.undo).not.toHaveBeenCalled();expect(screenshots.delete).not.toHaveBeenCalled();expect(screenshots.abort).not.toHaveBeenCalled();
  expect(container.querySelector('video')).toBe(video);expect(api.open).toHaveBeenCalledOnce();expect(api.close).not.toHaveBeenCalled();
});
it.each(['no-frame', 'oversize-dimensions', 'bad-png', 'oversize-png', 'tainted'] as const)('fails %s locally without sending invalid media', async reason => {
  if (reason === 'no-frame') vi.spyOn(HTMLMediaElement.prototype, 'readyState', 'get').mockReturnValue(0);
  if (reason === 'oversize-dimensions') vi.spyOn(HTMLVideoElement.prototype, 'videoWidth', 'get').mockReturnValue(4097);
  if (reason === 'bad-png') encoded = blob(2, 'image/jpeg');
  if (reason === 'oversize-png') encoded = blob(16 * 1024 * 1024 + 1);
  if (reason === 'tainted') vi.mocked(HTMLCanvasElement.prototype.toBlob).mockImplementation(() => { throw new DOMException('blocked', 'SecurityError'); });
  await render(); await click(); expect(screenshots.begin).not.toHaveBeenCalled(); expect(preview()).toBeNull(); expect(container.querySelector('.platform-live-capture-notice.is-error')).not.toBeNull(); expect(api.open).toHaveBeenCalledTimes(1);
});
it('times out a stalled encoder and zeros its temporary canvas', async () => {
  let canvas!: HTMLCanvasElement; vi.mocked(HTMLCanvasElement.prototype.toBlob).mockImplementation(function (this: HTMLCanvasElement) { canvas = this; });
  await render(); await click(); await act(async () => vi.advanceTimersByTimeAsync(5001));
  expect(canvas.width).toBe(0); expect(canvas.height).toBe(0); expect(camera().disabled).toBe(false); expect(screenshots.begin).not.toHaveBeenCalled();
});
it('targets S at only the focused multi-view player and ignores repeat/IME/modifiers/editing/modal', async () => {
  const second: MediaRef = { provider: 'soop', kind: 'live', externalId: 'fixture_channel' };
  vi.mocked(api.open).mockImplementation(async ref => ({ ...snapshot, reference: ref, token: ref.provider === 'soop' ? 'token-b' : 'token-a' }));
  await act(async () => root.render(<><PlatformLivePlayer reference={reference} title="A" privacyMode={false} api={api} screenshotsApi={screenshots} /><PlatformLivePlayer reference={second} title="B" privacyMode={false} api={api} screenshotsApi={screenshots} /></>));
  const surfaces = container.querySelectorAll<HTMLElement>('.platform-live-player'), surface = surfaces[1]!;
  for (const extra of [{ repeat: true }, { ctrlKey: true }, { shiftKey: true }, { altKey: true }, { metaKey: true }, { isComposing: true }]) await act(async () => surface.dispatchEvent(new KeyboardEvent('keydown', { key: 's', code: 'KeyS', bubbles: true, ...extra })));
  const input = document.createElement('textarea'); surface.append(input); await act(async () => input.dispatchEvent(new KeyboardEvent('keydown', { key: 's', bubbles: true }))); input.remove();
  const dialog = document.createElement('dialog'); dialog.setAttribute('open', ''); dialog.getBoundingClientRect = () => new DOMRect(0, 0, 200, 100); document.body.append(dialog); await act(async () => surface.dispatchEvent(new KeyboardEvent('keydown', { key: 's', bubbles: true }))); dialog.remove();
  expect(screenshots.begin).not.toHaveBeenCalled(); surface.focus(); await act(async () => surface.dispatchEvent(new KeyboardEvent('keydown', { key: 's', code: 'KeyS', bubbles: true })));
  expect(screenshots.begin).toHaveBeenCalledExactlyOnceWith('token-b', expect.any(String), 2, 1920, 1080); expect(draw.mock.calls[0]?.[0]).toBe(surface.querySelector('video')); expect(surfaces[0]?.querySelector('.platform-live-capture-preview')).toBeNull(); expect(api.open).toHaveBeenCalledTimes(2);
});
it('allows S inside its own floating detail dialog and ignores retained hidden dialogs', async () => {
  await act(async () => root.render(<div role="dialog" aria-modal="true"><PlatformLivePlayer reference={reference} title="Title" privacyMode={false} api={api} screenshotsApi={screenshots} /></div>));
  const other = document.createElement('div'); other.setAttribute('role', 'dialog'); other.setAttribute('aria-modal', 'true'); other.hidden = true; other.getBoundingClientRect = () => new DOMRect(0, 0, 200, 100); document.body.append(other);
  try { await act(async () => container.querySelector('.platform-live-player')!.dispatchEvent(new KeyboardEvent('keydown', { key: 's', bubbles: true }))); expect(screenshots.begin).toHaveBeenCalledTimes(1); }
  finally { other.remove(); }
});
