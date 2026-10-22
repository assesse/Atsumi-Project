import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { PlatformLivePlayer } from './PlatformLivePlayer';
import type { MediaLiveSnapshot, PlatformLiveApi } from './platformMedia';
const hls = vi.hoisted(() => ({ created: vi.fn(), destroyed: vi.fn(), sources: vi.fn(), listeners: new Map<string, (...args: unknown[]) => void>() }));
vi.mock('hls.js', () => ({ default: class { static Events = { MANIFEST_PARSED: 'manifest', ERROR: 'error' }; static isSupported() { return true; } constructor() { hls.created(); } on(name: string, fn: (...args: unknown[]) => void) { hls.listeners.set(name, fn); } loadSource(url: string) { hls.sources(url); } attachMedia() {} destroy() { hls.destroyed(); } } }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
const reference = { provider: 'youtube', kind: 'live', externalId: 'abcdefghijk' } as const;
const snapshot: MediaLiveSnapshot = { reference, token: 'live-token', url: 'http://platform-media.localhost/live/live-token/index.m3u8', state: 'playing', title: 'Live title', channelName: 'Channel', receivedBytes: 100, recordingJob: null, recordingStopping: false, chatStatus: '', error: null };
let container: HTMLDivElement, root: Root, api: PlatformLiveApi;
beforeEach(() => { vi.useFakeTimers(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); container = document.createElement('div'); document.body.append(container); root = createRoot(container); vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(); vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => undefined); vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => undefined); api = { open: vi.fn().mockResolvedValue(snapshot), snapshot: vi.fn().mockResolvedValue(snapshot), record: vi.fn().mockImplementation(async (_, enabled) => ({ ...snapshot, recordingJob: enabled ? 'recording-id' : null })), close: vi.fn().mockResolvedValue(undefined) }; hls.created.mockClear(); hls.destroyed.mockClear(); hls.sources.mockClear(); hls.listeners.clear(); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const render = async (privacyMode = false) => { await act(async () => root.render(<PlatformLivePlayer reference={reference} title="Title" privacyMode={privacyMode} api={api} />)); };
it('uses one native receiver and keeps its lease through privacy changes', async () => { await render(); const video = container.querySelector('video')!; expect(api.open).toHaveBeenCalledTimes(1); expect(hls.sources).toHaveBeenCalledWith(snapshot.url); await render(true); expect(container.querySelector('video')).toBe(video); expect(video.muted).toBe(true); expect(api.close).not.toHaveBeenCalled(); expect(hls.created).toHaveBeenCalledTimes(1); expect(container.textContent).not.toContain('Live title'); });
it('stops the loading indicator after a failed open and permits an explicit retry', async () => { vi.mocked(api.open).mockRejectedValueOnce('연결 실패'); await render(); expect(container.querySelector('[role=alert]')?.textContent).toContain('연결 실패'); expect(container.querySelector('[role=status]')).toBeNull(); const retry = container.querySelector('[role=alert] button')!; expect(retry.textContent).toBe('다시 시도'); await act(async () => retry.dispatchEvent(new MouseEvent('click', { bubbles: true }))); expect(api.open).toHaveBeenCalledTimes(2); expect(container.querySelector('[role=alert]')).toBeNull(); expect(hls.sources).toHaveBeenCalledWith(snapshot.url); });
it('reports a failed native snapshot even when its error field is missing', async () => { vi.mocked(api.open).mockResolvedValue({ ...snapshot, state: 'failed', error: null }); await render(); expect(container.querySelector('[role=alert]')?.textContent).toContain('라이브 수신이 중단됐습니다.'); expect(container.querySelector('[role=status]')).toBeNull(); });
it('clears a transient snapshot error after recovery without reopening the receiver', async () => {
  vi.mocked(api.snapshot).mockRejectedValueOnce('일시 연결 실패'); await render();
  await act(async () => vi.advanceTimersByTimeAsync(1000)); expect(container.querySelector('[role=alert]')?.textContent).toContain('일시 연결 실패');
  await act(async () => vi.advanceTimersByTimeAsync(1000)); expect(container.querySelector('[role=alert]')).toBeNull();
  expect(api.open).toHaveBeenCalledTimes(1); expect(hls.created).toHaveBeenCalledTimes(1);
});
it('does not erase a recording-operation error when snapshot polling succeeds', async () => {
  vi.mocked(api.record).mockRejectedValueOnce('저장 공간 부족'); await render();
  await act(async () => container.querySelector('.platform-live-record')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(container.querySelector('[role=alert]')?.textContent).toContain('저장 공간 부족'); expect(api.open).toHaveBeenCalledTimes(1);
});
it('starts recording on the existing viewer token rather than another receiver', async () => { await render(); const button = container.querySelector('.platform-live-record')!; await act(async () => button.dispatchEvent(new MouseEvent('click', { bubbles: true }))); expect(api.record).toHaveBeenCalledWith('live-token', true); expect(api.open).toHaveBeenCalledTimes(1); expect(container.querySelector('.platform-live-record')).toBe(button); expect(button.getAttribute('aria-pressed')).toBe('true'); });
it('fades all controls together after movement stops', async () => { await render(); const video = container.querySelector('video')!; Object.defineProperty(video, 'paused', { configurable: true, value: false }); await act(async () => video.dispatchEvent(new Event('play', { bubbles: true }))); expect(container.querySelector('.show-controls')).not.toBeNull(); await act(async () => vi.advanceTimersByTimeAsync(2300)); expect(container.querySelector('.show-controls')).toBeNull(); expect(container.querySelectorAll('.platform-live-overlay')).toHaveLength(2); });
it('closes a lease returned after the component has already unmounted', async () => { let resolve!: (value: MediaLiveSnapshot) => void; vi.mocked(api.open).mockReturnValue(new Promise(done => { resolve = done; })); await render(); await act(async () => root.render(null)); await act(async () => resolve(snapshot)); expect(api.close).toHaveBeenCalledWith('live-token'); });
it('keeps the same receiver and video when inactive or showing only chat', async () => {
  await render(); const element = container.querySelector('video');
  await act(async () => root.render(<PlatformLivePlayer reference={reference} title="Title" privacyMode={false} api={api} active={false} videoVisible={false} />));
  expect(container.querySelector('video')).toBe(element); expect(api.open).toHaveBeenCalledTimes(1); expect(api.close).not.toHaveBeenCalled();
  expect(container.querySelector('.platform-live-pair')?.hasAttribute('inert')).toBe(true); expect(container.querySelector('.chat-only')).not.toBeNull();
});
it('shares the live token with bounded chat reads and hides messages in privacy mode', async () => {
  api.chat = vi.fn().mockResolvedValue({items:[{sequence:1,kind:'chat',author:'Example',text:'Public fixture',amount:null,badges:[]}],cursor:1,status:'채팅 연결됨',ended:false,gap:false});
  await render(); expect(api.chat).toHaveBeenCalledWith('live-token',0,120); expect(container.textContent).toContain('Public fixture');
  await render(true); expect(container.textContent).not.toContain('Public fixture'); expect(container.textContent).not.toContain('Example'); expect(api.open).toHaveBeenCalledTimes(1);
});
it('ignores recording replies from a previous session', async () => {
  let resolve!: (value: MediaLiveSnapshot) => void; vi.mocked(api.record).mockReturnValueOnce(new Promise(done => { resolve = done; })); await render();
  await act(async () => container.querySelector('.platform-live-record')!.dispatchEvent(new MouseEvent('click',{bubbles:true})));
  const next = {...reference, externalId:'zyxwvutsrqp'}; vi.mocked(api.open).mockResolvedValue({...snapshot,reference:next,token:'new-token',title:'New channel'});
  await act(async () => root.render(<PlatformLivePlayer reference={next} title="New" privacyMode={false} api={api} />));
  await act(async () => resolve({...snapshot,recordingJob:'old-recording'}));
  expect(container.querySelector('.platform-live-record')?.getAttribute('aria-pressed')).toBe('false'); expect(container.textContent).toContain('New channel');
});
it('keeps chat toggling inside Atsumi without creating another receiver', async () => {
  await render(); const toggle=container.querySelector('[aria-label="채팅 표시"]')!; expect(container.querySelector('.platform-live-chat')).not.toBeNull();
  await act(async () => toggle.dispatchEvent(new MouseEvent('click',{bubbles:true}))); expect(container.querySelector('.platform-live-chat')).toBeNull(); expect(api.open).toHaveBeenCalledTimes(1);
  await act(async () => toggle.dispatchEvent(new MouseEvent('click',{bubbles:true}))); expect(container.querySelector('.platform-live-chat')).not.toBeNull();
});
it('pauses hidden chat IPC without stopping the native session and resumes from its cursor', async () => {
  api.chat=vi.fn().mockResolvedValue({items:[],cursor:10,status:'연결됨',ended:false,gap:false}); await render(); expect(api.chat).toHaveBeenCalledTimes(1);
  await render(true); await act(async () => vi.advanceTimersByTimeAsync(2200)); expect(api.chat).toHaveBeenCalledTimes(1); expect(api.close).not.toHaveBeenCalled();
  await render(false); expect(api.chat).toHaveBeenLastCalledWith('live-token',10,120);
});
it('keeps an operable stacked separator and preserves the receiver through orientation changes', async () => {
  let bounds = new DOMRect(10, 20, 480, 305), notify!: ResizeObserverCallback;
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function(this: HTMLElement) { return this.classList.contains('platform-live-pair') ? bounds : new DOMRect(); });
  vi.stubGlobal('ResizeObserver', class { constructor(callback: ResizeObserverCallback) { notify = callback; } observe() {} disconnect() {} });
  await render();
  const video = container.querySelector('video'), pair = container.querySelector<HTMLElement>('.platform-live-pair')!;
  const separator = container.querySelector<HTMLElement>('[role=separator]')!;
  expect(separator.getAttribute('aria-orientation')).toBe('horizontal');
  expect(separator.getAttribute('aria-label')).toBe('채팅 높이');
  const first = Number.parseFloat(pair.style.getPropertyValue('--platform-live-chat-size'));
  await act(async () => separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true })));
  const adjusted = Number.parseFloat(pair.style.getPropertyValue('--platform-live-chat-size'));
  expect(adjusted).toBeCloseTo(first + 20);
  bounds = new DOMRect(10, 20, 1000, 600);
  await act(async () => notify([], {} as ResizeObserver));
  expect(separator.getAttribute('aria-orientation')).toBe('vertical');
  expect(pair.style.getPropertyValue('--platform-live-chat-size')).toBe('240px');
  await act(async () => separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true })));
  expect(pair.style.getPropertyValue('--platform-live-chat-size')).toBe('96px');
  bounds = new DOMRect(10, 20, 480, 600);
  await act(async () => notify([], {} as ResizeObserver));
  expect(Number.parseFloat(pair.style.getPropertyValue('--platform-live-chat-size'))).toBeCloseTo(adjusted / 298 * 593);
  expect(container.querySelector('video')).toBe(video); expect(api.open).toHaveBeenCalledTimes(1); expect(api.close).not.toHaveBeenCalled(); expect(hls.created).toHaveBeenCalledTimes(1);
});
it('resizes by the actual stacked pointer axis without a covering layer or remount', async () => {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(10, 20, 480, 305));
  await render();
  const pair = container.querySelector<HTMLElement>('.platform-live-pair')!, video = container.querySelector('video');
  const separator = container.querySelector<HTMLElement>('[role=separator]')!;
  let captured = false;
  separator.setPointerCapture = vi.fn(() => { captured = true; }); separator.hasPointerCapture = vi.fn(() => captured); separator.releasePointerCapture = vi.fn(() => { captured = false; });
  const pointer = (type: string, values: object) => { const event = new Event(type, { bubbles: true }); Object.assign(event, { pointerId: 1, button: 0, ...values }); return event; };
  await act(async () => separator.dispatchEvent(pointer('pointerdown', {})));
  await act(async () => separator.dispatchEvent(pointer('pointermove', { clientX: 200, clientY: 180 })));
  expect(Number.parseFloat(pair.style.getPropertyValue('--platform-live-chat-size'))).toBeCloseTo(141.5);
  await act(async () => separator.dispatchEvent(pointer('pointercancel', {})));
  expect(captured).toBe(false);
  expect(container.querySelector('video')).toBe(video); expect(api.open).toHaveBeenCalledTimes(1);
  expect(pair.children).toHaveLength(3);
});
it('does not resize an inactive player by keyboard', async () => {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 1000, 600));
  await act(async () => root.render(<PlatformLivePlayer reference={reference} title="Title" privacyMode={false} api={api} active={false} />));
  const pair=container.querySelector<HTMLElement>('.platform-live-pair')!, separator=container.querySelector<HTMLElement>('[role=separator]')!;
  await act(async () => separator.dispatchEvent(new KeyboardEvent('keydown', { key:'Home', bubbles:true })));
  expect(pair.style.getPropertyValue('--platform-live-chat-size')).toBe('240px'); expect(separator.tabIndex).toBe(-1);
});
it('keeps each multi-view pair resize independent without reopening either receiver', async () => {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 480, 305));
  const second = { ...reference, externalId: 'zyxwvutsrqp' };
  vi.mocked(api.open).mockImplementation(async item => ({ ...snapshot, reference: item, token: item.externalId }));
  await act(async () => root.render(<><PlatformLivePlayer reference={reference} title="First" privacyMode={false} api={api} /><PlatformLivePlayer reference={second} title="Second" privacyMode={false} api={api} /></>));
  const pairs = [...container.querySelectorAll<HTMLElement>('.platform-live-pair')];
  const videos = [...container.querySelectorAll('video')];
  expect(pairs).toHaveLength(2);
  const first = pairs[0]!, other = pairs[1]!;
  const originalSecond = other.style.getPropertyValue('--platform-live-chat-size');
  await act(async () => first.querySelector('[role=separator]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true })));
  expect(first.style.getPropertyValue('--platform-live-chat-size')).toBe('96px');
  expect(other.style.getPropertyValue('--platform-live-chat-size')).toBe(originalSecond);
  expect([...container.querySelectorAll('video')]).toEqual(videos);
  expect(api.open).toHaveBeenCalledTimes(2); expect(api.close).not.toHaveBeenCalled(); expect(hls.created).toHaveBeenCalledTimes(2);
});

