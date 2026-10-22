import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { YouTubeViewingControls } from './YouTubeViewingControls';
import type { YouTubeApi, YouTubeSearchPage } from './api';
import { targetKey, type YouTubeMetadata, type YouTubeTarget } from './model';
import css from './YouTubeConnection.css?raw';
import type { YouTubeOriginalApi, YouTubeAccountSnapshot } from '../../api/youtubeOriginal';
import type { YouTubeAutoRecordApi, YouTubeAutoSnapshot } from '../../api/youtubeAutoRecord';
import type { YouTubeRecordingObservation } from './YouTubeOriginalPlayer';

const items: YouTubeMetadata[] = ['abcdefghijk', 'bcdefghijkl', 'cdefghijklm', 'defghijklmn', 'efghijklmno'].map((id, index) => ({ kind: 'video', id, url: 'https://www.youtube.com/watch?v=' + id, title: '제목 ' + (index + 1), author: '채널 ' + (index + 1), isLive: true }));
const ids = items.map(targetKey);
let root: Root, container: HTMLDivElement;
const actions = { onApply: vi.fn(), onStop: vi.fn(), onExplore: vi.fn(), onCloseItem: vi.fn(), onFavorite: vi.fn(), onDownload: vi.fn() };
beforeEach(() => { vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); Object.values(actions).forEach(action => action.mockReset()); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
type FixtureProps = { privacy?: boolean; active?: boolean; playing?: string[]; initial?: YouTubeMetadata[]; favorites?: YouTubeMetadata[]; api?: YouTubeApi; accountApi?:Pick<YouTubeOriginalApi,'account'|'login'|'logout'>; autoApi?:YouTubeAutoRecordApi; observations?:readonly YouTubeRecordingObservation[] };
function Fixture({ privacy = false, active = true, playing = [], initial = items, favorites = items, api, accountApi, autoApi, observations }: FixtureProps = {}) { const [open, setOpen] = useState(false); return <YouTubeViewingControls active={active} open={open} onOpenChange={setOpen} privacyMode={privacy} busy={false} items={initial} playingIds={playing} favorites={favorites} favoriteIds={favorites === items ? ids : favorites.map(targetKey)} api={api} accountApi={accountApi} autoApi={autoApi} observations={observations} mode="all" lead={playing[0] ?? ''} {...actions} />; }
const button = (label: string) => [...container.querySelectorAll<HTMLButtonElement>('button')].find(item => item.getAttribute('aria-label') === label || item.textContent === label)!;
const click = async (element: Element, detail = 0) => act(async () => element.dispatchEvent(new MouseEvent('click', { bubbles: true, detail })));
const render = async (props: FixtureProps = {}) => act(async () => root.render(<Fixture {...props} />));
const pointer = async (element: Element, type: string, y: number) => act(async () => { const event = new MouseEvent(type, { bubbles: true, button: 0, clientY: y }); Object.defineProperty(event, 'pointerId', { value: 1 }); element.dispatchEvent(event); });
const input = async (value: string, index = 0) => act(async () => { const element = container.querySelectorAll<HTMLInputElement>('.youtube-channel-input-wrap input')[index]!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(element, value); element.dispatchEvent(new Event('input', { bubbles: true })); });

it('opens/closes by drag without undoing the pull with its generated pointer click', async () => {
  await render(); const handle = container.querySelector('.youtube-bottom-handle')!;
  await pointer(handle, 'pointerdown', 100); await pointer(handle, 'pointermove', 70); await pointer(handle, 'pointerup', 70); await click(handle, 1);
  expect(handle).toHaveAttribute('aria-expanded', 'true');
  await pointer(handle, 'pointerdown', 70); await pointer(handle, 'pointermove', 100); await pointer(handle, 'pointerup', 100); await click(handle, 1);
  expect(handle).toHaveAttribute('aria-expanded', 'false'); expect(container.querySelector('[role=dialog]')).toBeNull();
  expect(actions.onApply).not.toHaveBeenCalled(); expect(actions.onStop).not.toHaveBeenCalled();
});
it('keeps drafts unapplied and returns focus on Escape without leaking shortcuts', async () => {
  await render(); const handle = button('방송 보기 열기'); await click(handle); await input(items[4]!.url);
  expect(actions.onApply).not.toHaveBeenCalled(); const outer = vi.fn(); window.addEventListener('keydown', outer);
  const target = button('선택 보기'); target.focus(); await act(async () => target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  window.removeEventListener('keydown', outer); expect(outer).not.toHaveBeenCalled(); expect(document.activeElement).toBe(handle); expect(container.querySelector('[role=dialog]')).toBeNull();
  await click(handle); expect(container.querySelector<HTMLInputElement>('.youtube-channel-input-wrap input')?.value).toBe(items[4]!.url);
});
it('does not swallow keyboard activation after a cancelled pointer gesture', async () => {
  await render(); const handle = button('방송 보기 열기');
  await pointer(handle, 'pointerdown', 100); await pointer(handle, 'pointercancel', 100); await click(handle);
  expect(handle).toHaveAttribute('aria-expanded', 'true');
});
it('caps draft fields and distinct videos at four, with explicit fifth-favorite feedback', async () => {
  vi.useFakeTimers();
  const api: YouTubeApi = { metadata: vi.fn(async target => items.find(item => item.id === target.id)!), openExternal: vi.fn() };
  await render({ api }); await click(button('방송 보기 열기')); expect(container.querySelectorAll('.youtube-channel-field')).toHaveLength(4);
  expect(button('방송 추가')).toBeDisabled(); await click(button('제목 5 시청 선택'));
  expect(container.querySelector('[role=alert]')?.textContent).toContain('네 개');
  await act(async () => vi.advanceTimersByTimeAsync(301));
  await click(button('시청 시작')); expect(actions.onApply).toHaveBeenCalledWith(items.slice(0, 4).map(targetKey), 'all', targetKey(items[0]!), items.slice(0, 4));
  expect(actions.onStop).not.toHaveBeenCalled(); expect(actions.onDownload).not.toHaveBeenCalled();
});
it('masks private labels and disables revealing/start actions without disabling explicit stop', async () => {
  await render({ privacy: true, playing: [targetKey(items[0]!)] }); await click(button('방송 조작 열기'));
  expect(container.textContent).not.toContain('제목 1'); expect(container.textContent).not.toContain('채널 1');
  expect(container.querySelector<HTMLInputElement>('.youtube-channel-input-wrap input')?.value).toBe('');
  expect(button('변경')).toBeDisabled(); expect(button('탐색에서 영상 찾기')).toBeDisabled();
  expect(container.querySelector('[aria-label="방송 배치"]')).toBeDisabled();
  await click(button('종료')); expect(actions.onStop).toHaveBeenCalledOnce(); expect(actions.onApply).not.toHaveBeenCalled();
});
it('uses a five-line portalled help and removes it on drawer close or inactive navigation', async () => {
  await render(); await click(button('방송 보기 열기')); await click(button('시청 도움말'));
  const help = document.querySelector('.youtube-connection-help-content')!;
  expect(help.querySelectorAll('p')).toHaveLength(5); expect(container.contains(help)).toBe(false);
  expect(help.closest('[data-native-overlay]')).toBeNull();
  expect(help).toHaveAttribute('data-native-dialog-surface', 'true');
  expect(container.querySelector('[role=dialog]')).toHaveAttribute('aria-modal', 'false');
  expect(button('시청 도움말')).toHaveAttribute('aria-describedby', help.id);
  await click(button('연결 설정 닫기')); expect(document.querySelector('.youtube-connection-help-content')).toBeNull();
  await click(button('방송 보기 열기')); await click(button('시청 도움말')); await render({ active: false });
  expect(document.querySelector('.youtube-connection-help-content')).toBeNull();
});
it('offers in-app discovery/saved management and honestly disables unsupported account/reservation', async () => {
  await render(); await click(button('방송 보기 열기'));
  expect(button('로그인')).toBeDisabled(); expect(button('로그아웃')).toBeDisabled(); expect(button('자동 녹화 설정')).toBeUndefined();
  for (const control of container.querySelectorAll('button[aria-label$=" 자동 녹화"]')) expect(control).toBeDisabled();
  await click(button('즐겨찾기 관리')); expect(actions.onExplore).toHaveBeenCalledWith(true);
  await click(button('방송 보기 열기')); await click(button('탐색에서 영상 찾기'));
  expect(actions.onExplore).toHaveBeenLastCalledWith(); expect(actions.onApply).not.toHaveBeenCalled();
});
it('uses only verified native account state and does not call a failed login successful',async()=>{
 const accountApi={account:vi.fn(async()=>({authenticated:false,open:false,error:null})),login:vi.fn(async()=>({authenticated:null,open:true,error:null})),logout:vi.fn(async()=>({authenticated:false,open:false,error:null}))};
 await render({accountApi});await click(button('방송 보기 열기'));expect(container.textContent).toContain('로그아웃됨');expect(button('로그아웃')).toBeDisabled();
 const login=button('로그인'),logout=button('로그아웃');await click(login);expect(accountApi.login).toHaveBeenCalledOnce();expect(container.textContent).toContain('로그인 창에서 확인');expect(button('로그인')).toBe(login);expect(button('로그아웃')).toBe(logout);
 accountApi.login.mockRejectedValueOnce(new Error('Google에서 이 브라우저의 로그인을 허용하지 않았습니다.'));await click(login);expect(container.textContent).toContain('Google에서 이 브라우저의 로그인을 허용하지 않았습니다.');expect(container.textContent).not.toContain('로그인됨');expect(button('로그인')).toBe(login);
});
it('disables authenticated login while preserving logout, busy guards and both button slots',async()=>{
 let finish!: (value:YouTubeAccountSnapshot)=>void;
 const accountApi={account:vi.fn(async()=>({authenticated:true,open:false,error:null})),login:vi.fn(async()=>({authenticated:true,open:false,error:null})),logout:vi.fn(()=>new Promise<YouTubeAccountSnapshot>(resolve=>{finish=resolve;}))};
 await render({accountApi});await click(button('방송 보기 열기'));
 const login=button('로그인'),logout=button('로그아웃');expect(login).toBeDisabled();expect(logout).not.toBeDisabled();
 await click(login);expect(accountApi.login).not.toHaveBeenCalled();
 await click(logout);expect(accountApi.logout).toHaveBeenCalledOnce();expect(login).toBeDisabled();expect(logout).toBeDisabled();
 await act(async()=>finish({authenticated:false,open:false,error:null}));
 expect(button('로그인')).toBe(login);expect(button('로그아웃')).toBe(logout);expect(login).not.toBeDisabled();expect(logout).toBeDisabled();
 await click(login);expect(accountApi.login).toHaveBeenCalledOnce();expect(login).toBeDisabled();expect(logout).not.toBeDisabled();
 await render({accountApi,privacy:true});expect(login).toBeDisabled();expect(logout).toBeDisabled();
});
it('updates login availability from native polling without treating unknown as authenticated',async()=>{
 vi.useFakeTimers();let snapshot:YouTubeAccountSnapshot={authenticated:null,open:true,error:null};
 const accountApi={account:vi.fn(async()=>snapshot),login:vi.fn(),logout:vi.fn()};
 await render({accountApi});await click(button('방송 보기 열기'));
 const login=button('로그인'),logout=button('로그아웃');expect(login).not.toBeDisabled();expect(logout).toBeDisabled();
 snapshot={authenticated:true,open:false,error:null};await act(async()=>vi.advanceTimersByTimeAsync(2000));
 expect(button('로그인')).toBe(login);expect(button('로그아웃')).toBe(logout);expect(login).toBeDisabled();expect(logout).not.toBeDisabled();
 snapshot={authenticated:false,open:false,error:null};await act(async()=>vi.advanceTimersByTimeAsync(2000));
 expect(login).not.toBeDisabled();expect(logout).toBeDisabled();expect(accountApi.login).not.toHaveBeenCalled();expect(accountApi.logout).not.toHaveBeenCalled();
});
it('uses the favorite-row fixed toggle for explicit channel reservations without changing the star or starting playback', async () => {
  const channelId = 'UC4QobU6STFB0P71PMvOGN5A';
  const channel:YouTubeMetadata = { kind:'channel', id:`channel/${channelId}`, url:`https://www.youtube.com/channel/${channelId}`, title:'채널 테스트', author:'' };
  const snapshot = { revision:1, ready:true, error:null, channels:[] };
  const autoApi:YouTubeAutoRecordApi = { snapshot:vi.fn(async()=>snapshot), resolve:vi.fn(async()=>({ channelId,name:channel.title })), set:vi.fn(async():Promise<YouTubeAutoSnapshot>=>({ ...snapshot, revision:2, channels:[{ channelId,name:channel.title,enabled:true,status:'idle',liveStatus:'offline',jobId:null,checkedAt:null,error:null,recordingCount:0 }] })) };
  await render({ favorites:[channel],initial:[],autoApi }); expect(autoApi.snapshot).not.toHaveBeenCalled();
  await click(button('방송 보기 열기')); const control=button('채널 테스트 자동 녹화'),star=button('채널 테스트 즐겨찾기 해제');
  expect(control).not.toBeDisabled(); expect(control).toHaveAttribute('aria-pressed','false');
  await click(control); expect(autoApi.set).toHaveBeenCalledExactlyOnceWith(channelId,channel.title,true,1);
  expect(button('채널 테스트 자동 녹화')).toBe(control); expect(button('채널 테스트 즐겨찾기 해제')).toBe(star);
  expect(control).toHaveAttribute('aria-pressed','true'); expect(container.textContent).toContain('녹화 예약 됨');
  expect(actions.onApply).not.toHaveBeenCalled(); expect(actions.onFavorite).not.toHaveBeenCalled();
});
it('separates current live and recording colors while retaining fixed favorite and reservation controls', async () => {
  vi.useFakeTimers();
  const channelId = 'UC83RKJs4eKHVE9v0YUyqzgg';
  const channel: YouTubeMetadata = { kind: 'channel', id: `channel/${channelId}`, url: `https://www.youtube.com/channel/${channelId}`, title: 'Status channel', author: '' };
  let state: YouTubeAutoSnapshot = { revision: 1, ready: true, error: null, channels: [{ channelId, name: channel.title, enabled: true, liveStatus: 'live', status: 'idle', jobId: null, checkedAt: null, error: null, recordingCount: 0 }] };
  const autoApi: YouTubeAutoRecordApi = { snapshot: vi.fn(async () => structuredClone(state)), resolve: vi.fn(), set: vi.fn() };
  const props = { favorites: [channel], initial: [], autoApi };
  await render(props); await click(button('방송 보기 열기'));
  const card = container.querySelector('.youtube-channel-card')!, star = button('Status channel 즐겨찾기 해제'), toggle = button('Status channel 자동 녹화');
  const presence = () => card.querySelector('.youtube-channel-presence')!, recording = () => card.querySelector('.youtube-channel-record-status')!;
  expect(presence()).toHaveClass('is-live'); expect(presence()).toHaveTextContent('● 방송 중');
  expect(recording()).toHaveTextContent('녹화 예약 됨'); expect(recording()).not.toHaveClass('is-recording'); expect(toggle).toHaveAttribute('data-reserved', 'true');
  for (const [status, liveStatus, enabled, count, text, tone] of [
    ['recording', 'live', true, 2, '2개 녹화 중', 'is-recording'],
    ['stopping', 'offline', false, 1, '마무리 중', ''],
    ['failed', 'unknown', true, 0, '확인 필요', 'is-problem'],
    ['starting', 'unknown', true, 0, '녹화 준비', ''],
    ['waiting', 'live', true, 0, '자리 대기', ''],
    ['skipped', 'live', true, 0, '이번 방송 건너뜀', ''],
    ['idle', 'offline', false, 0, '예약 꺼짐', ''],
  ] as const) {
    state = { ...state, channels: [{ ...state.channels[0]!, status, liveStatus, enabled, recordingCount: count }] };
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container.querySelector('.youtube-channel-card')).toBe(card); expect(button('Status channel 즐겨찾기 해제')).toBe(star); expect(button('Status channel 자동 녹화')).toBe(toggle);
    expect(presence()).toHaveClass(`is-${liveStatus}`); expect(recording()).toHaveTextContent(text);
    expect(recording().classList.contains('is-recording')).toBe(tone === 'is-recording'); expect(recording().classList.contains('is-problem')).toBe(tone === 'is-problem');
    expect(toggle).toHaveAttribute('aria-pressed', String(enabled)); expect(toggle).toHaveAttribute('data-reserved', String(enabled));
  }
  state = { ...state, channels: [{ ...state.channels[0]!, liveStatus: 'live', status: 'recording', enabled: true, recordingCount: 1 }] };
  await act(async () => vi.advanceTimersByTimeAsync(2000)); await render({ ...props, privacy: true });
  expect(container.textContent).not.toContain('Status channel'); expect(container.textContent).not.toContain('방송 중'); expect(container.textContent).not.toContain('녹화 중');
  expect(card.querySelector('.is-live,.is-recording,[data-reserved=true]')).toBeNull();
  expect(actions.onFavorite).not.toHaveBeenCalled(); expect(actions.onApply).not.toHaveBeenCalled(); expect(autoApi.set).not.toHaveBeenCalled();
});
it('does not color a cached saved live video as current channel presence', async () => {
  const channel: YouTubeMetadata = { kind: 'channel', id: '@unobserved', url: 'https://www.youtube.com/@unobserved', title: 'Unobserved', author: '' };
  await render({ initial: [], favorites: [items[0]!, channel] }); await click(button('방송 보기 열기'));
  const cards = container.querySelectorAll('.youtube-channel-card');
  expect(cards[0]?.querySelector('.youtube-channel-presence')).toHaveTextContent('라이브 영상');
  expect(cards[0]?.querySelector('.is-live')).toBeNull();
  expect(cards[1]?.querySelector('.youtube-channel-presence')).toHaveTextContent('방송 상태 미확인');
  expect(cards[1]?.querySelector('.is-live')).toBeNull();
});
it('resolves a channel live-entry URL as a channel favorite without silently starring the current video', async () => {
  vi.useFakeTimers();
  const channel: YouTubeMetadata = { kind: 'channel', id: '@example', url: 'https://www.youtube.com/@example', title: 'Example', author: '' };
  const api: YouTubeApi = { metadata: vi.fn(async target => target.kind === 'channel' ? channel : items[0]!), search: vi.fn(async () => ({ items: [items[0]!] })), openExternal: vi.fn() };
  await render({ initial: [], favorites: [], api }); await click(button('방송 보기 열기')); await input('@example/live');
  await act(async () => vi.advanceTimersByTimeAsync(301));
  expect(api.metadata).toHaveBeenNthCalledWith(1, { kind: 'channel', id: '@example', url: channel.url }, expect.any(AbortSignal));
  expect(api.search).toHaveBeenCalledWith({ query: '', channel: channel.url, filter: 'live', sort: 'relevance' });
  await click(button('방송 1 즐겨찾기 추가')); expect(actions.onFavorite).toHaveBeenCalledExactlyOnceWith(channel);
  expect(actions.onApply).not.toHaveBeenCalled(); expect(api.openExternal).not.toHaveBeenCalled();
  await click(button('시청 시작')); expect(actions.onApply).toHaveBeenCalledExactlyOnceWith([targetKey(items[0]!)], 'all', targetKey(items[0]!), [items[0]!]);
});
it('resolves pasted channels to an actual live video without autoplay, then applies only on confirmation', async () => {
  vi.useFakeTimers();
  const channel: YouTubeMetadata = { kind: 'channel', id: '@example', url: 'https://www.youtube.com/@example', title: 'Example', author: '' };
  const api: YouTubeApi = { metadata: vi.fn(async target => target.kind === 'channel' ? channel : items[0]!), search: vi.fn(async () => ({ items: [items[0]!] })), openExternal: vi.fn() };
  await render({ initial: [], api }); await click(button('방송 보기 열기')); await input('@example');
  expect(button('시청 시작')).toBeDisabled(); await act(async () => vi.advanceTimersByTimeAsync(301));
  expect(api.metadata).toHaveBeenCalledTimes(2); expect(api.metadata).toHaveBeenLastCalledWith(items[0], expect.any(AbortSignal)); expect(api.search).toHaveBeenCalledWith({ query: '', channel: channel.url, filter: 'live', sort: 'relevance' });
  expect(actions.onApply).not.toHaveBeenCalled(); expect(button('시청 시작')).not.toBeDisabled();
  await click(button('시청 시작')); expect(actions.onApply).toHaveBeenCalledWith([targetKey(items[0]!)], 'all', targetKey(items[0]!), [items[0]!]); expect(api.openExternal).not.toHaveBeenCalled();
});
it('does not restart slow channel resolution or flicker resolved rows on fresh equal-revision reservation polls', async () => {
  vi.useFakeTimers();
  const channel: YouTubeMetadata = { kind: 'channel', id: '@slow', url: 'https://www.youtube.com/@slow', title: 'Slow channel', author: '' };
  const channelId = 'UC4QobU6STFB0P71PMvOGN5A';
  let recording = false;
  const autoApi: YouTubeAutoRecordApi = {
    snapshot: vi.fn(async (): Promise<YouTubeAutoSnapshot> => ({ revision: 7, ready: true, error: null, channels: [{ channelId, name: '예약 채널', enabled: true, status: recording ? 'recording' : 'idle', liveStatus: 'live', jobId: null, checkedAt: null, error: null, recordingCount: recording ? 1 : 0 }] })),
    resolve: vi.fn(), set: vi.fn(),
  };
  const api: YouTubeApi = {
    metadata: vi.fn((target: YouTubeTarget) => new Promise<YouTubeMetadata>(resolve => setTimeout(() => resolve(target.kind === 'channel' ? channel : items[0]!), target.kind === 'channel' ? 2400 : 1800))),
    search: vi.fn(() => new Promise<YouTubeSearchPage>(resolve => setTimeout(() => resolve({ items: [items[0]!] }), 600))), openExternal: vi.fn(),
  };
  await render({ initial: [], favorites: [], api, autoApi }); await click(button('방송 보기 열기')); await input(channel.url);
  for (let second = 0; second < 6; second++) await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(button('시청 시작')).not.toBeDisabled(); expect(button('방송 1 즐겨찾기 추가')).not.toBeDisabled();
  expect(api.metadata).toHaveBeenCalledTimes(2); expect(api.search).toHaveBeenCalledOnce();
  for (const [, signal] of vi.mocked(api.metadata).mock.calls) expect(signal?.aborted).toBe(false);
  recording = true;
  for (let poll = 0; poll < 3; poll++) {
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(button('시청 시작')).not.toBeDisabled(); expect(container.textContent).not.toContain('확인 중');
  }
  expect(container.textContent).toContain('녹화 중'); expect(autoApi.snapshot).toHaveBeenCalledTimes(7);
  expect(api.metadata).toHaveBeenCalledTimes(2); expect(api.search).toHaveBeenCalledOnce();
  expect(actions.onApply).not.toHaveBeenCalled(); expect(autoApi.set).not.toHaveBeenCalled();
});
it('keeps the ten-second resolution deadline while fresh equal-revision reservation polls continue', async () => {
  vi.useFakeTimers();
  let finish!: (value: YouTubeMetadata) => void;
  const autoApi: YouTubeAutoRecordApi = { snapshot: vi.fn(async () => ({ revision: 1, ready: true, error: null, channels: [] })), resolve: vi.fn(), set: vi.fn() };
  const api: YouTubeApi = { metadata: vi.fn(() => new Promise<YouTubeMetadata>(resolve => { finish = resolve; })), search: vi.fn(), openExternal: vi.fn() };
  await render({ initial: [], favorites: [], api, autoApi }); await click(button('방송 보기 열기')); await input('@never');
  await act(async () => vi.advanceTimersByTimeAsync(300));
  for (let second = 0; second < 10; second++) await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(container.textContent).toContain('확인이 지연됩니다. 주소를 다시 입력해 주세요.');
  expect(container.textContent).not.toContain('확인 중'); expect(button('시청 시작')).toBeDisabled();
  expect(api.metadata).toHaveBeenCalledOnce(); expect(vi.mocked(api.metadata).mock.calls[0]?.[1]?.aborted).toBe(true);
  await act(async () => vi.advanceTimersByTimeAsync(4000));
  expect(container.textContent).toContain('확인이 지연됩니다. 주소를 다시 입력해 주세요.');
  expect(api.metadata).toHaveBeenCalledOnce(); expect(autoApi.snapshot).toHaveBeenCalledTimes(8);
  await act(async () => finish({ kind: 'channel', id: '@never', url: 'https://www.youtube.com/@never', title: 'Too late', author: '' }));
  expect(container.textContent).toContain('확인이 지연됩니다. 주소를 다시 입력해 주세요.'); expect(container.textContent).not.toContain('Too late');
  expect(actions.onApply).not.toHaveBeenCalled(); expect(api.search).not.toHaveBeenCalled();
});
it.each(['address', 'close', 'inactive', 'privacy'] as const)('still fences a late response after %s while reservation polling is isolated', async change => {
  vi.useFakeTimers();
  let finish!: (value: YouTubeMetadata) => void;
  const autoApi: YouTubeAutoRecordApi = { snapshot: vi.fn(async () => ({ revision: 1, ready: true, error: null, channels: [] })), resolve: vi.fn(), set: vi.fn() };
  const api: YouTubeApi = {
    metadata: vi.fn((target: YouTubeTarget) => target.kind === 'channel' ? new Promise<YouTubeMetadata>(resolve => { finish = resolve; }) : Promise.resolve(items[1]!)),
    search: vi.fn(async () => ({ items: [items[0]!] })), openExternal: vi.fn(),
  };
  const props: FixtureProps = { initial: [], favorites: [], playing: [], api, autoApi };
  await render(props); await click(button('방송 보기 열기')); await input('@pending');
  await act(async () => vi.advanceTimersByTimeAsync(300));
  await act(async () => vi.advanceTimersByTimeAsync(2000));
  expect(api.metadata).toHaveBeenCalledOnce();
  const signal = vi.mocked(api.metadata).mock.calls[0]?.[1]; expect(signal?.aborted).toBe(false);
  if (change === 'address') await input(items[1]!.url);
  else if (change === 'close') await click(button('연결 설정 닫기'));
  else await render({ ...props, active: change !== 'inactive', privacy: change === 'privacy' });
  expect(signal?.aborted).toBe(true);
  await act(async () => finish({ kind: 'channel', id: '@pending', url: 'https://www.youtube.com/@pending', title: 'Stale channel', author: '' }));
  expect(api.search).not.toHaveBeenCalled(); expect(container.textContent).not.toContain('Stale channel'); expect(actions.onApply).not.toHaveBeenCalled();
  if (change === 'close') await click(button('방송 보기 열기'));
  else if (change !== 'address') await render(props);
  if (change !== 'address') await input(items[1]!.url);
  await act(async () => vi.advanceTimersByTimeAsync(301));
  expect(button('시청 시작')).not.toBeDisabled(); await click(button('시청 시작'));
  expect(actions.onApply).toHaveBeenCalledExactlyOnceWith([targetKey(items[1]!)], 'all', targetKey(items[1]!), [items[1]!]);
});
it('uses the newest lookup seed on the next input without restarting the unchanged one', async () => {
  vi.useFakeTimers();
  const channel: YouTubeMetadata = { kind: 'channel', id: '@new', url: 'https://www.youtube.com/@new', title: 'New favorite', author: '' };
  const api: YouTubeApi = { metadata: vi.fn(async target => items.find(item => item.id === target.id)!), search: vi.fn(async () => ({ items: [items[1]!] })), openExternal: vi.fn() };
  const props: FixtureProps = { initial: [], favorites: [], playing: [], api };
  await render(props); await click(button('방송 보기 열기')); await input(items[0]!.url); await act(async () => vi.advanceTimersByTimeAsync(301));
  expect(button('시청 시작')).not.toBeDisabled(); expect(api.metadata).toHaveBeenCalledOnce();
  await render({ ...props, favorites: [channel] });
  expect(button('시청 시작')).not.toBeDisabled(); expect(api.metadata).toHaveBeenCalledOnce();
  await input(channel.url); await act(async () => vi.advanceTimersByTimeAsync(301));
  expect(api.metadata).toHaveBeenCalledTimes(2); expect(api.metadata).toHaveBeenLastCalledWith(items[1], expect.any(AbortSignal));
  expect(api.search).toHaveBeenCalledExactlyOnceWith({ query: '', channel: channel.url, filter: 'live', sort: 'relevance' });
  expect(button('방송 1 즐겨찾기 해제')).not.toBeDisabled(); expect(button('시청 시작')).not.toBeDisabled();
});
it('rejects no-live/ambiguous results and fences a late response after close and replacement', async () => {
  vi.useFakeTimers(); let finish!: (value: YouTubeMetadata) => void;
  const api: YouTubeApi = { metadata: vi.fn(() => new Promise<YouTubeMetadata>(resolve => { finish = resolve; })), search: vi.fn(async () => ({ items: [] })), openExternal: vi.fn() };
  await render({ initial: [], api }); await click(button('방송 보기 열기')); await input('https://www.youtube.com/@pending'); await act(async () => vi.advanceTimersByTimeAsync(301));
  await click(button('연결 설정 닫기')); await act(async () => finish({ kind: 'channel', id: '@pending', url: 'https://www.youtube.com/@pending', title: 'Stale', author: '' }));
  expect(actions.onApply).not.toHaveBeenCalled();
  vi.mocked(api.metadata).mockResolvedValue(items[1]!);
  await click(button('방송 보기 열기')); await input(items[1]!.url); expect(button('시청 시작')).toBeDisabled();
  await act(async () => vi.advanceTimersByTimeAsync(301)); expect(button('시청 시작')).not.toBeDisabled();
  await click(button('시청 시작')); expect(actions.onApply).toHaveBeenCalledWith([targetKey(items[1]!)], 'all', targetKey(items[1]!), [items[1]!]);
});
it('matches copied drawer geometry and keeps input/favorite hit targets in fixed slots', () => {
  expect(css).toContain('width:min(760px,calc(100% - 32px))'); expect(css).toContain('width:72px;height:22px');
  expect(css).toContain('bottom:100%'); expect(css).toContain('grid-auto-rows:var(--youtube-channel-row-height)');
  expect(css).toContain('--youtube-channel-row-height:88px'); expect(css).toContain('grid-template-columns:24px clamp(88px,22%,152px) minmax(0,1fr) 36px');
  expect(css).not.toContain('.connection-setup'); expect(css).not.toContain('data-native-overlay');
});
it('shows exact manual video/canonical/handle observations without enabling a reservation or resolving aliases',async()=>{
 const canonicalChannelId='UCabcdefghijklmnopqrstuv';
 const channel:YouTubeMetadata={kind:'channel',id:'channel/'+canonicalChannelId,url:'https://www.youtube.com/channel/'+canonicalChannelId,title:'같은 이름',author:''};
 const handle:YouTubeMetadata={kind:'channel',id:'@current',url:'https://www.youtube.com/@current',title:'같은 이름',author:''};
 const other:YouTubeMetadata={...handle,id:'@other',url:'https://www.youtube.com/@other'};
 const video={...items[0]!,authorUrl:handle.url};
 const autoApi:YouTubeAutoRecordApi={snapshot:vi.fn(async()=>({revision:1,ready:true,error:null,channels:[]})),resolve:vi.fn(),set:vi.fn()};
 const observation:YouTubeRecordingObservation={videoId:video.id,token:'lease',canonicalChannelId,state:'playing',recordingJob:'manual',recordingStopping:false};
 const props:FixtureProps={initial:[video],playing:[targetKey(video)],favorites:[video,channel,handle,other],autoApi,observations:[observation]};
 await render(props);await click(button('방송 조작 열기'));
 expect(container.querySelectorAll('.youtube-channel-record-status.is-recording')).toHaveLength(3);
 expect(container.querySelectorAll('.youtube-channel-presence.is-live')).toHaveLength(3);
 expect(container.querySelectorAll('.youtube-channel-auto[aria-pressed="true"]')).toHaveLength(0);
 expect(autoApi.resolve).not.toHaveBeenCalled();expect(autoApi.set).not.toHaveBeenCalled();
 await render({...props,observations:[{...observation,recordingStopping:true}]});
 expect(container.querySelectorAll('.is-recording')).toHaveLength(0);expect([...container.querySelectorAll('.youtube-channel-record-status')].filter(node=>node.textContent==='마무리 중')).toHaveLength(3);
 await render({...props,observations:[]});expect(container.querySelectorAll('.youtube-channel-presence.is-live,.is-recording')).toHaveLength(0);
 await render({...props,privacy:true});expect(container.querySelectorAll('.youtube-channel-presence.is-live,.is-recording')).toHaveLength(0);expect(container.textContent).not.toContain('같은 이름');
});
it('never upgrades cached or missing-current-descriptor video favorites to green',async()=>{
 const observation:YouTubeRecordingObservation={videoId:items[0]!.id,token:'lease',canonicalChannelId:null,state:'playing',recordingJob:null,recordingStopping:false};
 const props:FixtureProps={initial:[items[0]!],playing:[ids[0]!],favorites:[items[0]!],observations:[observation]};
 await render(props);await click(button('방송 조작 열기'));expect(container.querySelector('.youtube-channel-presence')).toHaveTextContent('라이브 영상');expect(container.querySelector('.is-live')).toBeNull();
 for(const state of ['loading','ended','error']){await render({...props,observations:[{...observation,canonicalChannelId:'UCabcdefghijklmnopqrstuv',state}]});expect(container.querySelector('.youtube-channel-presence.is-live')).toBeNull();}
 await render({...props,playing:[],observations:[{...observation,canonicalChannelId:'UCabcdefghijklmnopqrstuv',recordingJob:'stale'}]});expect(container.querySelector('.is-live,.is-recording')).toBeNull();
});
it.each([
 ['recording',false,'녹화 중',true],['recording',true,'2개 녹화 중',true],['stopping',false,'녹화 중',true],['failed',true,'마무리 중',false],['idle',false,'녹화 중',true],
] as const)('combines auto %s with observed stopping=%s without duplicate counting',async(status,recordingStopping,label,red)=>{
 const canonicalChannelId='UCabcdefghijklmnopqrstuv';const channel:YouTubeMetadata={kind:'channel',id:'channel/'+canonicalChannelId,url:'https://www.youtube.com/channel/'+canonicalChannelId,title:'Channel',author:''};
 const autoApi:YouTubeAutoRecordApi={snapshot:vi.fn(async():Promise<YouTubeAutoSnapshot>=>({revision:1,ready:true,error:null,channels:[{channelId:canonicalChannelId,name:'Channel',enabled:true,status,liveStatus:'live',jobId:'same-job',checkedAt:null,error:null,recordingCount:2}]})),resolve:vi.fn(),set:vi.fn()};
 await render({initial:[items[0]!],playing:[ids[0]!],favorites:[channel],autoApi,observations:[{videoId:items[0]!.id,token:'lease',canonicalChannelId,state:'playing',recordingJob:'same-job',recordingStopping}]});await click(button('방송 조작 열기'));
 const node=container.querySelector('.youtube-channel-record-status')!;expect(node).toHaveTextContent(label);expect(node.classList.contains('is-recording')).toBe(red);expect(node).not.toHaveClass('is-problem');expect(container.textContent).not.toContain('3개 녹화');
 expect(autoApi.resolve).not.toHaveBeenCalled();expect(autoApi.set).not.toHaveBeenCalled();
});
