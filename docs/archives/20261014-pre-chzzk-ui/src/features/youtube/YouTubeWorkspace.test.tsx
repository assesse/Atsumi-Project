import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { YouTubeWorkspace, type YouTubeWorkspaceProps } from './YouTubeWorkspace';
import { parseYouTubeTarget, saveYouTubeFavorites, YOUTUBE_FAVORITES_KEY, type YouTubeMetadata } from './model';
import type { YouTubeApi } from './api';
import type { PlatformMediaApi } from '../platforms/platformMedia';
import type { MediaRef } from '../platforms/platformMedia';
import layout from './YouTubeMultiView.css?raw';
const playerLifecycle = vi.hoisted(() => ({ open: vi.fn(), close: vi.fn() }));
vi.mock('../platforms/PlatformLivePlayer', async () => {
  const { useEffect } = await import('react');
  return { PlatformLivePlayer: ({ reference, active, videoVisible }: { reference: MediaRef; active: boolean; videoVisible: boolean }) => {
    useEffect(() => { playerLifecycle.open(reference.externalId); return () => playerLifecycle.close(reference.externalId); }, [reference.externalId]);
    return <div className="platform-live-pair" data-native-id={reference.externalId} data-active={active} data-video-visible={videoVisible}><video /><aside data-chat-for={reference.externalId} /></div>;
  } };
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../platforms/PlatformScreenshotLibrary', () => ({ PlatformScreenshotLibrary: ({ provider, active }: { provider: string; active: boolean }) => <section data-testid="screenshot-library" data-provider={provider} hidden={!active} /> }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
const video: YouTubeMetadata = { ...parseYouTubeTarget('M7lc1UVf-VE')!, title: 'Public | 공개 영상', author: 'Google for Developers', authorUrl: 'https://www.youtube.com/@GoogleDevelopers', thumbnail: 'data:image/jpeg;base64,AA==' };
const second: YouTubeMetadata = { ...parseYouTubeTarget('jNQXAC9IVRw')!, title: '두 번째 영상', author: 'Sample' };
const channel: YouTubeMetadata = { ...parseYouTubeTarget(video.authorUrl!)!, title: video.author, author: '' };
let root: Root, container: HTMLDivElement, api: YouTubeApi, media: PlatformMediaApi;
beforeEach(() => { playerLifecycle.open.mockClear(); playerLifecycle.close.mockClear(); localStorage.clear(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); container = document.createElement('div'); document.body.append(container); root = createRoot(container); api = { metadata: vi.fn().mockResolvedValue(video), search: vi.fn().mockResolvedValue({ items: [video, second] }), thumbnail: vi.fn().mockResolvedValue(null), openExternal: vi.fn().mockResolvedValue(undefined) }; media = { download: vi.fn(), downloadBatch: vi.fn().mockResolvedValue([]), jobs: vi.fn().mockResolvedValue([]), library: vi.fn().mockResolvedValue({ items: [] }), cancel: vi.fn(), retry: vi.fn(), open: vi.fn(), close: vi.fn(), openFolder: vi.fn(), delete: vi.fn() }; });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const button = (label: string) => [...container.querySelectorAll<HTMLButtonElement>('button')].find(item => item.getAttribute('aria-label') === label || item.textContent === label)!;
async function render(patch: Partial<YouTubeWorkspaceProps> = {}) { await act(async () => root.render(<YouTubeWorkspace active runtime="browser-mock" privacyMode={false} view="explore" api={api} mediaApi={media} {...patch} />)); }
async function search(value = 'NASA') { await act(async () => { const input = container.querySelector('input')!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value); input.dispatchEvent(new Event('input', { bubbles: true })); }); await act(async () => container.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))); }
async function click(element: Element, modifiers: MouseEventInit = {}) { await act(async () => element.dispatchEvent(new MouseEvent('click', { bubbles: true, ...modifiers }))); }
async function detail() { await act(async () => container.querySelector('.youtube-card')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))); }

