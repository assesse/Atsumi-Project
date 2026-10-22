import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { YouTubeWorkspace, type YouTubeWorkspaceProps } from './YouTubeWorkspace';
import { parseYouTubeTarget, saveYouTubeFavorites, YOUTUBE_FAVORITES_KEY, type YouTubeMetadata } from './model';
import type { YouTubeApi } from './api';
import type { PlatformMediaApi } from '../platforms/platformMedia';
import layout from './YouTubeMultiView.css?raw';
import type { YouTubeViewerObservation } from './YouTubeOriginalPlayer';
const playerLifecycle = vi.hoisted(() => ({ open: vi.fn(), close: vi.fn(), actions: new Map<string, () => void>(), resumes: new Map<string, number>(), observations:new Map<string,(event:YouTubeViewerObservation)=>void>() }));
vi.mock('./YouTubeOriginalPlayer', async () => {
  const { useEffect } = await import('react');
  return { YouTubeOriginalPlayer: ({ channelId, active, videoVisible, playbackKind='live', onClose, onObservation, resumeRequest=0 }: { channelId: string; active: boolean; videoVisible: boolean; playbackKind?:string; onClose?:()=>void; onObservation?:(event:YouTubeViewerObservation)=>void; resumeRequest?:number }) => {
    if(onClose) playerLifecycle.actions.set(channelId,onClose); playerLifecycle.resumes.set(channelId,resumeRequest);
    if(onObservation)playerLifecycle.observations.set(channelId,onObservation);
    useEffect(() => { playerLifecycle.open(channelId); return () => playerLifecycle.close(channelId); }, [channelId]);
    return <div className="platform-live-pair" data-native-id={channelId} data-playback-kind={playbackKind} data-active={active} data-video-visible={videoVisible}><video />{playbackKind==='live'?<aside data-chat-for={channelId} />:null}</div>;
  } };
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../platforms/PlatformScreenshotLibrary', () => ({ PlatformScreenshotLibrary: ({ provider, active }: { provider: string; active: boolean }) => <section data-testid="screenshot-library" data-provider={provider} hidden={!active} /> }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
const video: YouTubeMetadata = { ...parseYouTubeTarget('M7lc1UVf-VE')!, title: 'Public | 공개 영상', author: 'Google for Developers', authorUrl: 'https://www.youtube.com/@GoogleDevelopers', thumbnail: 'data:image/jpeg;base64,AA==' };
const second: YouTubeMetadata = { ...parseYouTubeTarget('jNQXAC9IVRw')!, title: '두 번째 영상', author: 'Sample' };
const channel: YouTubeMetadata = { ...parseYouTubeTarget(video.authorUrl!)!, title: video.author, author: '' };
let root: Root, container: HTMLDivElement, api: YouTubeApi, media: PlatformMediaApi;
beforeEach(()=>{playerLifecycle.actions.clear();playerLifecycle.resumes.clear();playerLifecycle.observations.clear();});
beforeEach(() => { playerLifecycle.open.mockClear(); playerLifecycle.close.mockClear(); localStorage.clear(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); container = document.createElement('div'); document.body.append(container); root = createRoot(container); api = { metadata: vi.fn(async target => {
  const response = await vi.mocked(api.search!).mock.results.at(-1)?.value;
  const item = response?.items?.find((value: YouTubeMetadata) => value.id === target.id) ?? [video, second, channel].find(value => value.id === target.id) ?? { ...target, title: target.id, author: '' };
  return { ...item, ...target, isLive: item.isLive ?? false };
}), search: vi.fn().mockResolvedValue({ items: [video, second] }), thumbnail: vi.fn().mockResolvedValue(null), openExternal: vi.fn().mockResolvedValue(undefined) }; media = { download: vi.fn(), downloadBatch: vi.fn().mockResolvedValue([]), jobs: vi.fn().mockResolvedValue([]), library: vi.fn().mockResolvedValue({ items: [] }), cancel: vi.fn(), retry: vi.fn(), open: vi.fn(), close: vi.fn(), openFolder: vi.fn(), delete: vi.fn() }; });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const button = (label: string) => [...container.querySelectorAll<HTMLButtonElement>('button')].find(item => item.getAttribute('aria-label') === label || item.textContent === label)!;
async function render(patch: Partial<YouTubeWorkspaceProps> = {}) { await act(async () => root.render(<YouTubeWorkspace active runtime="browser-mock" privacyMode={false} view="explore" api={api} mediaApi={media} {...patch} />)); }
async function search(value = 'NASA') { await act(async () => { const input = container.querySelector('input')!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value); input.dispatchEvent(new Event('input', { bubbles: true })); }); await act(async () => container.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))); }
async function click(element: Element, modifiers: MouseEventInit = {}) { await act(async () => element.dispatchEvent(new MouseEvent('click', { bubbles: true, ...modifiers }))); }
async function draftInput(value: string, index: number) { await act(async () => { const input = container.querySelectorAll<HTMLInputElement>('.youtube-channel-input-wrap input')[index]!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value); input.dispatchEvent(new Event('input', { bubbles: true })); }); }
async function detail() { await act(async () => container.querySelector('.youtube-card')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))); }

