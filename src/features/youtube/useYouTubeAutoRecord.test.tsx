import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import type { YouTubeAutoChannel, YouTubeAutoRecordApi, YouTubeAutoSnapshot } from '../../api/youtubeAutoRecord';
import type { YouTubeMetadata } from './model';
import { autoRecordLabel, useYouTubeAutoRecord } from './useYouTubeAutoRecord';
const id = 'UC4QobU6STFB0P71PMvOGN5A';
const favorite: YouTubeMetadata = { kind: 'channel', id: `channel/${id}`, url: `https://www.youtube.com/channel/${id}`, title: '채널', author: '' };
const favorites = [favorite];
const row: YouTubeAutoChannel = { channelId: id, name: '채널', enabled: true, status: 'idle', liveStatus: 'offline', jobId: null, checkedAt: 1, error: null, recordingCount: 0 };
const off: YouTubeAutoSnapshot = { revision: 1, channels: [], ready: true, error: null };
const on: YouTubeAutoSnapshot = { ...off, revision: 2, channels: [row] };
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { promise, resolve }; };
let host: HTMLDivElement, root: Root;
beforeEach(() => { vi.useFakeTimers(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
function View({ api, active = true, items = favorites }: { api: YouTubeAutoRecordApi; active?: boolean; items?: YouTubeMetadata[] }) {
  const model = useYouTubeAutoRecord(api, active, items);
  return <><button disabled={!model.canToggle(items[0]!)} aria-pressed={!!model.rowFor(items[0]!)?.enabled} onClick={() => void model.toggle(items[0]!)}>예약</button><output>{model.rows.length}</output><p role="alert">{model.error}</p></>;
}
const apiFixture = () => ({ snapshot: vi.fn().mockResolvedValue(off), resolve: vi.fn().mockResolvedValue({ channelId: id, name: '채널' }), set: vi.fn().mockResolvedValue(on) });
const render = async (api: YouTubeAutoRecordApi, active = true, items = favorites) => act(async () => root.render(<View api={api} active={active} items={items} />));
const click = async () => act(async () => host.querySelector('button')!.click());

it('only explicit channel toggles write; inactive/privacy panel never polls or resolves', async () => {
  const api = apiFixture(); await render(api, false); await act(async () => vi.advanceTimersByTimeAsync(6000));
  expect(api.snapshot).not.toHaveBeenCalled(); expect(api.resolve).not.toHaveBeenCalled(); expect(api.set).not.toHaveBeenCalled();
  await render(api); await click(); expect(api.set).toHaveBeenCalledExactlyOnceWith(id, '채널', true, 1);
  await render(api, false); const calls = api.snapshot.mock.calls.length; await act(async () => vi.advanceTimersByTimeAsync(6000));
  expect(api.snapshot).toHaveBeenCalledTimes(calls); expect(api.set).toHaveBeenCalledTimes(1);
});
it('locks duplicate clicks and keeps the fixed toggle unchanged until durable acknowledgement', async () => {
  const write = deferred<YouTubeAutoSnapshot>(), api = apiFixture(); api.set.mockReturnValue(write.promise);
  await render(api); const button = host.querySelector('button'); await click(); await click();
  expect(api.set).toHaveBeenCalledTimes(1); expect(button).toBeDisabled(); expect(button).toHaveAttribute('aria-pressed', 'false');
  await act(async () => write.resolve(on)); expect(host.querySelector('button')).toBe(button); expect(button).toHaveAttribute('aria-pressed', 'true');
});
it('late polling cannot overwrite a write and failed polling cannot disable an existing reservation', async () => {
  const old = deferred<YouTubeAutoSnapshot>(), api = apiFixture(); api.snapshot.mockResolvedValueOnce(off).mockReturnValueOnce(old.promise).mockRejectedValue('일시 오류');
  await render(api); await act(async () => vi.advanceTimersByTimeAsync(2000)); await click(); await act(async () => old.resolve(off));
  expect(host.querySelector('button')).toHaveAttribute('aria-pressed', 'true'); await act(async () => vi.advanceTimersByTimeAsync(2000));
  expect(host.querySelector('button')).toHaveAttribute('aria-pressed', 'true'); expect(host.querySelector('[role=alert]')).toHaveTextContent('일시 오류');
});
it('CAS conflict refreshes once without retrying; explicit fault-time off does not resolve the network', async () => {
  const api = apiFixture(); api.snapshot.mockResolvedValueOnce({ ...on, error: '설정 오류' }).mockResolvedValue({ ...on, revision: 3, channels: [{ ...row, enabled: false }], error: '이번 실행에만 적용' }); api.set.mockRejectedValue('이번 실행에만 적용');
  await render(api); expect(host.querySelector('button')).not.toBeDisabled(); await click();
  expect(api.resolve).not.toHaveBeenCalled(); expect(api.set).toHaveBeenCalledExactlyOnceWith(id, '채널', false, 2);
  expect(host.querySelector('button')).toBeDisabled(); expect(host.querySelector('[role=alert]')).toHaveTextContent('이번 실행에만 적용');
});
it('API replacement fences old writes and does not reuse its revision', async () => {
  const previous = deferred<YouTubeAutoSnapshot>(), next = deferred<YouTubeAutoSnapshot>();
  const a = apiFixture(), b = apiFixture(); a.set.mockReturnValue(previous.promise); b.set.mockReturnValue(next.promise);
  await render(a); await click(); await render(b); await click(); await act(async () => previous.resolve(on));
  expect(host.querySelector('button')).toBeDisabled(); expect(host.querySelector('button')).toHaveAttribute('aria-pressed', 'false');
  await act(async () => next.resolve(on)); expect(host.querySelector('button')).toHaveAttribute('aria-pressed', 'true');
});
it('reconciles handle favorites once with two bounded requests rather than restarting on every alias', async () => {
  const api = apiFixture(); api.snapshot.mockResolvedValue(on);
  const items = Array.from({ length: 6 }, (_, i) => ({ ...favorite, id: `@channel${i}`, url: `https://www.youtube.com/@channel${i}` }));
  await render(api, true, items); await act(async () => vi.advanceTimersByTimeAsync(8000));
  expect(api.resolve).toHaveBeenCalledTimes(6); expect(api.set).not.toHaveBeenCalled(); expect(host.querySelector('output')).toHaveTextContent('6');
});
it('rejects ordinary video favorites and describes multiple recordings without claiming failure complete', async () => {
  const api = apiFixture(); await render(api, true, [{ ...favorite, kind: 'video', id: 'abcdefghijk', url: 'https://www.youtube.com/watch?v=abcdefghijk' }]); await click();
  expect(api.set).not.toHaveBeenCalled(); expect(autoRecordLabel()).toBe('예약 꺼짐'); expect(autoRecordLabel(row)).toBe('녹화 예약 됨');
  expect(autoRecordLabel({ ...row, status: 'recording', recordingCount: 2 })).toBe('2개 녹화 중'); expect(autoRecordLabel({ ...row, status: 'failed' })).toBe('확인 필요');
});