const portraitFixture = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';
const identityHeader = () => container.querySelector('.platform-live-player>header')!;
it('shows verified channel and title with the cached portrait even when chat is closed', async () => {
  api.liveProfile = vi.fn().mockResolvedValue(portraitFixture);
  await render(); const video = container.querySelector('video'), avatar = container.querySelector('.platform-live-avatar');
  expect(api.liveProfile).toHaveBeenCalledExactlyOnceWith(snapshot.token);
  expect(identityHeader().textContent).toContain('Channel'); expect(identityHeader().textContent).toContain('Live title');
  expect(avatar?.querySelector('img')?.getAttribute('src')).toBe(portraitFixture);
  expect(avatar?.getAttribute('aria-hidden')).toBe('true'); expect(avatar?.querySelector('img')?.getAttribute('alt')).toBe('');
  await act(async () => container.querySelector('[aria-label="채팅 표시"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  await act(async () => vi.advanceTimersByTimeAsync(3100));
  expect(container.querySelector('.platform-live-chat')).toBeNull(); expect(identityHeader().textContent).toContain('Channel');
  expect(container.querySelector('.platform-live-avatar')).toBe(avatar); expect(container.querySelector('video')).toBe(video);
  expect(api.liveProfile).toHaveBeenCalledTimes(1); expect(api.open).toHaveBeenCalledTimes(1); expect(hls.created).toHaveBeenCalledTimes(1);
  await render(true); expect(container.querySelector('.platform-live-identity')).toBeNull();
  await render(false); expect(container.querySelector('.platform-live-avatar img')?.getAttribute('src')).toBe(portraitFixture); expect(api.liveProfile).toHaveBeenCalledTimes(1);
});
it.each([['missing', null], ['remote', 'https://example.com/avatar.png'], ['SVG', 'data:image/svg+xml;base64,AAAA'], ['oversized', `data:image/png;base64,${'A'.repeat(180_000)}`]])('keeps a neutral fixed slot for %s portrait', async (_, value) => {
  api.liveProfile = vi.fn().mockResolvedValue(value); await render();
  expect(container.querySelector('.platform-live-avatar svg')).not.toBeNull(); expect(container.querySelector('.platform-live-avatar img')).toBeNull();
  expect(identityHeader().textContent).toContain('Channel');
  await act(async () => vi.advanceTimersByTimeAsync(2100)); expect(api.liveProfile).toHaveBeenCalledTimes(1);
});
it('keeps profile failure and image decode failure local to the avatar without restarting media', async () => {
  api.liveProfile = vi.fn().mockRejectedValueOnce(new Error('profile unavailable')); await render();
  expect(container.querySelector('.platform-live-avatar svg')).not.toBeNull(); expect(container.querySelector('[role=alert]')).toBeNull();
  await render(true); await render(false); expect(api.liveProfile).toHaveBeenCalledTimes(1);
  const second = { ...reference, externalId: 'zyxwvutsrqp' };
  vi.mocked(api.open).mockResolvedValue({ ...snapshot, reference: second, token: 'next-token' }); vi.mocked(api.liveProfile).mockResolvedValue(portraitFixture);
  await act(async () => root.render(<PlatformLivePlayer reference={second} title="Next" privacyMode={false} api={api} />));
  const slot = container.querySelector('.platform-live-avatar');
  await act(async () => slot!.querySelector('img')!.dispatchEvent(new Event('error')));
  expect(container.querySelector('.platform-live-avatar')).toBe(slot); expect(slot?.querySelector('svg')).not.toBeNull();
  expect(container.querySelector('[role=alert]')).toBeNull(); expect(api.open).toHaveBeenCalledTimes(2);
});
it('does not acquire portraits before playing, while inactive, in privacy or in chat-only mode', async () => {
  api.liveProfile = vi.fn().mockResolvedValue(portraitFixture);
  vi.mocked(api.open).mockResolvedValue({ ...snapshot, state: 'resolving' }); await render(); expect(api.liveProfile).not.toHaveBeenCalled();
  await render(true); await act(async () => vi.advanceTimersByTimeAsync(1000)); expect(api.liveProfile).not.toHaveBeenCalled();
  await act(async () => root.render(<PlatformLivePlayer reference={reference} title="Title" privacyMode={false} api={api} active={false} />));
  expect(api.liveProfile).not.toHaveBeenCalled(); expect(container.querySelector('.platform-live-identity')).toBeNull();
  await act(async () => root.render(<PlatformLivePlayer reference={reference} title="Title" privacyMode={false} api={api} videoVisible={false} />));
  expect(api.liveProfile).not.toHaveBeenCalled(); expect(container.querySelector('.platform-live-identity')).toBeNull();
  await render(); expect(api.liveProfile).toHaveBeenCalledExactlyOnceWith(snapshot.token); expect(api.open).toHaveBeenCalledTimes(1);
});
it.each(['private', 'inactive', 'chat-only', 'unmounted'] as const)('discards late portrait replies after becoming %s', async mode => {
  let resolve!: (value: string | null) => void;
  api.liveProfile = vi.fn().mockReturnValueOnce(new Promise(done => { resolve = done; })).mockResolvedValue(null);
  await render();
  await act(async () => root.render(mode === 'unmounted' ? null : <PlatformLivePlayer reference={reference} title="Title" privacyMode={mode === 'private'} active={mode !== 'inactive'} videoVisible={mode !== 'chat-only'} api={api} />));
  await act(async () => resolve(portraitFixture)); expect(container.querySelector('.platform-live-avatar img')).toBeNull();
  if (mode !== 'unmounted') {
    await render(); expect(container.querySelector('.platform-live-avatar img')).toBeNull(); expect(api.liveProfile).toHaveBeenCalledTimes(2); expect(api.open).toHaveBeenCalledTimes(1);
  }
});
it('does not display or cache a former identity or its late portrait in a new session', async () => {
  let resolve!: (value: string | null) => void, openNext!: (value: MediaLiveSnapshot) => void;
  api.liveProfile = vi.fn().mockReturnValueOnce(new Promise(done => { resolve = done; })).mockResolvedValue(null);
  await render(); const second = { ...reference, externalId: 'zyxwvutsrqp' };
  vi.mocked(api.open).mockReturnValueOnce(new Promise(done => { openNext = done; }));
  await act(async () => root.render(<PlatformLivePlayer reference={second} title="Next" privacyMode={false} api={api} />));
  expect(identityHeader().textContent).not.toContain('Channel'); expect(identityHeader().textContent).not.toContain('Live title');
  await act(async () => resolve(portraitFixture)); expect(container.querySelector('.platform-live-avatar img')).toBeNull();
  await act(async () => openNext({ ...snapshot, reference: second, token: 'new-token', channelName: 'Other channel', title: 'Other title' }));
  expect(identityHeader().textContent).toContain('Other channel'); expect(container.querySelector('.platform-live-avatar img')).toBeNull();
  expect(api.liveProfile).toHaveBeenCalledTimes(2); expect(api.liveProfile).toHaveBeenLastCalledWith('new-token');
});