describe('YouTube app discovery', () => {
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
    const items = ['abcdefghijk', 'bcdefghijkl', 'cdefghijklm', 'defghijklmn', 'efghijklmno'].map((id, index) => ({ ...video, id, isLive: true, title: `라이브 ${index + 1}` }));
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
    await render();
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
    expect(onViewChange).toHaveBeenCalledTimes(2);
    expect(list.scrollTop).toBe(140);
    await render({ view: 'favorites', onViewChange }); expect(list.scrollTop).toBe(70);
    await click(button('영상')); expect(list.scrollTop).toBe(35);
  });
  it.each([false, true])('opens a favorite creator with background=%s while keeping favorites distinct from results', async background => {
    saveYouTubeFavorites(localStorage, [{ ...video, savedAt: 1 }]);
    vi.mocked(api.search!).mockImplementation(async request => ({ items: request.channel ? [second] : [video] }));
    const onViewChange = vi.fn();
    await render({ onViewChange }); await search();
    await render({ view: 'favorites', onViewChange });
    await click(container.querySelector('.youtube-author')!, { ctrlKey: background });
    expect(api.search).toHaveBeenLastCalledWith({ query: '', channel: channel.url, filter: 'all', sort: 'relevance' });
    expect(onViewChange).toHaveBeenCalledTimes(background ? 0 : 1);
    if (!background) expect(onViewChange).toHaveBeenCalledWith('explore');
    expect(container.querySelector('.youtube-card-text strong')?.textContent).toBe('공개 영상');
    expect(container.querySelector('.youtube-search-tabs')).toBeNull();
    expect(container.querySelector('.youtube-card-check')).toBeNull();
    await render({ onViewChange });
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
    await render(); expect(list.scrollTop).toBe(480);
    await render({ view: 'favorites' }); expect(list.scrollTop).toBe(35);
    await click(button('채널')); expect(list.scrollTop).toBe(0);
    await click(button('영상')); expect(list.scrollTop).toBe(35);
    await render(); expect(list.scrollTop).toBe(480);
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
    await click(container.querySelector('.youtube-viewing-empty button')!);
    expect(container.querySelector('.youtube-viewing-panel')).not.toBeNull();
    await click(button('탐색에서 영상 찾기')); expect(onViewChange).toHaveBeenCalledWith('explore');
    expect(container.querySelector('iframe')).toBeNull(); expect(api.openExternal).not.toHaveBeenCalled();
  });
  it('moves viewing actions into a sheet without changing VOD detail or playback identity', async () => {
    await render(); await search(); await detail(); await click(button('재생'));
    const frame = container.querySelector('iframe'), host = frame?.closest('.youtube-playing-cell');
    await render({ view: 'live' });
    expect(container.querySelector('.youtube-detail-tabs')).toBeNull(); expect(container.querySelector('.youtube-detail-actions')).toBeNull();
    await click(button('방송 조작 열기')); expect(container.querySelector('.youtube-viewing-panel')).not.toBeNull();
    await click(button('방송 보기 닫기')); expect(container.querySelector('.youtube-viewing-panel')).toBeNull();
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
    expect(container.querySelector('.youtube-viewing-panel')).toBeNull(); expect(container.querySelector('iframe')).toBe(frame);
    await render({ view: 'live' }); expect(container.querySelector('.youtube-viewing-panel')).toBeNull();
    await render(); expect(container.querySelector<HTMLInputElement>('[aria-label="YouTube 검색"]')?.value).toBe('keep this query'); expect(container.querySelector('iframe')).toBe(frame);
  });
  it('keeps all live receivers and chats when applying2→4 and all→selected→all', async () => {
    const items = ['abcdefghijk', 'bcdefghijkl', 'cdefghijklm', 'defghijklmn'].map((id, index) => ({ ...video, id, url: `https://www.youtube.com/watch?v=${id}`, isLive: true, author: `채널 ${index + 1}`, title: `라이브 ${index + 1}` }));
    vi.mocked(api.search!).mockResolvedValue({ items }); await render(); await search();
    const cards = container.querySelectorAll('.youtube-card'); await click(cards[0]!); await click(cards[3]!, { shiftKey: true });
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    await render({ view: 'live' }); await click(button('방송 보기 열기'));
    await click(button('라이브 1 시청 선택')); await click(button('라이브 2 시청 선택')); expect(playerLifecycle.open).not.toHaveBeenCalled();
    await click(button('시청')); const firstTwo = [...container.querySelectorAll('[data-native-id]')]; expect(playerLifecycle.open).toHaveBeenCalledTimes(2);
    await click(button('방송 조작 열기')); await click(button('라이브 3 시청 선택')); await click(button('라이브 4 시청 선택')); expect(playerLifecycle.open).toHaveBeenCalledTimes(2); await click(button('변경'));
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
    await render({ view: 'live' }); await click(button('방송 보기 열기')); await click(button(`${video.title} 시청 선택`)); await click(button(`${live.title} 시청 선택`)); await click(button('시청'));
    const frame = container.querySelector('iframe')!, native = container.querySelector('[data-native-id]'), messages = vi.spyOn(frame.contentWindow!, 'postMessage');
    await click(button('방송 조작 열기')); await click(button('선택 보기')); const select = container.querySelector<HTMLSelectElement>('[aria-label="볼 영상"]')!;
    await act(async () => { select.value = `video:${live.id}`; select.dispatchEvent(new Event('change', { bubbles: true })); }); await click(button('변경'));
    expect(container.querySelector('iframe')).toBe(frame); expect(container.querySelector('[data-native-id]')).toBe(native); expect(frame.closest('.youtube-playing-cell')?.getAttribute('data-background-vod')).toBe('true');
    expect(frame.closest('.youtube-playing-cell')).toHaveAttribute('inert'); expect(messages).not.toHaveBeenCalled(); expect(playerLifecycle.close).not.toHaveBeenCalled(); expect(playerLifecycle.open).toHaveBeenCalledTimes(1);
    await render(); expect(container.querySelector('iframe')).toBe(frame); expect(frame.closest('.youtube-playing-cell')).not.toHaveAttribute('inert'); expect(container.querySelector('.youtube-detail-actions')).not.toBeNull();
  });
  it('keeps the four-view canvas bounded without an implicit one-column narrow layout', () => {
    expect(layout).toContain('.youtube-viewing .youtube-playing-grid { height: 100%; min-height: 0;');
    expect(layout).toContain('grid-template-rows: repeat(2, minmax(0, 1fr))');
    expect(layout).toContain('.youtube-viewing .youtube-playing-cell>.platform-live-pair { box-sizing: border-box; width: 100%; height: 100%; min-height: 0; max-height: none; }');
    expect(layout).toContain('.youtube-detail-backdrop:not(.youtube-viewing) .youtube-playing-grid.is-multiple { grid-template-columns: minmax(0, 1fr); }');
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