describe('YouTube app discovery', () => {
  it('fences viewer recording updates and clears by exact mounted attempt while preserving replacement',async()=>{
    const live={...video,isLive:true};saveYouTubeFavorites(localStorage,[{...live,savedAt:1}]);
    vi.mocked(api.search!).mockResolvedValue({items:[live]});await render();await search();await detail();await click(button('영상 재생'));
    const emit=playerLifecycle.observations.get(video.id)!;
    const a={instanceId:'a',attempt:1,videoId:video.id};
    const value={videoId:video.id,token:'lease-a',canonicalChannelId:'UCabcdefghijklmnopqrstuv',state:'playing',recordingJob:'manual-a',recordingStopping:false};
    await act(async()=>{emit({...a,kind:'begin'});emit({...a,kind:'update',value});});
    await render({view:'live'});await click(button('방송 조작 열기'));expect(container.querySelector('.youtube-channel-record-status')).toHaveClass('is-recording');
    await act(async()=>emit({...a,kind:'clear',detached:false}));expect(container.querySelector('.is-recording')).toBeNull();
    await act(async()=>emit({...a,kind:'update',value:{...value,token:'foreign'}}));expect(container.querySelector('.is-recording')).toBeNull();
    await act(async()=>emit({...a,kind:'update',value}));expect(container.querySelector('.is-recording')).not.toBeNull();
    await act(async()=>playerLifecycle.actions.get(video.id)!());
    await act(async()=>emit({...a,kind:'update',value}));expect(container.querySelector('.is-recording')).toBeNull();
    await render();await click(button('영상 재생'));
    const b={...a,instanceId:'b'},next=playerLifecycle.observations.get(video.id)!;
    await act(async()=>{next({...b,kind:'begin'});next({...b,kind:'update',value:{...value,token:'lease-b',recordingJob:'manual-b'}});emit({...a,kind:'clear',detached:true});emit({...a,kind:'update',value});});
    await render({view:'live'});await click(button('방송 조작 열기'));expect(container.querySelector('.is-recording')).not.toBeNull();
    await act(async()=>next({...b,kind:'clear',detached:true}));expect(container.querySelector('.is-recording')).toBeNull();
    await act(async()=>next({...b,kind:'update',value:{...value,token:'lease-b'}}));expect(container.querySelector('.is-recording')).toBeNull();
    expect(playerLifecycle.open).toHaveBeenCalledTimes(2);expect(playerLifecycle.close).toHaveBeenCalledOnce();expect(media.cancel).not.toHaveBeenCalled();
  });
  it('stops queued covers when Explore is hidden and reloads the visible missing cover on return', async () => {
    vi.stubGlobal('IntersectionObserver', undefined);
    const pending: Array<(image: string | null) => void> = [];
    const items = Array.from({ length: 6 }, (_, n) => ({ ...second, id: String(n).padStart(11, '0'), url: `https://www.youtube.com/watch?v=${String(n).padStart(11, '0')}` }));
    vi.mocked(api.search!).mockResolvedValue({ items });
    vi.mocked(api.thumbnail!).mockImplementation(() => new Promise(done => pending.push(done)));
    await render(); await search(); expect(api.thumbnail).toHaveBeenCalledTimes(4);
    await render({ view: 'live' });
    await act(async () => pending.forEach(done => done('data:image/png;base64,AQ==')));
    expect(api.thumbnail).toHaveBeenCalledTimes(4);
    await render(); expect(api.thumbnail).toHaveBeenCalledTimes(6);
    await render({ privacyMode: true });
    await act(async () => pending.slice(4).forEach(done => done('data:image/png;base64,AQ==')));
    expect(container.querySelector('.youtube-card-cover img')).toBeNull();
    expect(api.thumbnail).toHaveBeenCalledTimes(6);
  });
  it('closes only the addressed viewer even when captured close callbacks arrive in reverse order', async () => {
    vi.mocked(api.search!).mockResolvedValue({ items: [{...video,isLive:true},{...second,isLive:true}] });
    await render(); await search(); await detail(); await click(button('영상 재생'));
    await click(button('상세 내리기'));
    await act(async()=>container.querySelectorAll('.youtube-card')[1]!.dispatchEvent(new MouseEvent('dblclick',{bubbles:true})));
    await click(button('영상 재생'));
    const first=container.querySelector(`[data-native-id="${video.id}"]`), closeFirst=playerLifecycle.actions.get(video.id)!, closeSecond=playerLifecycle.actions.get(second.id)!;
    expect(playerLifecycle.open).toHaveBeenCalledTimes(2);
    await act(async()=>closeSecond());
    expect(container.querySelector(`[data-native-id="${video.id}"]`)).toBe(first);
    expect(container.querySelector(`[data-native-id="${second.id}"]`)).toBeNull();
    expect(playerLifecycle.close).toHaveBeenCalledExactlyOnceWith(second.id);
    await act(async()=>{closeFirst();closeSecond();});
    expect(container.querySelectorAll('[data-native-id]')).toHaveLength(0);
    expect(playerLifecycle.open).toHaveBeenCalledTimes(2); expect(playerLifecycle.close).toHaveBeenCalledTimes(2);
    expect(media.cancel).not.toHaveBeenCalled();
  });
  it('requests resume only for an explicit foreground reopen of the retained live', async () => {
    vi.mocked(api.search!).mockResolvedValue({items:[{...video,isLive:true}]});
    await render(); await search(); await detail(); await click(button('영상 재생'));
    const before=playerLifecycle.resumes.get(video.id)??0, retained=container.querySelector('[data-native-id]');
    await click(button('상세 내리기')); await render({active:false}); await render();
    expect(playerLifecycle.resumes.get(video.id)).toBe(before);
    await detail(); expect(playerLifecycle.resumes.get(video.id)).toBe(before+1);
    expect(container.querySelector('[data-native-id]')).toBe(retained);
    expect(playerLifecycle.open).toHaveBeenCalledOnce(); expect(playerLifecycle.close).not.toHaveBeenCalled();
  });
  it('resolves a badge-less current live before enabling actions and selects the live host', async () => {
    let finish!: (value: YouTubeMetadata) => void;
    vi.mocked(api.metadata).mockImplementation(() => new Promise(done => { finish = done; }));
    await render({ runtime: 'tauri' }); await search(); expect(api.metadata).not.toHaveBeenCalled(); await detail();
    expect(button('영상 재생')).toBeDisabled(); expect(button('재생')).toBeDisabled(); expect(button('다운로드')).toBeDisabled();
    expect(container.querySelector('[aria-label="영상 상태 확인 중"]')).not.toBeNull(); expect(playerLifecycle.open).not.toHaveBeenCalled();
    await act(async () => finish({ ...video, isLive: true }));
    expect(button('영상 재생')).not.toBeDisabled(); expect(button('다운로드')).toBeDisabled();
    expect(container.querySelector('.youtube-card-text small')?.textContent).toContain('라이브');
    expect(playerLifecycle.open).not.toHaveBeenCalled(); await click(button('영상 재생'));
    expect(container.querySelector('[data-native-id]')).toHaveAttribute('data-playback-kind', 'live');
    expect(container.querySelector('[data-chat-for]')).not.toBeNull(); expect(media.downloadBatch).not.toHaveBeenCalled();
  });
  it('keeps an unknown or mismatched result retryable instead of opening a VOD', async () => {
    vi.mocked(api.metadata).mockResolvedValueOnce({ ...video, isLive: undefined }).mockResolvedValueOnce({ ...second, isLive: false }).mockResolvedValueOnce({ ...video, isLive: false });
    await render(); await search(); await detail(); expect(button('영상 재생')).toBeDisabled(); expect(button('다시 확인')).toBeDefined();
    await click(button('다시 확인')); expect(container.textContent).toContain('다른 정보'); expect(button('영상 재생')).toBeDisabled();
    await click(button('다시 확인')); expect(button('영상 재생')).not.toBeDisabled(); expect(button('다운로드')).not.toBeDisabled(); expect(playerLifecycle.open).not.toHaveBeenCalled();
  });
  it('ignores a closed detail instance reply after the same video is reopened', async () => {
    const finish: Array<(value: YouTubeMetadata) => void> = [];
    vi.mocked(api.metadata).mockImplementation(() => new Promise(done => { finish.push(done); }));
    await render(); await search(); await detail(); await click(button('상세 탭 닫기')); await detail();
    await act(async () => finish[1]!({ ...video, title: '현재 응답', isLive: true }));
    await act(async () => finish[0]!({ ...video, title: '오래된 응답', isLive: false }));
    expect(container.querySelector('.youtube-detail-actions strong')?.textContent).toBe('현재 응답');
    expect(button('다운로드')).toBeDisabled(); expect(playerLifecycle.open).not.toHaveBeenCalled();
  });
  it.each([{ privacyMode: true }, { active: false }])('invalidates late detail/action response on %j', async patch => {
    let finish!: (value: YouTubeMetadata) => void;
    vi.mocked(api.metadata).mockImplementation(() => new Promise(done => { finish = done; }));
    await render(); await search(); await detail(); await render(patch);
    await act(async () => finish({ ...video, isLive: true })); await render();
    expect(button('영상 재생')).toBeDisabled(); expect(button('다시 확인')).toBeDefined();
    expect(playerLifecycle.open).not.toHaveBeenCalled(); expect(media.downloadBatch).not.toHaveBeenCalled();
  });
  it('continues an explicit expired play recheck once, but not an expired play after close', async () => {
    const now = vi.spyOn(Date, 'now'); now.mockReturnValue(100_000);
    await render({ runtime: 'tauri' }); await search(); await detail(); now.mockReturnValue(131_000);
    let finish!: (value: YouTubeMetadata) => void;
    vi.mocked(api.metadata).mockImplementation(() => new Promise(done => { finish = done; }));
    await click(button('재생')); expect(playerLifecycle.open).not.toHaveBeenCalled();
    await act(async () => finish({ ...video, isLive: true })); expect(playerLifecycle.open).toHaveBeenCalledOnce();
    await click(button('시청 종료')); now.mockReturnValue(162_000); await click(button('재생')); await click(button('상세 탭 닫기'));
    await act(async () => finish({ ...video, isLive: false })); expect(playerLifecycle.open).toHaveBeenCalledOnce(); expect(container.querySelector('[role="dialog"]')).toBeNull();
  });
  it.each(['downloads', 'screenshots', 'live'] as const)('cancels a pending new play on %s navigation while retaining an existing receiver', async view => {
    const now = vi.spyOn(Date, 'now'); now.mockReturnValue(100_000);
    await render({ runtime: 'tauri' }); await search(); await detail(); await click(button('재생'));
    const existing = container.querySelector('[data-native-id]'); await click(button('상세 내리기'));
    await act(async () => container.querySelectorAll('.youtube-card')[1]!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    now.mockReturnValue(131_000);
    let finish!: (value: YouTubeMetadata) => void;
    vi.mocked(api.metadata).mockImplementation(() => new Promise(done => { finish = done; }));
    await click(button('재생')); await render({ runtime: 'tauri', view });
    await act(async () => finish({ ...second, isLive: true }));
    expect(playerLifecycle.open).toHaveBeenCalledOnce(); expect(playerLifecycle.close).not.toHaveBeenCalled();
    expect(container.querySelector('[data-native-id]')).toBe(existing);
    await render({ runtime: 'tauri' }); expect(button('다시 확인')).toBeDefined();
  });
  it.each(['context', 'keyboard', 'selection'])('preflights every %s download before submitting any job', async entry => {
    vi.mocked(api.metadata).mockImplementation(async target => ({ ...(target.id === video.id ? video : second), isLive: target.id === second.id }));
    await render(); await search(); const cards = container.querySelectorAll('.youtube-card');
    if (entry === 'context') {
      await act(async () => cards[1]!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))); await click(container.querySelector('[role="menu"] button:nth-child(3)')!);
    } else {
      await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
      if (entry === 'keyboard') await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true })));
      else await click(container.querySelector('.youtube-selection button')!);
    }
    expect(container.textContent).toContain('라이브를 재생한 뒤 녹화'); expect(media.download).not.toHaveBeenCalled(); expect(media.downloadBatch).not.toHaveBeenCalled();
  });
  it('blocks a pending batch if privacy activates and prevents duplicate in-flight submission', async () => {
    let finish!: (value: YouTubeMetadata) => void;
    vi.mocked(api.metadata).mockImplementation(() => new Promise(done => { finish = done; }));
    await render(); await search(); const card = container.querySelector('.youtube-card')!;
    await click(card); await act(async () => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true })); window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true })); });
    expect(api.metadata).toHaveBeenCalledOnce(); await render({ privacyMode: true });
    await act(async () => finish({ ...video, isLive: false })); expect(media.downloadBatch).not.toHaveBeenCalled();
  });
  it('stops fallback individual enqueues after privacy changes without cancelling the already submitted job', async () => {
    let finish!: () => void;
    media.downloadBatch = undefined;
    vi.mocked(media.download).mockImplementation(() => new Promise(done => { finish = () => done({} as never); }));
    await render(); await search(); const cards = container.querySelectorAll('.youtube-card');
    await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true })));
    expect(media.download).toHaveBeenCalledOnce(); await render({ privacyMode: true }); await act(async () => finish());
    expect(media.download).toHaveBeenCalledOnce(); expect(media.cancel).not.toHaveBeenCalled();
  });
  it('uses native authenticated-profile VOD in the desktop runtime without replacing it on detail minimize',async()=>{
    await render({runtime:'tauri'});await search();await detail();await click(button('영상 재생'));
    const player=container.querySelector('[data-native-id]');expect(player).toHaveAttribute('data-playback-kind','vod');expect(container.querySelector('iframe')).toBeNull();expect(container.querySelector('[data-chat-for]')).toBeNull();
    await click(button('상세 내리기'));expect(container.querySelector('[data-native-id]')).toBe(player);expect(playerLifecycle.close).not.toHaveBeenCalled();
    await render({runtime:'tauri',view:'live'});expect(container.querySelector('[data-native-id]')).toBe(player);expect(playerLifecycle.open).toHaveBeenCalledOnce();
  });
  it('keeps saved categories free of remote controls inside Explore and retains the prior query',async()=>{
    saveYouTubeFavorites(localStorage,[{...video,savedAt:1}]);
    await render();await search('preserved');await click(button('즐겨찾기'));
    expect(container.querySelector('form')).toBeNull();expect(container.querySelector('.youtube-search-tabs')).toBeNull();
    expect(container.querySelectorAll('.youtube-card')).toHaveLength(1);expect(api.search).toHaveBeenCalledTimes(1);
    await click(button('전체 탐색'));expect(container.querySelector<HTMLInputElement>('input')?.value).toBe('preserved');
    expect(container.querySelectorAll('.youtube-card')).toHaveLength(2);expect(api.search).toHaveBeenCalledTimes(1);
  });
  it('sizes previews independently from mounted live players and requests actual channel portraits',async()=>{
    const discovered={...channel,id:'channel/UCLA_DiR1FfKNvjuUpBHmylQ',url:'https://www.youtube.com/channel/UCLA_DiR1FfKNvjuUpBHmylQ'};
    vi.mocked(api.search!).mockResolvedValue({items:[{...video,isLive:true},discovered]});
    vi.mocked(api.thumbnail!).mockResolvedValue('data:image/png;base64,AQ==');
    await render();await search();expect(api.thumbnail).toHaveBeenCalledWith(discovered.id);
    expect(container.querySelector('.youtube-card-cover[data-kind="channel"] img')).not.toBeNull();
    await detail();await click(button('영상 재생'));const player=container.querySelector('[data-native-id]');
    await render({previewWidth:400});expect(container.querySelector<HTMLElement>('.youtube-surface')!.style.getPropertyValue('--youtube-preview-width')).toBe('400px');
    expect(container.querySelector('[data-native-id]')).toBe(player);expect(playerLifecycle.open).toHaveBeenCalledTimes(1);
    await render({previewWidth:9999});expect(container.querySelector<HTMLElement>('.youtube-surface')!.style.getPropertyValue('--youtube-preview-width')).toBe('320px');
    expect(container.querySelector('[data-native-id]')).toBe(player);
  });
  it('starts the official VOD player from the floating preview only when its play button is activated', async () => {
    const onViewChange = vi.fn();
    await render({ onViewChange }); await search(); await detail();
    const preview = container.querySelector('.youtube-detail-preview')!, play = button('영상 재생');
    expect(play).toHaveAttribute('type', 'button'); expect(play).not.toBeDisabled();
    expect(preview.contains(play)).toBe(true); expect(container.querySelector('iframe')).toBeNull();
    await click(preview.querySelector('.youtube-card-cover')!);
    expect(container.querySelector('iframe')).toBeNull();
    await act(async () => { play.click(); play.click(); });
    const frames = container.querySelectorAll('iframe');
    expect(frames).toHaveLength(1); expect(new URL(frames[0]!.src).hostname).toBe('www.youtube.com');
    expect(container.querySelector('[role="dialog"][aria-label="영상 상세"]')?.contains(frames[0]!)).toBe(true);
    expect(container.querySelector('.youtube-detail-preview')).toBeNull();
    expect(container.querySelector('.youtube-player [role="status"]')?.textContent).toContain('플레이어 연결 중');
    expect(onViewChange).not.toHaveBeenCalled(); expect(api.openExternal).not.toHaveBeenCalled();
    expect(media.download).not.toHaveBeenCalled(); expect(media.downloadBatch).not.toHaveBeenCalled();
  });
  it.each(['Enter', ' '])('leaves native %s activation available on the preview play button', async key => {
    await render(); await search(); await detail(); const play = button('영상 재생'); play.focus();
    const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
    await act(async () => play.dispatchEvent(event));
    expect(event.defaultPrevented).toBe(false); expect(document.activeElement).toBe(play);
    // jsdom does not synthesize a button's browser default click from a key event.
    await act(async () => play.click());
    expect(container.querySelectorAll('iframe')).toHaveLength(1); expect(media.downloadBatch).not.toHaveBeenCalled();
  });
  it('keeps one live receiver and no iframe when preview playback is requested twice', async () => {
    vi.mocked(api.search!).mockResolvedValue({ items: [{ ...video, isLive: true }] });
    await render(); await search(); await detail(); const play = button('영상 재생');
    await act(async () => { play.click(); play.click(); });
    const native = container.querySelector('[data-native-id]');
    expect(native).not.toBeNull(); expect(container.querySelector('iframe')).toBeNull();
    expect(playerLifecycle.open).toHaveBeenCalledExactlyOnceWith(video.id); expect(playerLifecycle.close).not.toHaveBeenCalled();
    await click(button('상세 내리기')); await render({ view: 'live' }); await render({ view: 'live', active: false });
    expect(container.querySelector('[data-native-id]')).toBe(native); expect(playerLifecycle.open).toHaveBeenCalledTimes(1);
    expect(media.download).not.toHaveBeenCalled(); expect(media.downloadBatch).not.toHaveBeenCalled();
  });
  it('disables the floating preview entry under privacy without starting media', async () => {
    await render(); await search(); await detail(); await render({ privacyMode: true });
    expect(button('영상 재생')).toBeDisabled(); expect(button('재생')).toBeDisabled();
    await act(async () => button('영상 재생').click());
    expect(container.querySelector('iframe')).toBeNull(); expect(playerLifecycle.open).not.toHaveBeenCalled();
    expect(container.querySelector('.youtube-detail-preview img')).toBeNull();
  });
  it('shows the four-player capacity notice inside the detail and preserves all existing receivers', async () => {
    const items = ['abcdefghijk', 'bcdefghijkl', 'cdefghijklm', 'defghijklmn', 'efghijklmno'].map((id, index) => ({ ...video, id, url: `https://www.youtube.com/watch?v=${id}`, isLive: true, title: `라이브 ${index + 1}` }));
    vi.mocked(api.search!).mockResolvedValue({ items }); await render(); await search();
    const cards = container.querySelectorAll('.youtube-card'); await click(cards[0]!); await click(cards[4]!, { shiftKey: true });
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    for (const item of items) {
      await click([...container.querySelectorAll('.youtube-detail-tabs button')].find(button => button.textContent === item.title)!);
      await click(button('영상 재생'));
    }
    expect(container.querySelectorAll('[data-native-id]')).toHaveLength(4); expect(playerLifecycle.open).toHaveBeenCalledTimes(4);
    expect(playerLifecycle.close).not.toHaveBeenCalled();
    expect(container.querySelector('.youtube-detail [role="alert"]')?.textContent).toContain('동시에 네 개까지');
  });
  it('opens local screenshots without remounting live playback or leaving hidden detail shortcuts active', async () => {
    const live = { ...video, isLive: true };
    vi.mocked(api.search!).mockResolvedValue({ items: [live] });
    await render(); await search(); await detail(); await click(button('재생'));
    const player = container.querySelector('[data-native-id]');
    expect(player).not.toBeNull();
    await render({ view: 'screenshots' });
    expect(container.querySelector('[data-native-id]')).toBe(player);
    expect(player).toHaveAttribute('data-active', 'false');
    expect(container.querySelector('.youtube-workspace')).toHaveAttribute('hidden');
    expect(container.querySelector('.youtube-detail-backdrop')).toHaveAttribute('hidden');
    expect(container.querySelector('.youtube-detail')).not.toHaveAttribute('aria-modal', 'true');
    expect(container.querySelector('.youtube-restore')).toBeNull();
    expect(container.querySelector('[data-testid="screenshot-library"]')).not.toHaveAttribute('hidden');
    expect(container.querySelector('[data-testid="screenshot-library"]')).toHaveAttribute('data-provider', 'youtube');
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(playerLifecycle.close).not.toHaveBeenCalled();
    await render({ view: 'live' });
    expect(container.querySelector('[data-native-id]')).toBe(player);
    expect(player).toHaveAttribute('data-active', 'true');
    expect(container.querySelector('[data-testid="screenshot-library"]')).toHaveAttribute('hidden');
    expect(playerLifecycle.open).toHaveBeenCalledTimes(1);
  });
  it('limits remote search controls to Explore without changing saved favorite categories or the retained query', async () => {
    saveYouTubeFavorites(localStorage, [video, channel].map(item => ({ ...item, savedAt: 1 })));
    await render();
    await act(async () => { const filter = container.querySelector<HTMLSelectElement>('[aria-label="검색 종류"]')!; filter.value = 'video'; filter.dispatchEvent(new Event('change', { bubbles: true })); });
    await search('retained query');
    await act(async () => container.querySelector('input')!.dispatchEvent(new FocusEvent('focusin', { bubbles: true })));
    expect(container.querySelector('.youtube-history')).not.toBeNull();
    await render({ view: 'favorites' });
    expect(container.querySelector('form')).toBeNull();
    expect(container.querySelector('[aria-label="검색 종류"]')).toBeNull();
    expect(container.querySelector('[aria-label="정렬"]')).toBeNull();
    expect(container.querySelector('.youtube-history')).toBeNull();
    expect(container.querySelector('.youtube-search-tabs')).toBeNull();
    expect(container.querySelectorAll('.youtube-card')).toHaveLength(1);
    expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe('공개 영상');
    expect(button('영상')).toHaveAttribute('aria-pressed', 'true');
    await click(button('채널'));
    expect(container.querySelectorAll('.youtube-card')).toHaveLength(1);
    expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe(channel.title);
    expect(button('채널')).toHaveAttribute('aria-pressed', 'true');
    expect(api.search).toHaveBeenCalledTimes(1);
    await render(); await click(button('전체 탐색'));
    expect(container.querySelector<HTMLInputElement>('input')?.value).toBe('retained query');
    expect(container.querySelector<HTMLSelectElement>('[aria-label="검색 종류"]')?.value).toBe('video');
    expect(container.querySelector<HTMLSelectElement>('[aria-label="정렬"]')?.value).toBe('relevance');
    expect(container.querySelector('.youtube-history')).toBeNull();
    expect(container.querySelectorAll('.youtube-card')).toHaveLength(2);
    expect(JSON.parse(localStorage.getItem(YOUTUBE_FAVORITES_KEY)!)).toHaveLength(2);
  });
  it('opens a favorite channel in Explore and reuses its retained tab without losing either scroll position', async () => {
    saveYouTubeFavorites(localStorage, [{ ...video, savedAt: 1 }, { ...channel, savedAt: 1 }]);
    vi.mocked(api.search!).mockImplementation(async request => ({ items: request.channel ? [second] : [video, second] }));
    const onViewChange = vi.fn();
    await render({ onViewChange }); await search();
    const list = container.querySelector<HTMLDivElement>('.youtube-results')!;
    const scroll = async (top: number) => act(async () => { list.scrollTop = top; list.dispatchEvent(new Event('scroll', { bubbles: true })); });
    await scroll(480);
    await render({ view: 'favorites', onViewChange }); await scroll(35);
    await click(button('채널')); await scroll(70); await detail();
    expect(onViewChange).toHaveBeenCalledWith('explore');
    expect(api.search).toHaveBeenLastCalledWith({ query: '', channel: channel.url, filter: 'all', sort: 'relevance' });
    expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe(channel.title);
    await render({ onViewChange });
    expect(container.querySelector('h1')?.textContent).toBe('탐색');
    expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe(second.title);
    expect(container.querySelectorAll('.youtube-search-tabs>div')).toHaveLength(2);
    await scroll(140);
    await click([...container.querySelectorAll('.youtube-search-tabs button')].find(item => item.textContent === 'NASA')!);
    expect(list.scrollTop).toBe(480);
    await render({ view: 'favorites', onViewChange }); expect(list.scrollTop).toBe(70);
    await detail(); await render({ onViewChange });
    expect(api.search).toHaveBeenCalledTimes(2);
    expect(onViewChange).toHaveBeenCalledTimes(4);
    expect(list.scrollTop).toBe(140);
    await render({ view: 'favorites', onViewChange }); expect(list.scrollTop).toBe(70);
    await click(button('영상')); expect(list.scrollTop).toBe(35);
  });
  it.each([false, true])('opens a favorite creator with background=%s while keeping favorites distinct from results', async background => {
    saveYouTubeFavorites(localStorage, [{ ...video, savedAt: 1 }]);
    vi.mocked(api.search!).mockImplementation(async request => ({ items: request.channel ? [second] : [video] }));
    const onViewChange = vi.fn();
    await render({ onViewChange }); await search();
    await render({ view: 'favorites', onViewChange }); onViewChange.mockClear();
    await click(container.querySelector('.youtube-author')!, { ctrlKey: background });
    expect(api.search).toHaveBeenLastCalledWith({ query: '', channel: channel.url, filter: 'all', sort: 'relevance' });
    expect(onViewChange).toHaveBeenCalledTimes(background ? 0 : 1);
    if (!background) expect(onViewChange).toHaveBeenCalledWith('explore');
    expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe('공개 영상');
    expect(container.querySelector('.youtube-search-tabs')).toBeNull();
    expect(container.querySelector('.youtube-card-check')).toBeNull();
    await render({ onViewChange }); if (background) await click(button('전체 탐색'));
    expect(container.querySelectorAll('.youtube-search-tabs>div')).toHaveLength(2);
    expect(container.querySelector('.youtube-search-tabs .is-active')?.textContent).toContain(background ? 'NASA' : channel.title);
    expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe(background ? '공개 영상' : second.title);
    if (background) {
      await click([...container.querySelectorAll('.youtube-search-tabs button')].find(item => item.textContent === channel.title)!);
      expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe(second.title);
    }
    expect(api.search).toHaveBeenCalledTimes(2);
  });
  it('keeps favorite and search scroll positions independent on return', async () => {
    await render(); await search(); await click(container.querySelector('.youtube-favorite')!);
    const list = container.querySelector<HTMLDivElement>('.youtube-results')!;
    await act(async () => { list.scrollTop = 480; list.dispatchEvent(new Event('scroll', { bubbles: true })); });
    await render({ view: 'favorites' }); expect(list.scrollTop).toBe(0);
    await act(async () => { list.scrollTop = 35; list.dispatchEvent(new Event('scroll', { bubbles: true })); });
    await render(); await click(button('전체 탐색')); expect(list.scrollTop).toBe(480);
    await render({ view: 'favorites' }); expect(list.scrollTop).toBe(35);
    await click(button('채널')); expect(list.scrollTop).toBe(0);
    await click(button('영상')); expect(list.scrollTop).toBe(35);
    await render(); await click(button('전체 탐색')); expect(list.scrollTop).toBe(480);
  });
  it('does not switch the active detail when a background detail is closed', async () => {
    const third = { ...video, id: 'aqz-KE-bpKQ', url: 'https://www.youtube.com/watch?v=aqz-KE-bpKQ', title: '세 번째 영상' };
    vi.mocked(api.search!).mockResolvedValue({ items: [video, second, third] });
    await render(); await search(); const cards = container.querySelectorAll('.youtube-card');
    await click(cards[0]!); await click(cards[2]!, { shiftKey: true });
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    await click([...container.querySelectorAll('.youtube-detail-tabs button')].find(item => item.textContent === third.title)!);
    await click(container.querySelector('.youtube-detail-tabs [aria-label="상세 탭 닫기"]')!);
    expect(container.querySelector('.youtube-detail-tabs .is-active')?.textContent).toContain(third.title);
    expect(container.querySelector('.youtube-detail-actions strong')?.textContent).toBe(third.title);
  });
  it('shows the existing player in the viewing route without remounting it', async () => {
    await render(); await search(); await detail(); await click(button('재생')); const frame = container.querySelector('iframe');
    await click(button('상세 내리기')); await render({ view: 'live' });
    expect(container.querySelector('iframe')).toBe(frame);
    expect(container.querySelector<HTMLElement>('.youtube-viewing')?.hidden).toBe(false);
    expect(container.querySelector<HTMLElement>('.youtube-workspace')?.hidden).toBe(true);
    expect(container.querySelector('[role="region"][aria-label="YouTube 시청"]')).not.toBeNull();
    expect(container.querySelector('[aria-modal=true]')).toBeNull();
    await render({ active: false, view: 'live' }); expect(container.querySelector('iframe')).toBe(frame);
    await render(); expect(container.querySelector('iframe')).toBe(frame);
  });
  it('offers an explicit next step when viewing has no running video', async () => {
    const onViewChange = vi.fn(); await render({ view: 'live', onViewChange });
    expect(container.querySelector<HTMLElement>('.youtube-viewing-empty')?.hidden).toBe(false);
    expect(container.querySelector('.youtube-viewing-empty')?.textContent).toContain('아래 손잡이를 누르거나 끌어올려');
    await click(button('방송 보기 열기'));
    expect(container.querySelector('.youtube-connection-panel')).not.toBeNull();
    await click(button('탐색에서 영상 찾기')); expect(onViewChange).toHaveBeenCalledWith('explore');
    expect(container.querySelector('iframe')).toBeNull(); expect(api.openExternal).not.toHaveBeenCalled();
  });
  it('moves viewing actions into a sheet without changing VOD detail or playback identity', async () => {
    await render(); await search(); await detail(); await click(button('재생'));
    const frame = container.querySelector('iframe'), host = frame?.closest('.youtube-playing-cell');
    await render({ view: 'live' });
    expect(container.querySelector('.youtube-detail-tabs')).toBeNull(); expect(container.querySelector('.youtube-detail-actions')).toBeNull();
    await click(button('방송 조작 열기')); expect(container.querySelector('.youtube-connection-panel')).not.toBeNull();
    await click(button('연결 설정 닫기')); expect(container.querySelector('.youtube-connection-panel')).toBeNull();
    expect(document.activeElement).toBe(button('방송 조작 열기'));
    expect(container.querySelector('iframe')).toBe(frame); expect(frame?.closest('.youtube-playing-cell')).toBe(host);
    await render(); expect(container.querySelector('.youtube-detail-actions strong')?.textContent).toBe(video.title); expect(container.querySelector('iframe')).toBe(frame);
  });
  it('applies draft viewing selection without replacing existing VOD iframes or their source', async () => {
    await render(); await search(); const cards = container.querySelectorAll('.youtube-card');
    await click(cards[0]!); await click(cards[1]!, { shiftKey: true });
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    await click(button('재생')); const first = container.querySelector('iframe')!;
    await click([...container.querySelectorAll('.youtube-detail-tabs button')].find(item => item.textContent === second.title)!); await click(button('재생'));
    const frames = [...container.querySelectorAll('iframe')], sources = frames.map(item => item.src);
    await render({ view: 'live' }); await click(button('방송 조작 열기')); await click(button('선택 보기'));
    expect(container.querySelector('.youtube-playing-grid')?.getAttribute('data-mode')).toBe('all');
    const lead = container.querySelector<HTMLSelectElement>('[aria-label="볼 영상"]')!;
    expect([...lead.options].map(option => option.textContent)).toEqual([video.author, second.author]);
    await act(async () => { lead.value = `video:${second.id}`; lead.dispatchEvent(new Event('change', { bubbles: true })); });
    await click(button('변경'));
    expect(container.querySelector('.youtube-playing-grid')?.getAttribute('data-mode')).toBe('selected');
    expect(first.closest('.youtube-playing-cell')?.getAttribute('inert')).not.toBeNull();
    expect([...container.querySelectorAll('iframe')]).toEqual(frames); expect(frames.map(item => item.src)).toEqual(sources);
    await click(button('방송 조작 열기')); await click(button('모두 보기')); await click(button('변경'));
    expect(first.closest('.youtube-playing-cell')?.getAttribute('inert')).toBeNull(); expect([...container.querySelectorAll('iframe')]).toEqual(frames);
  });
  it('closes the viewing sheet on inactive navigation while preserving playback and original search state', async () => {
    await render(); await search('keep this query'); await detail(); await click(button('재생')); const frame = container.querySelector('iframe');
    await render({ view: 'live' }); await click(button('방송 조작 열기')); await render({ view: 'live', active: false });
    expect(container.querySelector('.youtube-connection-panel')).toBeNull(); expect(container.querySelector('iframe')).toBe(frame);
    await render({ view: 'live' }); expect(container.querySelector('.youtube-connection-panel')).toBeNull();
    await render(); expect(container.querySelector<HTMLInputElement>('[aria-label="YouTube 검색"]')?.value).toBe('keep this query'); expect(container.querySelector('iframe')).toBe(frame);
  });
  it('keeps all live receivers and chats when applying2→4 and all→selected→all', async () => {
    const items = ['abcdefghijk', 'bcdefghijkl', 'cdefghijklm', 'defghijklmn'].map((id, index) => ({ ...video, id, url: `https://www.youtube.com/watch?v=${id}`, isLive: true, author: `채널 ${index + 1}`, title: `라이브 ${index + 1}` }));
    vi.mocked(api.search!).mockResolvedValue({ items }); await render(); await search();
    const cards = container.querySelectorAll('.youtube-card'); await click(cards[0]!); await click(cards[3]!, { shiftKey: true });
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    await render({ view: 'live' }); await click(button('방송 보기 열기'));
    await click(button('방송 4 지우기')); await click(button('방송 3 지우기')); expect(playerLifecycle.open).not.toHaveBeenCalled();
    await act(async () => new Promise(resolve => setTimeout(resolve, 350)));
    await click(button('시청 시작')); const firstTwo = [...container.querySelectorAll('[data-native-id]')]; expect(playerLifecycle.open).toHaveBeenCalledTimes(2);
    await click(button('방송 조작 열기')); await click(button('방송 추가')); await draftInput(items[2]!.url, 2); await click(button('방송 추가')); await draftInput(items[3]!.url, 3); expect(playerLifecycle.open).toHaveBeenCalledTimes(2); await act(async () => new Promise(resolve => setTimeout(resolve, 350))); await click(button('변경'));
    const all = [...container.querySelectorAll('[data-native-id]')]; expect(all.slice(0, 2)).toEqual(firstTwo); expect(playerLifecycle.open).toHaveBeenCalledTimes(4); expect(playerLifecycle.close).not.toHaveBeenCalled();
    await click(button('방송 조작 열기')); await click(button('선택 보기'));
    const select = container.querySelector<HTMLSelectElement>('[aria-label="볼 영상"]')!;
    expect([...select.options].map(item => item.textContent)).toEqual(items.map(item => item.author));
    await act(async () => { select.value = `video:${items[2]!.id}`; select.dispatchEvent(new Event('change', { bubbles: true })); }); await click(button('변경'));
    expect([...container.querySelectorAll('[data-native-id]')]).toEqual(all); expect(container.querySelectorAll('[data-chat-for]')).toHaveLength(4);
    expect(container.querySelectorAll('[data-native-id][data-video-visible="true"]')).toHaveLength(1); expect(container.querySelector('[data-native-id][data-video-visible="true"]')?.getAttribute('data-native-id')).toBe(items[2]!.id);
    await render({ view: 'live', active: false }); expect(container.querySelectorAll('[data-native-id][data-active="false"]')).toHaveLength(4);
    await render({ view: 'live' }); await click(button('방송 조작 열기')); await click(button('모두 보기')); await click(button('변경'));
    expect([...container.querySelectorAll('[data-native-id]')]).toEqual(all); expect(playerLifecycle.close).not.toHaveBeenCalled(); expect(playerLifecycle.open).toHaveBeenCalledTimes(4);
    await click(button('방송 조작 열기')); await click(button('종료')); expect(playerLifecycle.close).toHaveBeenCalledTimes(4); expect(container.querySelectorAll('.youtube-card')).toHaveLength(4);
  });
  it('keeps a hidden VOD iframe and native live receiver unchanged in mixed selected viewing', async () => {
    const live = { ...second, isLive: true }; vi.mocked(api.search!).mockResolvedValue({ items: [video, live] }); await render(); await search();
    const cards = container.querySelectorAll('.youtube-card'); await click(cards[0]!); await click(cards[1]!, { shiftKey: true }); await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    await render({ view: 'live' }); await click(button('방송 보기 열기')); await act(async () => new Promise(resolve => setTimeout(resolve, 350))); await click(button('시청 시작'));
    const frame = container.querySelector('iframe')!, native = container.querySelector('[data-native-id]'), messages = vi.spyOn(frame.contentWindow!, 'postMessage');
    await click(button('방송 조작 열기')); await click(button('선택 보기')); const select = container.querySelector<HTMLSelectElement>('[aria-label="볼 영상"]')!;
    await act(async () => { select.value = `video:${live.id}`; select.dispatchEvent(new Event('change', { bubbles: true })); }); await click(button('변경'));
    expect(container.querySelector('iframe')).toBe(frame); expect(container.querySelector('[data-native-id]')).toBe(native); expect(frame.closest('.youtube-playing-cell')?.getAttribute('data-background-vod')).toBe('true');
    expect(frame.closest('.youtube-playing-cell')).toHaveAttribute('inert'); expect(messages).not.toHaveBeenCalled(); expect(playerLifecycle.close).not.toHaveBeenCalled(); expect(playerLifecycle.open).toHaveBeenCalledTimes(1);
    await render(); expect(container.querySelector('iframe')).toBe(frame); expect(frame.closest('.youtube-playing-cell')).not.toHaveAttribute('inert'); expect(container.querySelector('.youtube-detail-actions')).not.toBeNull();
  });
  it('keeps four views bounded, provides CHZZK-like resize/narrow chat tabs and restores fullscreen pair sizing', () => {
    expect(layout).toContain('.youtube-viewing .youtube-playing-grid{height:100%;min-height:0;overflow:hidden}');
    expect(layout).toContain('grid-template-rows:minmax(0,var(--pane-first-row)) 10px minmax(0,var(--pane-second-row))');
    expect(layout).toContain('container:youtube-workspace/inline-size');
    expect(layout).toContain('display:contents');
    expect(layout).toContain('.youtube-chat-tabs{display:flex;grid-column:5;grid-row:1}');
    expect(layout).toContain('.platform-live-pair:fullscreen{display:flex!important');
    expect(layout).toContain('var(--platform-live-chat-size');
  });
  it('keeps previous playing videos mounted when adding another video', async () => {
    await render(); await search(); const cards=container.querySelectorAll('.youtube-card');
    await click(cards[0]!); await click(cards[1]!,{shiftKey:true}); await act(async()=>window.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true})));
    await click(button('재생')); const first=container.querySelector('iframe');
    const tabs=container.querySelectorAll<HTMLButtonElement>('.youtube-detail-tabs>div>button');
    await click([...tabs].find(item=>item.textContent===second.title)!); await click(button('재생'));
    expect(container.querySelectorAll('iframe')).toHaveLength(2); expect([...container.querySelectorAll('iframe')]).toContain(first); expect(container.querySelector('.youtube-playing-grid.is-multiple')).not.toBeNull();
  });
  it('searches internally and keeps original title only in detail', async () => { await render(); await search(); expect(api.search).toHaveBeenCalledWith({ query: 'NASA', filter: 'all', sort: 'relevance' }); expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe('공개 영상'); expect(container.querySelector('iframe')).toBeNull(); await detail(); expect(container.querySelector('.youtube-detail-actions')?.textContent).toContain('Public | 공개 영상'); expect(api.openExternal).not.toHaveBeenCalled(); });
  it('uses focus on one click, Ctrl selection on two, and native batch on Ctrl+Enter', async () => { await render(); await search(); const cards = container.querySelectorAll('.youtube-card'); await click(cards[0]!); expect(container.querySelector('.youtube-card-check')).toBeNull(); expect(container.querySelector('[role=dialog]')).toBeNull(); await click(cards[1]!, { ctrlKey: true }); expect(container.querySelectorAll('.youtube-card-check')).toHaveLength(2); await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true }))); expect(media.downloadBatch).toHaveBeenCalledWith([{ provider: 'youtube', kind: 'video', externalId: video.id }, { provider: 'youtube', kind: 'video', externalId: second.id }], expect.any(String)); });
  it('opens multiple selected videos as detail tabs on Enter', async () => { await render(); await search(); const cards = container.querySelectorAll('.youtube-card'); await click(cards[0]!); await click(cards[1]!, { shiftKey: true }); await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))); expect(container.querySelectorAll('.youtube-detail-tabs>div')).toHaveLength(2); expect(media.download).not.toHaveBeenCalled(); });
  it('gives nested creator Ctrl-click a background search without changing the selected tab', async () => { await render(); await search(); const active = container.querySelector('.youtube-search-tabs .is-active')?.textContent; await click(container.querySelector('.youtube-author')!, { ctrlKey: true }); expect(container.querySelectorAll('.youtube-search-tabs>div')).toHaveLength(2); expect(container.querySelector('.youtube-search-tabs .is-active')?.textContent).toBe(active); expect(container.querySelector('.youtube-card-check')).toBeNull(); });
  it('removes keyboard focus when a focused card is deselected', async () => { await render(); await search(); const card = container.querySelector<HTMLElement>('.youtube-card')!; card.focus(); await click(card); await click(card, { ctrlKey: true }); expect(card.classList.contains('is-focused')).toBe(false); expect(document.activeElement).not.toBe(card); });
  it('gives creator context actions priority over the underlying video', async () => { await render(); await search(); await act(async () => container.querySelector('.youtube-author')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))); const menu = container.querySelector('[role=menu]')!; expect(menu.textContent).not.toContain('다운로드'); await click([...menu.querySelectorAll('button')].find(item => item.textContent === '즐겨찾기')!); const saved = JSON.parse(localStorage.getItem(YOUTUBE_FAVORITES_KEY)!); expect(saved[0].kind).toBe('channel'); expect(saved[0].title).toBe(video.author); });
  it('traps detail focus and lets Escape close from a focused button', async () => { await render(); await search(); await detail(); const dialog = container.querySelector('[role=dialog]')!, first = dialog.querySelector<HTMLButtonElement>('button')!; expect(document.activeElement).toBe(first); await act(async () => first.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true }))); expect(document.activeElement).toBe([...dialog.querySelectorAll('button:not(:disabled)')].at(-1)); await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))); expect(container.querySelector('[role=dialog]')).toBeNull(); });
  it('persists favorites without thumbnails or account changes', async () => { await render(); await search(); await click(container.querySelector('.youtube-favorite')!); expect(localStorage.getItem(YOUTUBE_FAVORITES_KEY)).not.toContain('data:image'); await render({ view: 'favorites' }); expect(container.querySelectorAll('.youtube-card')).toHaveLength(1); expect(api.openExternal).not.toHaveBeenCalled(); });
  it('preserves background playback across platform changes and minimizes only the presentation', async () => { await render(); await search(); await detail(); await click(button('재생')); const iframe = container.querySelector('iframe'); expect(iframe).not.toBeNull(); await render({ active: false }); expect(container.querySelector('iframe')).toBe(iframe); expect(container.querySelector<HTMLElement>('.youtube-detail-backdrop')?.hidden).toBe(true); await render(); await click(button('상세 내리기')); expect(container.querySelector('iframe')).toBe(iframe); expect(button('열린 영상 1')).toBeDefined(); });
  it('masks privacy without destroying the playback session', async () => { await render(); await search(); await detail(); await click(button('재생')); const iframe = container.querySelector('iframe'); await render({ privacyMode: true }); expect(container.querySelector('iframe')).toBe(iframe); expect(container.querySelector('.youtube-privacy-mask')).not.toBeNull(); expect(container.querySelector('img')).toBeNull(); expect(container.textContent).not.toContain('Public'); });
  it('ignores spoofed iframe errors and renders verified errors', async () => { await render(); await search(); await detail(); await click(button('재생')); const iframe = container.querySelector('iframe')!; await act(async () => window.dispatchEvent(new MessageEvent('message', { origin: 'https://evil.com', source: iframe.contentWindow, data: { event: 'onError', info: 153 } }))); expect(container.textContent).not.toContain('앱의 재생 요청'); await act(async () => window.dispatchEvent(new MessageEvent('message', { origin: 'https://www.youtube.com', source: iframe.contentWindow, data: { event: 'onError', info: 153 } }))); expect(container.textContent).toContain('앱의 재생 요청'); });
  it('retains a result resolving while another platform is active', async () => { let resolve!: (value: { items: YouTubeMetadata[] }) => void; vi.mocked(api.search!).mockReturnValue(new Promise(complete => { resolve = complete; })); await render(); await search(); await render({ active: false }); await act(async () => resolve({ items: [video] })); await render(); expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe('공개 영상'); });
  it('remains usable when localStorage is blocked', async () => { vi.spyOn(window, 'localStorage', 'get').mockImplementation(() => { throw new DOMException('Blocked', 'SecurityError'); }); await render(); await search(); await click(container.querySelector('.youtube-favorite')!); expect(container.textContent).toContain('즐겨찾기를 저장하지 못했습니다'); });
});
