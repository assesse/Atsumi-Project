import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { PlatformMediaLibrary } from './PlatformMediaLibrary';
import type { MediaChatArchive, MediaChatAsset, MediaJob, MediaLibraryItem, MediaRecordingGroup, MediaGroupMembersPage, MediaGroupsPage, MediaReplayChatPage, PlatformMediaApi } from './platformMedia';
import { groupPanelPosition, mediaGroupStorageKey, readOpenMediaGroup, saveOpenMediaGroup } from './mediaGroupPaging';
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
const item = (id: string): MediaLibraryItem => ({ libraryId: id, revision: 6, reference: { provider: 'youtube', kind: 'video', externalId: 'jNQXAC9IVRw' }, title: `Saved | 저장 영상 ${id}`, channelName: '채널', bytes: 1000000, duration: 19, completedAt: 1, sha256: 'a'.repeat(64) });
let container: HTMLDivElement, root: Root, api: PlatformMediaApi;
beforeEach(() => { localStorage.removeItem(mediaGroupStorageKey('youtube')); localStorage.removeItem(mediaGroupStorageKey('soop')); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); container = document.createElement('div'); document.body.append(container); root = createRoot(container); vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(); vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => undefined); api = { download: vi.fn(), jobs: vi.fn().mockResolvedValue([]), cancel: vi.fn(), retry: vi.fn(), library: vi.fn().mockResolvedValue({ items: [item('a'), item('b')] }), thumbnail: vi.fn().mockResolvedValue('data:image/jpeg;base64,AA=='), open: vi.fn().mockImplementation(async id => ({ token: `token-${id}`, url: `http://platform-media.localhost/token-${id}/media.mp4`, item: item(id) })), close: vi.fn().mockResolvedValue(undefined), openFolder: vi.fn().mockResolvedValue(undefined), delete: vi.fn().mockResolvedValue(undefined) }; });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); delete document.documentElement.dataset.tutorialOpen; vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });
const render = async (active = true, privacyMode = false) => { await act(async () => root.render(<PlatformMediaLibrary provider="youtube" active={active} privacyMode={privacyMode} api={api} />)); };
const click = async (target: Element, props: MouseEventInit = {}) => { await act(async () => target.dispatchEvent(new MouseEvent('click', { bubbles: true, ...props }))); };
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>('button')].find(v => v.textContent === text || v.getAttribute('aria-label') === text)!;
it('shows a labelled current channel picture for missing historical proof only on active visible cards', async () => {
  api.profile=vi.fn().mockResolvedValue(null); api.currentProfile=vi.fn().mockResolvedValue('data:image/png;base64,AQ==');
  await render(false); expect(api.profile).not.toHaveBeenCalled(); await render(true,true); expect(api.currentProfile).not.toHaveBeenCalled();
  await render(); expect(api.currentProfile).toHaveBeenCalledWith('a');
  expect(container.querySelector('.platform-media-channel-avatar img')).toHaveAttribute('src','data:image/png;base64,AQ==');
  expect(container.querySelector('.platform-media-channel-avatar')).toHaveAttribute('title','현재 채널 사진 · 녹화 당시 사진 아님');
  expect(api.open).not.toHaveBeenCalled();
});
it('uses compact profile-first CHZZK composition without poster reads and single focus before multiselect', async () => { await render(); expect(api.thumbnail).not.toHaveBeenCalled(); const cards = container.querySelectorAll('article'); expect(cards[0]!.firstElementChild).toHaveClass('platform-media-channel'); await click(cards[0]!); expect(container.querySelector('.media-selection')).toBeNull(); expect(container.querySelector('.platform-media-library-detail')).toHaveTextContent('저장 영상 a'); await click(cards[1]!, { ctrlKey: true }); expect(container.querySelector('.media-selection')?.textContent).toContain('2개 선택'); expect(api.open).not.toHaveBeenCalled(); });
it('keeps partial chat warning beside file info and in replay after refresh', async () => {
  const saved: MediaLibraryItem = { ...item('a'), reference: { ...item('a').reference, kind: 'live' }, chatArchive: { state: 'partial', reasons: ['archive_limit'], originalsRetained: true, assetsIncomplete: false } };
  api.library = vi.fn().mockResolvedValue({ items: [saved] }); api.open = vi.fn().mockResolvedValue({ token: 'partial', url: 'http://platform-media.localhost/partial/media.mp4', item: saved });
  vi.useFakeTimers(); await render();
  const card = container.querySelector('article')!;
  expect(card.querySelector('.platform-media-file-top')).toHaveTextContent('0:19');
  expect(card.querySelector('.platform-media-file-bottom')).toHaveTextContent('1.0 MB 저장됨 · 채팅 일부 누락');
  expect(card.querySelector('.platform-media-chat-warning')).toHaveAttribute('aria-label', '채팅 일부 누락 · 원본 보존');
  await act(async () => card.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  expect(container.querySelector('.platform-media-player header')).toHaveTextContent('채팅 일부 누락');
  await act(async () => vi.advanceTimersByTime(3000));
  expect(container.querySelector('.platform-media-player header')).toHaveTextContent('채팅 일부 누락');
  expect(api.close).not.toHaveBeenCalled();
});
it.each([false, true])('keeps unverified partial chat visibly uncertain in the card and replay after refresh (assets: %s)', async assetsIncomplete => {
  const saved: MediaLibraryItem = { ...item('a'), reference: { ...item('a').reference, kind: 'live' }, chatArchive: { state: 'partial', reasons: ['status_unknown'], originalsRetained: true, assetsIncomplete } };
  api.library = vi.fn().mockResolvedValue({ items: [saved] });
  api.open = vi.fn().mockResolvedValue({ token: 'unverified', url: 'http://platform-media.localhost/unverified/media.mp4', item: saved });
  api.replayChat = vi.fn().mockResolvedValue({ ...savedChatPage(), state: 'empty', items: [], total: 0, partialFile: true });
  vi.useFakeTimers(); await render();
  const card = container.querySelector('article')!;
  expect(card.querySelector('.platform-media-file-bottom')).toHaveTextContent('1.0 MB 저장됨 · 채팅 상태 확인 필요');
  expect(card.querySelectorAll('.platform-media-file-bottom')).toHaveLength(1);
  const assertWarning = (scope: Element) => {
    const warning = scope.querySelector('.platform-media-chat-warning')!;
    expect(warning).toHaveAttribute('aria-label', '채팅 상태 확인 필요 · 원본 보존');
    expect(warning.getAttribute('title')).toContain('채팅 보존 상태를 확인하지 못했습니다.');
    expect(warning.getAttribute('title')).toContain('원본 보존.');
    expect(warning.textContent).not.toMatch(/일부 누락|정상|미제공|문제없음/);
    expect(warning.getAttribute('title')).not.toMatch(/건너뛰|제공하지 않|문제없음/);
    if (assetsIncomplete) expect(warning.getAttribute('title')).toContain('일부 이모티콘은 문자로 표시됩니다.');
    else expect(warning.getAttribute('title')).not.toContain('이모티콘');
  };
  assertWarning(card);
  await act(async () => card.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = container.querySelector('video');
  assertWarning(container.querySelector('.platform-media-player header')!);
  await act(async () => vi.advanceTimersByTime(3000));
  assertWarning(container.querySelector('article')!); assertWarning(container.querySelector('.platform-media-player header')!);
  expect(container.querySelector('video')).toBe(video); expect(api.close).not.toHaveBeenCalled();
  expect(saved.chatArchive).toEqual({ state: 'partial', reasons: ['status_unknown'], originalsRetained: true, assetsIncomplete });
});
it.each([
  ['archive_limit', '채팅 저장 한도에 도달했습니다.'],
  ['write_failed', '일부 채팅을 저장하지 못했습니다.'],
  ['flush_failed', '채팅의 최종 저장을 확인하지 못했습니다.'],
  ['interrupted', '채팅 기록이 중단되었습니다.'],
] as const)('retains the explicit %s warning across state contradictions and either unknown-reason ordering', async (failure, detail) => {
  const saved = (['partial', 'unknown', 'complete'] as const).flatMap(state => ([[failure], ['status_unknown', failure], [failure, 'status_unknown']] as MediaChatArchive['reasons'][]).map((reasons, index): MediaLibraryItem => ({
    ...item(`${state}-${index}`), chatArchive: { state, reasons, originalsRetained: index !== 0, assetsIncomplete: index === 2 },
  })));
  api.library = vi.fn().mockResolvedValue({ items: saved }); await render();
  const cards = container.querySelectorAll('article'); expect(cards).toHaveLength(saved.length);
  cards.forEach((card, index) => {
    const warning = card.querySelector('.platform-media-chat-warning')!, status = saved[index]!.chatArchive!;
    expect(warning).toHaveTextContent('채팅 일부 누락');
    expect(warning.getAttribute('title')).toContain(detail);
    expect(warning.getAttribute('title')).not.toContain('채팅 보존 상태를 확인하지 못했습니다.');
    expect(warning).toHaveAttribute('aria-label', `채팅 일부 누락${status.originalsRetained ? ' · 원본 보존' : ''}`);
    if (status.assetsIncomplete) expect(warning.getAttribute('title')).toContain('일부 이모티콘은 문자로 표시됩니다.');
    else expect(warning.getAttribute('title')).not.toContain('이모티콘');
  });
});
it('keeps empty or future reason uncertainty visible and does not treat a contradictory complete flag as proof', async () => {
  const saved = (['partial', 'unknown', 'complete'] as const).flatMap(state => [[], ['status_unknown'], ['future_reason']].filter(reasons => state !== 'complete' || reasons.length > 0).map((reasons, index): MediaLibraryItem => ({
    ...item(`${state}-${index}`), chatArchive: { state, reasons: reasons as MediaChatArchive['reasons'], originalsRetained: false, assetsIncomplete: false },
  })));
  saved.push({ ...item('verified'), chatArchive: { state: 'complete', reasons: [], originalsRetained: false, assetsIncomplete: false } });
  api.library = vi.fn().mockResolvedValue({ items: saved }); await render();
  const cards = container.querySelectorAll('article');
  [...cards].slice(0, -1).forEach(card => {
    const warning = card.querySelector('.platform-media-chat-warning')!;
    expect(warning).toHaveAttribute('aria-label', '채팅 상태 확인 필요');
    expect(warning.getAttribute('title')).toContain('채팅 보존 상태를 확인하지 못했습니다.');
    expect(warning.getAttribute('title')).not.toMatch(/future_reason|원본 보존|정상|미제공/);
  });
  expect(cards[cards.length - 1]!.querySelector('.platform-media-chat-warning')).toBeNull();
});
it.each(['complete', 'unknown'] as const)('distinguishes asset-only and unknown chat status without new card rows: %s', async state => {
  const saved = { ...item('a'), chatArchive: { state, reasons: state === 'unknown' ? ['status_unknown' as const] : [], originalsRetained: state === 'unknown', assetsIncomplete: true } };
  api.library = vi.fn().mockResolvedValue({ items: [saved] }); await render();
  expect(container.querySelector('.platform-media-file-bottom')).toHaveTextContent(state === 'unknown' ? '채팅 상태 확인 필요' : '이모티콘 일부 누락');
  expect(container.querySelectorAll('.platform-media-file-bottom')).toHaveLength(1);
  expect(container.textContent).not.toContain('채팅 일부 누락');
});
it('does not invent chat completion warnings for legacy items without proof', async () => { await render(); expect(container.querySelector('.platform-media-chat-warning')).toBeNull(); });
it.each(['complete', 'partial', 'interrupted'] as const)('shows truthful raw capture %s beside size without changing playback or selection', async state => {
  const saved: MediaLibraryItem = { ...item('raw'), reference: { ...item('raw').reference, kind: 'live' },
    captureOutcome: { state, reasons: state === 'complete' ? [] : state === 'partial' ? ['incomplete_structure'] : ['source_interrupted'], excludedUnits: [0, 0], excludedBytes: [0, 0] } };
  api.library = vi.fn().mockResolvedValue({ items: [saved] });
  await render();
  const status = container.querySelector('.platform-media-file-bottom .platform-media-saved-state')!;
  expect(status).toHaveTextContent(state === 'complete' ? '정상' : state === 'partial' ? '일부 저장' : '중단됨');
  expect(status).toHaveClass(state === 'complete' ? 'is-success' : 'is-warning');
  await click(container.querySelector('article')!); expect(container.querySelector('.media-selection')).toBeNull();
  await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  expect(api.open).toHaveBeenCalledWith('raw');
});
const savedAtSeconds = 1_792_010_400;
const expectedSavedDate = () => new Date(savedAtSeconds * 1000).toLocaleString('ko-KR', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false });
it.each(['youtube', 'soop'] as const)('renders native Unix-seconds completion dates for %s and keeps privacy masking', async provider => {
  const saved: MediaLibraryItem = { ...item('dated'), reference: { provider, kind: 'video', externalId: provider === 'soop' ? '209244227' : 'jNQXAC9IVRw' }, completedAt: savedAtSeconds };
  api.library = vi.fn().mockResolvedValue({ items: [saved] });
  await act(async () => root.render(<PlatformMediaLibrary provider={provider} active privacyMode={false} api={api} />));
  const date = container.querySelector('.platform-media-file-bottom time')!;
  expect(date).toHaveTextContent(expectedSavedDate()); expect(date).toHaveTextContent('2026'); expect(date).not.toHaveTextContent('1970');
  expect(saved.completedAt).toBe(savedAtSeconds);
  await act(async () => root.render(<PlatformMediaLibrary provider={provider} active privacyMode api={api} />));
  expect(container.querySelector('.platform-media-file-bottom time')?.textContent).toBe('');
});
it('uses archived portraits in fixed channel slots without replacing titles or file info', async () => {
  api.profile = vi.fn().mockResolvedValueOnce('data:image/png;base64,AA==').mockResolvedValueOnce(null);
  await render();
  expect(api.profile).toHaveBeenCalledWith('a'); expect(api.profile).toHaveBeenCalledWith('b');
  expect(container.querySelectorAll('.platform-media-channel-avatar')).toHaveLength(2);
  expect(container.querySelectorAll('.platform-media-channel-avatar img')).toHaveLength(1);
  const card = container.querySelector('article')!;
  expect(card).toHaveTextContent('채널'); expect(card).toHaveTextContent('저장 영상 a'); expect(card.querySelector('.platform-media-file-top')).toHaveTextContent('0:19'); expect(card.querySelector('.platform-media-file-bottom')).toHaveTextContent('1.0 MB');
  await act(async () => card.querySelector('.platform-media-channel-avatar img')!.dispatchEvent(new Event('error')));
  expect(card.querySelector('.platform-media-channel-avatar img')).toBeNull();
  expect(card.querySelector('.platform-media-channel-avatar')).toHaveTextContent('채');
});
it.each(['https://example.com/portrait.png', 'data:image/svg+xml;base64,AA==', `data:image/png;base64,${'A'.repeat(180_001)}`])('rejects nonlocal or oversized portrait response', async value => {
  api.profile = vi.fn().mockResolvedValue(value); await render();
  expect(container.querySelector('.platform-media-channel-avatar img')).toBeNull();
  expect(container.querySelectorAll('.platform-media-channel-avatar')).toHaveLength(2);
});
it('ignores a late portrait result after privacy is enabled and does not request in privacy', async () => {
  let resolve!: (value: string | null) => void;
  api.profile = vi.fn().mockReturnValue(new Promise(done => { resolve = done; }));
  await render(true, true); expect(api.profile).not.toHaveBeenCalled();
  await render(true, false); expect(api.profile).toHaveBeenCalledTimes(2);
  await render(true, true); await act(async () => resolve('data:image/png;base64,AA=='));
  expect(container.querySelector('.platform-media-channel-avatar img')).toBeNull();
  expect(container.textContent).not.toContain('채널'); expect(api.profile).toHaveBeenCalledTimes(2);
});
it('defers offline portrait reads until the card is near the viewport and ignores failed reads', async () => {
  const callbacks: IntersectionObserverCallback[] = [];
  vi.stubGlobal('IntersectionObserver', class {
    constructor(callback: IntersectionObserverCallback) { callbacks.push(callback); }
    observe() {} disconnect() {}
  });
  api.profile = vi.fn().mockRejectedValue(new Error('missing old portrait'));
  await render(); expect(api.profile).not.toHaveBeenCalled();
  await act(async () => callbacks.forEach(callback => callback([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver)));
  expect(api.profile).toHaveBeenCalledTimes(2); expect(container.querySelectorAll('.platform-media-channel-avatar')).toHaveLength(2);
  expect(container.querySelector('.platform-media-error')).toBeNull();
});
it('requires explicit confirmation before selected media deletion', async () => { await render(); const cards = container.querySelectorAll('article'); await click(cards[0]!); await click(cards[1]!, { shiftKey: true }); await click(button('삭제')); expect(api.delete).not.toHaveBeenCalled(); expect(container.querySelector('[role=alertdialog]')).not.toBeNull(); await click(container.querySelector('.platform-media-confirm .danger')!); expect(api.delete).toHaveBeenCalledWith('a', 6); expect(api.delete).toHaveBeenCalledWith('b', 6); });
it('keeps the local player when inactive and revokes token only on close', async () => { await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))); const video = container.querySelector('video'); expect(video?.src).toContain('platform-media.localhost'); await render(false); expect(container.querySelector('video')).toBe(video); expect(api.close).not.toHaveBeenCalled(); await render(); await click(button('재생 닫기')); expect(api.close).toHaveBeenCalledWith('token-a'); expect(container.querySelector('video')).toBeNull(); });
it('offers center replay without removing controls after the end', async () => { await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))); const video = container.querySelector('video')!; await act(async () => video.dispatchEvent(new Event('ended', { bubbles: true }))); expect(container.querySelector('.platform-media-replay')).not.toBeNull(); expect(container.querySelector('input[aria-label="재생 위치"]')).not.toBeNull(); await click(container.querySelector('.platform-media-replay')!); expect(video.currentTime).toBe(0); expect(video.play).toHaveBeenCalled(); });
it('masks saved media and does not open playback in privacy mode', async () => { await render(true, true); expect(container.querySelector('img')).toBeNull(); expect(container.textContent).not.toContain('채널'); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))); expect(api.open).not.toHaveBeenCalled(); });
it('clears both selection and DOM focus on a second solo click and Ctrl deselection', async () => {
  await render(); const card = container.querySelector<HTMLElement>('article')!;
  card.focus(); await click(card); expect(card).toHaveClass('is-focused');
  await click(card); expect(card).not.toHaveClass('is-focused'); expect(document.activeElement).not.toBe(card);
  card.focus(); await click(card); await click(card, { ctrlKey: true });
  expect(card).not.toHaveClass('is-focused'); expect(document.activeElement).not.toBe(card);
});
it('adds repeated Shift ranges to noncontiguous selections without exposing checks for one item', async () => {
  api.library = vi.fn().mockResolvedValue({ items: ['a', 'b', 'c', 'd', 'e'].map(item) });
  await render(); const cards = container.querySelectorAll<HTMLElement>('article');
  await click(cards[0]!); expect(container.querySelector('.platform-media-check')).toBeNull();
  await click(cards[4]!, { metaKey: true });
  await click(cards[2]!, { shiftKey: true });
  expect([...cards].map(card => card.classList.contains('is-focused'))).toEqual([true, false, true, true, true]);
  expect(container.querySelector('.media-selection')).toHaveTextContent('4개 선택');
  await click(cards[3]!, { shiftKey: true });
  expect([...cards].map(card => card.classList.contains('is-focused'))).toEqual([true, false, true, true, true]);
  await click(cards[1]!, { ctrlKey: true, shiftKey: true });
  expect(container.querySelector('.media-selection')).toHaveTextContent('5개 선택');
  expect(api.open).not.toHaveBeenCalled();
});
it('ignores the second click in a real double-click sequence before opening exactly once', async () => {
  let finish!: (value: Awaited<ReturnType<PlatformMediaApi['open']>>) => void;
  vi.mocked(api.open).mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  await render(); const card = container.querySelector<HTMLElement>('article')!;
  card.focus(); await click(card, { detail: 1 });
  await click(card, { detail: 2 });
  expect(card).toHaveClass('is-focused'); expect(document.activeElement).toBe(card);
  expect(container.querySelector('.platform-media-check')).toBeNull();
  await act(async () => card.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, detail: 2 })));
  expect(card).toHaveClass('is-focused'); expect(api.open).toHaveBeenCalledExactlyOnceWith('a');
  await act(async () => finish({ token: 'token-a', url: 'http://platform-media.localhost/token-a/media.mp4', item: item('a') }));
  expect(container.querySelector('video')).not.toBeNull(); expect(api.open).toHaveBeenCalledTimes(1);
});
it('prunes removed selections and their range anchor when the visible library refreshes', async () => {
  vi.useFakeTimers();
  vi.mocked(api.library).mockResolvedValueOnce({ items: ['a', 'b', 'c', 'd', 'e'].map(item) });
  await render(); let cards = container.querySelectorAll<HTMLElement>('article');
  await click(cards[0]!); await click(cards[2]!, { ctrlKey: true });
  expect(container.querySelector('.media-selection')).toHaveTextContent('2개 선택');
  vi.mocked(api.library).mockResolvedValueOnce({ items: ['a', 'b', 'd', 'e'].map(item) });
  await act(async () => vi.advanceTimersByTimeAsync(3000));
  cards = container.querySelectorAll<HTMLElement>('article');
  expect(cards[0]).toHaveClass('is-focused'); expect(container.querySelector('.media-selection')).toBeNull();
  expect(container.querySelector('.platform-media-check')).toBeNull();
  await click(cards[2]!, { shiftKey: true });
  expect([...cards].map(card => card.classList.contains('is-focused'))).toEqual([false, false, true, false]);
  await click(cards[1]!, { shiftKey: true });
  expect([...cards].map(card => card.classList.contains('is-focused'))).toEqual([false, true, true, false]);
  await click(button('삭제')); await click(container.querySelector('.platform-media-confirm .danger')!);
  expect(api.delete).toHaveBeenCalledTimes(2);
  expect(api.delete).toHaveBeenCalledWith('b', 6); expect(api.delete).toHaveBeenCalledWith('d', 6);
});
it('selects only loaded cards with Ctrl+A and clears them with Ctrl+Shift+A', async () => {
  vi.mocked(api.library).mockResolvedValueOnce({ items: [item('a'), item('b')], cursor: 'more-unloaded' });
  await render();
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true, bubbles: true, cancelable: true })));
  expect(container.querySelector('.media-selection')).toHaveTextContent('2개 선택');
  expect(api.library).toHaveBeenCalledTimes(1);
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true })));
  expect(container.querySelector('.media-selection')).toBeNull();
  expect(container.querySelector('.is-focused')).toBeNull(); expect(container.querySelector('.platform-media-check')).toBeNull();
});
it('keeps context-menu precedence and the existing multi-selection for a selected card', async () => {
  await render(); const cards = container.querySelectorAll<HTMLElement>('article');
  await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
  await act(async () => cards[1]!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true })));
  expect(container.querySelector('.media-selection')).toHaveTextContent('2개 선택');
  await click(container.querySelector('[role="menu"] button:last-child')!);
  expect(container.querySelector('[role="menu"]')).toBeNull();
  expect(container.querySelector('[role="alertdialog"]')).toHaveTextContent('2개 영상을 삭제할까요?');
  expect(api.delete).not.toHaveBeenCalled(); expect(api.open).not.toHaveBeenCalled();
});
it('contains confirmation focus and closes it on Escape from its button', async () => {
  await render(); await click(container.querySelector('article')!);
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true })));
  expect(document.activeElement).toBe(button('닫기'));
  await act(async () => button('닫기').dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true })));
  expect(document.activeElement).toBe(container.querySelector('.platform-media-confirm .danger'));
  await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  expect(container.querySelector('[role=alertdialog]')).toBeNull(); expect(api.delete).not.toHaveBeenCalled();
});
it('defers library shortcuts to tutorials, unrelated modals and the foreground player', async () => {
  await render(); await click(container.querySelector('article')!);
  document.documentElement.dataset.tutorialOpen = 'true';
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true })));
  expect(container.querySelector('[role=alertdialog]')).toBeNull();
  delete document.documentElement.dataset.tutorialOpen;
  const foreign = document.createElement('div'); foreign.setAttribute('aria-modal', 'true'); document.body.append(foreign);
  try { await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true }))); expect(container.querySelector('[role=alertdialog]')).toBeNull(); }
  finally { foreign.remove(); }
  await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true })));
  expect(container.querySelector('[role=alertdialog]')).toBeNull();
  await act(async () => button('재생 닫기').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  expect(api.close).toHaveBeenCalledWith('token-a');
});
it('keeps operation errors across a successful periodic refresh', async () => {
  vi.useFakeTimers(); await render(); vi.mocked(api.open).mockRejectedValue(new Error('검증 파일 누락'));
  await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  expect(container.textContent).toContain('검증 파일 누락');
  await act(async () => vi.advanceTimersByTimeAsync(3100)); expect(container.textContent).toContain('검증 파일 누락');
});
it('shows actual queued/running/settled work below one stable summary, with cancellation settled', async () => {
  const states = ['queued', 'downloading', 'verifying', 'cancelled', 'failed'] as const;
  vi.mocked(api.jobs).mockResolvedValue(states.map((state, i) => ({ jobId: `${i}`, requestId: `${i}`, reference: item('a').reference, revision: 1, state, title: state, channelName: '', createdAt: 1, receivedBytes: 1000 } satisfies MediaJob)));
  await render();
  expect(container.querySelector('.queue-progress-track')).toHaveAttribute('aria-label', '자동 처리 종료 40% (취소·실패·중단 포함)');
  expect(container.querySelector('[role=tabpanel]')).toHaveTextContent('다운로드 중');
  expect(container.querySelector('[role=tabpanel]')).toHaveTextContent('검증 중');
  const tabs = [...container.querySelectorAll<HTMLButtonElement>('[role=tab]')];
  await click(tabs[0]!); expect(container.querySelector('[role=tabpanel]')).toHaveTextContent('queued'); expect(container.querySelector('[role=tabpanel]')).not.toHaveTextContent('downloading');
  await click(tabs[2]!); expect(container.querySelector('[role=tabpanel]')).toHaveTextContent('취소됨'); expect(container.querySelector('[role=tabpanel]')).toHaveTextContent('실패');
  expect(container.querySelector('.queue-summary-count')).toHaveTextContent('1 / 5 완료');
});
it('shows decoder and retry errors inside the open player, not behind it', async () => {
  await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  await act(async () => container.querySelector('video')!.dispatchEvent(new Event('error')));
  const dialog = container.querySelector('[role=dialog]')!;
  expect(dialog.querySelector('[role=alert]')).toHaveTextContent('저장 영상을 재생하지 못했습니다.');
  vi.mocked(api.open).mockRejectedValueOnce(new Error('파일을 찾을 수 없습니다.'));
  await click(button('다시 시도'));
  expect(dialog.querySelector('[role=alert]')).toHaveTextContent('파일을 찾을 수 없습니다.');
  expect(button('재생 닫기')).not.toBeDisabled();
});
it('uses foreground player shortcuts without intercepting controls, privacy or inactive tabs', async () => {
  await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = container.querySelector('video')!;
  const dialog = container.querySelector<HTMLElement>('[role=dialog]')!;
  expect(document.activeElement).toBe(dialog);
  const key = async (target: EventTarget, value: string) => act(async () => { target.dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true })); });
  video.currentTime = 10; await key(dialog, 'ArrowLeft'); expect(video.currentTime).toBe(5);
  await key(dialog, 'ArrowRight'); expect(video.currentTime).toBe(10);
  await key(dialog, 'm'); expect(video.muted).toBe(true); expect(button('음소거 해제')).toHaveAttribute('aria-pressed', 'true');
  await key(container.querySelector('input[aria-label="재생 위치"]')!, 'ArrowRight'); expect(video.currentTime).toBe(10);
  await key(dialog, 'k'); expect(video.play).toHaveBeenCalled(); vi.mocked(video.play).mockClear();
  await render(false); await key(window, 'k'); expect(video.play).not.toHaveBeenCalled();
  await render(true, true); expect(video.pause).toHaveBeenCalled(); await key(dialog, 'k'); expect(video.play).not.toHaveBeenCalled();
});
it('keeps a cancelled recording recoverable from its settled-work row', async () => {
  vi.mocked(api.jobs).mockResolvedValue([{ jobId: 'record', requestId: 'record', reference: { ...item('a').reference, kind: 'live' }, revision: 4, state: 'cancelled', title: '녹화', channelName: '', createdAt: 1, receivedBytes: 1000 }]);
  await render(); await click(container.querySelectorAll('[role=tab]')[2]!);
  expect(container.querySelector('.queue-summary-count')).toHaveTextContent('1 / 1 완료');
  await click(button('병합 재시도')); expect(api.retry).toHaveBeenCalledWith('record', 4);
});
it('connects saved recording chat to video time and keeps chat popup Escape local', async () => {
  const recorded = { ...item('a'), reference: { ...item('a').reference, kind: 'live' as const } };
  vi.mocked(api.open).mockResolvedValue({ token: 'token-a', url: 'http://platform-media.localhost/token-a/media.mp4', item: recorded });
  api.replayChat = vi.fn().mockResolvedValue({ items: [{ id: '1', timeMs: 0, kind: 'chat', author: '작성자', text: '저장 채팅', amount: null, badges: [], color: null }], state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: 1, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false });
  await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = container.querySelector('video')!;
  expect(container.querySelector('.platform-replay-chat')).toHaveTextContent('저장 채팅');
  expect(container.querySelectorAll('button[aria-label="재생 닫기"]')).toHaveLength(1);
  expect(container.querySelector('.platform-replay-chat button[aria-label="재생 닫기"]')).not.toBeNull();
  video.currentTime = 8;
  await act(async () => video.dispatchEvent(new Event('timeupdate')));
  expect(api.replayChat).toHaveBeenLastCalledWith('a', 8000, undefined, 120);
  await click(button('작성자')); expect(container.querySelector('[aria-label="채팅 사용자 정보"]')).not.toBeNull();
  await act(async () => button('사용자 정보 닫기').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  expect(container.querySelector('[aria-label="채팅 사용자 정보"]')).toBeNull(); expect(api.close).not.toHaveBeenCalled();
  expect(container.querySelector('video')).toBe(video);
  await render(false); const calls = vi.mocked(api.replayChat).mock.calls.length;
  video.currentTime = 9; await act(async () => video.dispatchEvent(new Event('timeupdate')));
  expect(api.replayChat).toHaveBeenCalledTimes(calls); expect(api.close).not.toHaveBeenCalled();
});
it.each(['soop', 'youtube'] as const)('connects the local %s presentation without replacing the owned file or chat clock', async provider => {
  const saved: MediaLibraryItem = { ...item('a'), reference: { provider, kind: 'live', externalId: provider === 'soop' ? 'phonics1' : 'jNQXAC9IVRw' } };
  api.open = vi.fn().mockResolvedValue({ token: 'local', url: 'http://platform-media.localhost/local/media.mp4', item: saved });
  api.replayChat = vi.fn().mockResolvedValue(savedChatPage());
  await act(async () => root.render(<PlatformMediaLibrary provider={provider} active privacyMode={false} api={api} />));
  await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = container.querySelector('video')!;
  expect(container.querySelector('.platform-replay-player')).toHaveClass(`is-${provider}`);
  expect(container.querySelector('.platform-replay-chat')).toHaveAttribute('data-provider', provider);
  expect(container.querySelectorAll('video')).toHaveLength(1); expect(container.querySelector('iframe')).toBeNull();
  expect(container.querySelector('.platform-media-player')).not.toHaveTextContent(/[Ⅱ▶⛶]/);
  video.currentTime = 8; await act(async () => video.dispatchEvent(new Event('timeupdate')));
  await click(button('재생 설정'));
  await click([...container.querySelectorAll('[role="menuitemradio"]')].find(node => node.textContent === '1.5배')!);
  expect(video.playbackRate).toBe(1.5); expect(video.currentTime).toBe(8); expect(container.querySelector('video')).toBe(video);
  expect(api.open).toHaveBeenCalledOnce(); expect(api.close).not.toHaveBeenCalled();
  expect(api.replayChat).toHaveBeenLastCalledWith('a', 8000, undefined, 120);
});
it('closes replay settings before the player and respects a foreign modal Escape', async () => {
  await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = container.querySelector('video'); await click(button('재생 설정'));
  const foreign = document.createElement('div'); foreign.setAttribute('aria-modal', 'true'); document.body.append(foreign);
  try {
    const foreignEscape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    await act(async () => foreign.dispatchEvent(foreignEscape)); expect(foreignEscape.defaultPrevented).toBe(false);
    expect(container.querySelector('.platform-replay-settings')).not.toBeNull(); expect(api.close).not.toHaveBeenCalled();
  } finally { foreign.remove(); }
  await act(async () => container.querySelector('.platform-replay-settings button')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  expect(container.querySelector('.platform-replay-settings')).toBeNull(); expect(api.close).not.toHaveBeenCalled(); expect(container.querySelector('video')).toBe(video);
  await act(async () => button('재생 설정').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  expect(api.close).toHaveBeenCalledExactlyOnceWith('token-a');
});
const savedChatPage = (): MediaReplayChatPage => ({ items: [{ id: 'vod-message', timeMs: 50, kind: 'chat', author: '저장 작성자', text: '보관된 다시보기 채팅', amount: null, badges: [], color: null }], state: 'ready', timelineQuality: 'native-offset', nextAtMs: null, cursor: null, previousCursor: null, total: 1, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false });
it.each(['soop', 'youtube'] as const)('opens archived video chat from the authoritative playback item for %s', async provider => {
  for (const state of ['complete', 'partial', 'unknown'] as const) {
    const saved: MediaLibraryItem = { ...item('a'), reference: { provider, kind: 'video', externalId: provider === 'soop' ? '209244227' : 'jNQXAC9IVRw' }, chatArchive: { state, reasons: state === 'partial' ? ['archive_limit'] : state === 'unknown' ? ['status_unknown'] : [], originalsRetained: state !== 'complete', assetsIncomplete: false } };
    // The list predates the proof; the opened item's current summary controls replay.
    api.library = vi.fn().mockResolvedValue({ items: [{ ...saved, chatArchive: null }] });
    api.open = vi.fn().mockResolvedValue({ token: 'vod', url: 'http://platform-media.localhost/vod/media.mp4', item: saved });
    api.replayChat = vi.fn().mockResolvedValue(savedChatPage());
    await act(async () => root.render(<PlatformMediaLibrary provider={provider} active privacyMode={false} api={api} />));
    await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    expect(container.querySelector('.platform-media-player')).toHaveClass('has-chat');
    expect(container.querySelectorAll('.platform-replay-chat')).toHaveLength(1);
    expect(container.querySelector('.platform-replay-chat')).toHaveTextContent('보관된 다시보기 채팅');
    expect(api.replayChat).toHaveBeenCalledExactlyOnceWith('a', 0, undefined, 120);
    expect(container.querySelectorAll('button[aria-label="재생 닫기"]')).toHaveLength(1);
    expect(container.querySelector('.platform-replay-chat button[aria-label="재생 닫기"]')).not.toBeNull();
    if (state === 'complete') expect(container.querySelector('.platform-media-player .platform-media-chat-warning')).toBeNull();
    else expect(container.querySelector('.platform-media-player .platform-media-chat-warning')).toHaveTextContent(state === 'partial' ? '채팅 일부 누락' : '채팅 상태 확인 필요');
    await click(button('재생 닫기'));
    expect(api.close).toHaveBeenCalledWith('vod');
  }
});
it.each(['soop', 'youtube'] as const)('uses the current unverified playback summary instead of a stale complete list summary for %s', async provider => {
  const listed: MediaLibraryItem = { ...item('a'), reference: { provider, kind: 'video', externalId: provider === 'soop' ? '209244227' : 'jNQXAC9IVRw' }, chatArchive: { state: 'complete', reasons: [], originalsRetained: false, assetsIncomplete: false } };
  const opened: MediaLibraryItem = { ...listed, chatArchive: { state: 'partial', reasons: ['status_unknown'], originalsRetained: true, assetsIncomplete: false } };
  api.library = vi.fn().mockResolvedValue({ items: [listed] });
  api.open = vi.fn().mockResolvedValue({ token: 'current', url: 'http://platform-media.localhost/current/media.mp4', item: opened });
  api.replayChat = vi.fn().mockResolvedValue({ ...savedChatPage(), partialFile: true });
  await act(async () => root.render(<PlatformMediaLibrary provider={provider} active privacyMode={false} api={api} />));
  expect(container.querySelector('article .platform-media-chat-warning')).toBeNull();
  await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  expect(container.querySelector('.platform-media-player header .platform-media-chat-warning')).toHaveAttribute('aria-label', '채팅 상태 확인 필요 · 원본 보존');
  expect(container.querySelector('.platform-replay-chat')).toHaveTextContent('보관된 다시보기 채팅');
  expect(api.replayChat).toHaveBeenCalledExactlyOnceWith('a', 0, undefined, 120);
  expect(container.querySelectorAll('button[aria-label="재생 닫기"]')).toHaveLength(1);
  await click(button('재생 닫기')); expect(api.close).toHaveBeenCalledWith('current');
});
it.each(['soop', 'youtube'] as const)('keeps ordinary %s videos without chat proof full width and does not read chat', async provider => {
  for (const chatArchive of [undefined, null]) {
    const saved: MediaLibraryItem = { ...item('a'), reference: { provider, kind: 'video', externalId: provider === 'soop' ? '209244227' : 'jNQXAC9IVRw' }, chatArchive };
    api.library = vi.fn().mockResolvedValue({ items: [saved] });
    api.open = vi.fn().mockResolvedValue({ token: 'plain', url: 'http://platform-media.localhost/plain/media.mp4', item: saved });
    api.replayChat = vi.fn().mockResolvedValue(savedChatPage());
    await act(async () => root.render(<PlatformMediaLibrary provider={provider} active privacyMode={false} api={api} />));
    await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    expect(container.querySelector('.platform-media-player')).not.toHaveClass('has-chat');
    expect(container.querySelector('.platform-replay-chat')).toBeNull();
    expect(api.replayChat).not.toHaveBeenCalled();
    expect(container.querySelector('.platform-media-player header button[aria-label="재생 닫기"]')).not.toBeNull();
    await click(button('재생 닫기'));
  }
});
it.each([undefined, null])('keeps legacy live replay with an honest missing state when proof is %s', async chatArchive => {
  const recorded: MediaLibraryItem = { ...item('a'), reference: { ...item('a').reference, kind: 'live' }, chatArchive };
  api.open = vi.fn().mockResolvedValue({ token: 'legacy', url: 'http://platform-media.localhost/legacy/media.mp4', item: recorded });
  api.replayChat = vi.fn().mockResolvedValue({ ...savedChatPage(), state: 'missing', items: [], total: 0, timelineQuality: 'missing-anchor' });
  await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  expect(container.querySelector('.platform-replay-chat')).toHaveTextContent('저장된 채팅이 없습니다.');
  expect(container.querySelector('.platform-media-player .platform-media-chat-warning')).toBeNull();
  expect(api.replayChat).toHaveBeenCalledExactlyOnceWith('a', 0, undefined, 120);
});
it('does not use a stale list archive claim when the opened video has no summary', async () => {
  api.library = vi.fn().mockResolvedValue({ items: [{ ...item('a'), chatArchive: { state: 'complete', reasons: [], originalsRetained: false, assetsIncomplete: false } }] });
  api.replayChat = vi.fn().mockResolvedValue(savedChatPage());
  await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  expect(container.querySelector('.platform-replay-chat')).toBeNull(); expect(api.replayChat).not.toHaveBeenCalled();
});
it('keeps SOOP VOD chat seekable with end replay, inactive playback and privacy unchanged', async () => {
  const saved: MediaLibraryItem = { ...item('a'), reference: { provider: 'soop', kind: 'video', externalId: '209244227' }, chatArchive: { state: 'complete', reasons: [], originalsRetained: false, assetsIncomplete: false } };
  api.library = vi.fn().mockResolvedValue({ items: [saved] });
  api.open = vi.fn().mockResolvedValue({ token: 'vod', url: 'http://platform-media.localhost/vod/media.mp4', item: saved });
  api.replayChat = vi.fn().mockResolvedValue(savedChatPage());
  const renderSoop = async (active = true, privacyMode = false) => { await act(async () => root.render(<PlatformMediaLibrary provider="soop" active={active} privacyMode={privacyMode} api={api} />)); };
  await renderSoop(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = container.querySelector('video')!;
  for (const seconds of [8, 2]) {
    video.currentTime = seconds; await act(async () => video.dispatchEvent(new Event('timeupdate')));
    expect(api.replayChat).toHaveBeenLastCalledWith('a', seconds * 1000, undefined, 120);
  }
  await act(async () => video.dispatchEvent(new Event('ended', { bubbles: true })));
  expect(container.querySelector('.platform-media-replay')).not.toBeNull();
  expect(container.querySelector('input[aria-label="재생 위치"]')).not.toBeNull();
  await click(container.querySelector('.platform-media-replay')!);
  expect(video.currentTime).toBe(0); expect(video.play).toHaveBeenCalled();
  await renderSoop(false); const calls = vi.mocked(api.replayChat).mock.calls.length;
  video.currentTime = 5; await act(async () => video.dispatchEvent(new Event('timeupdate')));
  expect(container.querySelector('video')).toBe(video); expect(api.replayChat).toHaveBeenCalledTimes(calls); expect(api.close).not.toHaveBeenCalled();
  await renderSoop(true, true);
  expect(container.querySelector('.platform-replay-chat')).not.toHaveTextContent('보관된 다시보기 채팅');
  expect(api.replayChat).toHaveBeenCalledTimes(calls); expect(video.muted).toBe(true);
  await renderSoop(); await click(button('재생 닫기'));
  expect(api.close).toHaveBeenCalledWith('vod'); expect(container.querySelector('video')).toBeNull();
});
it.each([false, true])('forwards local replay emoji reads and masks ready or late assets in privacy (pending: %s)', async pending => {
  const assetId = 'a'.repeat(64), image = 'data:image/png;base64,aGVsbG8=';
  const recorded = { ...item('a'), reference: { ...item('a').reference, kind: 'live' as const } };
  const page: MediaReplayChatPage = { items: [{ id: 'emoji', timeMs: 0, kind: 'chat', author: '작성자', text: ':flower:', amount: null, badges: [], color: null, parts: [{ type: 'emoji', assetId, alt: ':flower:' }] }], state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: 1, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false };
  vi.mocked(api.open).mockResolvedValue({ token: 'token-a', url: 'http://platform-media.localhost/token-a/media.mp4', item: recorded });
  api.replayChat = vi.fn().mockResolvedValue(page);
  let finish!: (asset: MediaChatAsset) => void;
  api.replayChatAsset = pending ? vi.fn().mockReturnValue(new Promise<MediaChatAsset>(resolve => { finish = resolve; })) : vi.fn().mockResolvedValue({ state: 'ready', dataUrl: image });
  await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = container.querySelector('video')!;
  expect(api.replayChatAsset).toHaveBeenCalledExactlyOnceWith('a', assetId);
  if (pending) expect(container.querySelector('.platform-replay-chat img')).toBeNull();
  else expect(container.querySelector('.platform-replay-chat img')).toHaveAttribute('src', image);
  await render(true, true);
  video.currentTime = 8; await act(async () => video.dispatchEvent(new Event('timeupdate')));
  if (pending) await act(async () => finish({ state: 'ready', dataUrl: image }));
  expect(container.querySelector('.platform-replay-chat img')).toBeNull();
  expect(container.querySelector('.platform-replay-chat')).not.toHaveTextContent(':flower:');
  expect(api.replayChatAsset).toHaveBeenCalledTimes(1);
  expect(container.querySelector('video')).toBe(video); expect(api.close).not.toHaveBeenCalled();
});
it('does not restore a deleted card from a poll started before deletion', async () => {
  vi.useFakeTimers(); await render();
  let resolvePage!: (page: { items: MediaLibraryItem[] }) => void;
  vi.mocked(api.library).mockReturnValueOnce(new Promise(resolve => { resolvePage = resolve; }));
  await act(async () => vi.advanceTimersByTimeAsync(3000));
  await click(container.querySelector('article')!);
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true })));
  await click(container.querySelector('.platform-media-confirm .danger')!);
  expect(container.querySelectorAll('article')).toHaveLength(1);
  await act(async () => resolvePage({ items: [item('a'), item('b')] }));
  expect(container.querySelectorAll('article')).toHaveLength(1);
  expect(container.querySelector('article')).toHaveTextContent('저장 영상 b');
});
it('passes the stable library cursor unchanged and merges concurrent new completions without duplicate cards', async () => {
  vi.useFakeTimers();
  const cursor = 'v1:boundary-b';
  vi.mocked(api.library).mockResolvedValueOnce({ items: [item('a'), item('b')], cursor });
  await render();
  vi.mocked(api.library).mockResolvedValueOnce({ items: [item('c')], cursor: 'v1:boundary-c' });
  await click(button('더 보기'));
  expect(api.library).toHaveBeenLastCalledWith('youtube', cursor);
  vi.mocked(api.library).mockResolvedValueOnce({ items: [{ ...item('new'), completedAt: 2 }, item('a')], cursor: 'v1:boundary-a' });
  await act(async () => vi.advanceTimersByTimeAsync(3000));
  expect(container.querySelectorAll('article')).toHaveLength(4);
  expect(container.querySelectorAll('article')[0]).toHaveTextContent('저장 영상 new');
  vi.mocked(api.library).mockResolvedValueOnce({ items: [item('d')] });
  await click(button('더 보기'));
  expect(api.library).toHaveBeenLastCalledWith('youtube', 'v1:boundary-c');
  expect(container.querySelectorAll('article')).toHaveLength(5);
  expect(button('더 보기')).toBeUndefined();
});
it('revokes a late playback result without replacing the more recent player', async () => {
  await render();
  let resolveFirst!: (value: Awaited<ReturnType<PlatformMediaApi['open']>>) => void;
  vi.mocked(api.open).mockReturnValueOnce(new Promise(resolve => { resolveFirst = resolve; }));
  await act(async () => container.querySelectorAll('article')[0]!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  await act(async () => container.querySelectorAll('article')[1]!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = container.querySelector('video')!;
  expect(video.src).toContain('token-b');
  await act(async () => resolveFirst({ token: 'late-a', url: 'http://platform-media.localhost/late-a/media.mp4', item: item('a') }));
  expect(api.close).toHaveBeenCalledWith('late-a');
  expect(container.querySelector('video')).toBe(video);
  expect(video.src).toContain('token-b');
});
it('consumes close rejection during unmount without a player state update', async () => {
  await render(); await act(async () => container.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  vi.mocked(api.close).mockRejectedValueOnce(new Error('앱 종료 중'));
  await act(async () => root.unmount());
  expect(api.close).toHaveBeenCalledWith('token-a');
  root = createRoot(container);
});

const recording = (id: string): MediaLibraryItem => ({ ...item(id), reference: { provider: 'youtube', kind: 'live', externalId: 'abcdefghijk' }, broadcastId: 'abcdefghijk' });
const group = (key: string, files = [recording(`${key}-1`), recording(`${key}-2`)]): MediaRecordingGroup => ({ type: 'recordingGroup', key, groupRevision: `revision-${key}`, representative: files[0]!, fileCount: files.length, totalBytes: files.reduce((sum, value) => sum + value.bytes, 0), latestCompletedAt: 1 });
const groupedApi = (groups = [group('a'), group('b')]) => {
  api.groups = vi.fn().mockResolvedValue({ status: 'ready', catalogRevision: 'catalog-1', entries: groups, cursor: null });
  api.groupMembers = vi.fn().mockImplementation(async (_provider, key) => ({ status: 'ready', group: groups.find(value => value.key === key)!, items: [recording(`${key}-1`), recording(`${key}-2`)], cursor: null }));
};
const heading = (key: string) => container.querySelector<HTMLButtonElement>(`[data-group-key="${key}"] .platform-media-group-heading`)!;
const panel = () => document.querySelector<HTMLDivElement>('.platform-media-group-panel')!;
const panelButton = (text: string) => [...panel().querySelectorAll<HTMLButtonElement>('button')].find(value => value.textContent === text || value.getAttribute('aria-label') === text)!;
const tick = async () => { await act(async () => vi.advanceTimersByTimeAsync(3000)); };

it('renders the same seconds-based date inside an expanded recording group without altering its identity', async () => {
  const saved = { ...recording('dated'), completedAt: savedAtSeconds }, value = { ...group('dated', [saved]), latestCompletedAt: savedAtSeconds };
  groupedApi([value]); api.groupMembers = vi.fn().mockResolvedValue({ status: 'ready', group: value, items: [saved], cursor: null });
  await render(); await click(heading('dated'));
  expect(panel().querySelector('.platform-media-file-bottom time')).toHaveTextContent(expectedSavedDate());
  expect(panel().querySelector('.platform-media-file-bottom time')).not.toHaveTextContent('1970');
  expect(value.latestCompletedAt).toBe(savedAtSeconds); expect(saved.completedAt).toBe(savedAtSeconds);
  expect(readOpenMediaGroup('youtube')).toBe('dated');
});
it('uses whole-catalog summaries instead of grouping a truncated flat page', async () => {
  groupedApi(); await render();
  expect(container.querySelectorAll('.platform-media-group')).toHaveLength(2);
  expect(container.querySelectorAll('article')).toHaveLength(0);
  expect(api.library).not.toHaveBeenCalled(); expect(api.groupMembers).not.toHaveBeenCalled();
  expect(heading('a')).toHaveTextContent('파일 2개');
  expect(heading('a')).toHaveAttribute('aria-expanded', 'false');
  await click(heading('a')); expect(panel()).not.toBeNull(); expect(panel().querySelectorAll('article')).toHaveLength(2);
  expect(container.querySelectorAll('article')).toHaveLength(0); // Portal, not a pushed grid row.
  await act(async () => heading('a').dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  await act(async () => heading('a').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true })));
  expect(api.open).not.toHaveBeenCalled(); expect(container.querySelector('[role=menu]')).toBeNull();
  expect(container.querySelector('.is-focused')).toBeNull();
});
it('keeps legacy adapters flat even if multiple loaded recordings share an identity', async () => {
  api.library = vi.fn().mockResolvedValue({ items: [recording('a'), recording('b')], cursor: 'unloaded' });
  api.groups = vi.fn(); // Both methods are necessary; no locally invented group.
  await render(); expect(container.querySelectorAll('article')).toHaveLength(2);
  expect(container.querySelector('.platform-media-group')).toBeNull(); expect(api.groups).not.toHaveBeenCalled();
});
it('reports135 files and exposes every member through bounded independent pages', async () => {
  const files = Array.from({ length: 135 }, (_, index) => recording(`file-${index}`)), value = group('a', files);
  groupedApi([value]);
  api.groupMembers = vi.fn().mockImplementation(async (_provider, _key, cursor) => ({ status: 'ready', group: value, items: cursor ? files.slice(100) : files.slice(0, 100), cursor: cursor ? null : 'members-2' }));
  await render(); expect(heading('a')).toHaveTextContent('파일 135개');
  await click(heading('a')); expect(panel()).toHaveTextContent('100 / 135개 파일');
  expect(panel().querySelectorAll('article')).toHaveLength(100);
  await click(panelButton('파일 더 보기')); expect(panel()).toHaveTextContent('135 / 135개 파일');
  expect(new Set([...panel().querySelectorAll('article')].map(value => value.getAttribute('data-library-id'))).size).toBe(135);
  expect(api.groupMembers).toHaveBeenLastCalledWith('youtube', 'a', 'members-2');
});
it('allows all groups closed and remembers explicit null after revisiting', async () => {
  groupedApi(); await render(); await click(heading('a')); await click(panelButton('녹화 묶음 닫기'));
  expect(panel()).toBeNull(); expect(readOpenMediaGroup('youtube')).toBeNull(); expect(document.activeElement).toBe(heading('a'));
  await act(async () => root.unmount()); root = createRoot(container); await render();
  expect(panel()).toBeNull(); expect(heading('a')).toHaveAttribute('aria-expanded', 'false');
  expect(JSON.parse(localStorage.getItem(mediaGroupStorageKey('youtube'))!)).toEqual({ version: 1, openKey: null });
});
it('does not erase a stored later-page key during empty loading, but opening B replaces it permanently', async () => {
  saveOpenMediaGroup('youtube', 'a'); groupedApi();
  api.groups = vi.fn().mockResolvedValueOnce({ status: 'ready', catalogRevision: 'empty', entries: [], cursor: null });
  vi.useFakeTimers(); await render(); expect(readOpenMediaGroup('youtube')).toBe('a');
  vi.mocked(api.groups).mockResolvedValue({ status: 'ready', catalogRevision: 'loaded', entries: [group('b')], cursor: 'p2' });
  await tick(); expect(panel()).toBeNull(); expect(readOpenMediaGroup('youtube')).toBe('a');
  await click(heading('b')); expect(readOpenMediaGroup('youtube')).toBe('b');
  vi.mocked(api.groups).mockImplementation(async (_provider, cursor) => cursor ? { status: 'ready', catalogRevision: 'loaded', entries: [group('a')], cursor: null } : { status: 'ready', catalogRevision: 'loaded', entries: [group('b')], cursor: 'p2' });
  await click(button('더 보기')); expect(heading('a')).toHaveAttribute('aria-expanded', 'false');
  expect(heading('b')).toHaveAttribute('aria-expanded', 'true');
  await click(panelButton('녹화 묶음 닫기')); await tick(); expect(panel()).toBeNull();
});
it('adopts a fresh first-member count and files in one render and keeps the count after closing', async () => {
  groupedApi(); let finish!: (page: MediaGroupMembersPage) => void;
  api.groupMembers = vi.fn().mockReturnValue(new Promise(resolve => { finish = resolve; }));
  await render(); await click(heading('a'));
  expect(heading('a')).toHaveTextContent('파일 2개'); expect(panel().querySelector('article')).toBeNull();
  const files = [recording('current')];
  await act(async () => finish({ status: 'ready', group: { ...group('a', files), groupRevision: 'changed' }, items: files, cursor: null }));
  expect(heading('a')).toHaveTextContent('파일 1개'); expect(panel()).toHaveTextContent('1 / 1개 파일');
  expect(panel().querySelector('article')).toHaveTextContent('저장 영상 current');
  await click(panelButton('녹화 묶음 닫기')); expect(heading('a')).toHaveTextContent('파일 1개');
});
it.each(['archive_limit', 'status_unknown'] as const)('refreshes %s sidecar warnings on an unchanged catalog and member revision', async reason => {
  vi.useFakeTimers(); groupedApi(); await render(); await click(heading('a'));
  expect(panel().querySelector('.platform-media-chat-warning')).toBeNull();
  api.groupMembers = vi.fn().mockResolvedValue({ status: 'ready', group: group('a'), items: [{ ...recording('a-1'), chatArchive: { state: 'partial', reasons: [reason], originalsRetained: true, assetsIncomplete: false } }, recording('a-2')], cursor: null });
  const label = reason === 'archive_limit' ? '채팅 일부 누락' : '채팅 상태 확인 필요';
  await tick(); expect(panel().querySelector('.platform-media-chat-warning')).toHaveAttribute('aria-label', `${label} · 원본 보존`);
  expect(heading('a').querySelector('.platform-media-chat-warning')).toBeNull();
  expect(panel().querySelectorAll('.platform-media-chat-warning')).toHaveLength(1);
});
it('stages two-page catalog refreshes atomically and keeps polls single-flight', async () => {
  vi.useFakeTimers(); groupedApi();
  api.groups = vi.fn().mockImplementation(async (_provider, cursor) => ({ status: 'ready', catalogRevision: 'one', entries: [group(cursor ? 'tail' : 'a')], cursor: cursor ? null : 'p2' }));
  await render(); await click(button('더 보기')); expect(heading('tail')).not.toBeNull();
  let finish!: (page: MediaGroupsPage) => void;
  vi.mocked(api.groups).mockResolvedValueOnce({ status: 'ready', catalogRevision: 'two', entries: [group('new')], cursor: 'new-p2' }).mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  await tick(); const count = vi.mocked(api.groups).mock.calls.length;
  expect(heading('new')).toBeNull(); expect(heading('tail')).not.toBeNull();
  await tick(); await tick(); expect(api.groups).toHaveBeenCalledTimes(count);
  await act(async () => finish({ status: 'ready', catalogRevision: 'two', entries: [group('next-tail')], cursor: null }));
  expect(heading('new')).not.toBeNull(); expect(heading('next-tail')).not.toBeNull(); expect(heading('tail')).toBeNull();
});
it('defers a changed catalog to a later poll or explicit retry without recursive requests', async () => {
  vi.useFakeTimers(); groupedApi(); await render();
  vi.mocked(api.groups!).mockResolvedValueOnce({ status: 'changed', catalogRevision: 'two' });
  await tick(); expect(heading('a')).not.toBeNull(); expect(container).toHaveTextContent('목록이 변경');
  const calls = vi.mocked(api.groups!).mock.calls.length;
  await act(async () => Promise.resolve()); expect(api.groups).toHaveBeenCalledTimes(calls);
  await click(button('다시 확인')); expect(api.groups).toHaveBeenCalledTimes(calls + 1); expect(container.querySelector('[role=alert]')).toBeNull();
});
it('rejects an old member response after close/reopen of the same key', async () => {
  groupedApi(); let old!: (page: MediaGroupMembersPage) => void, latest!: (page: MediaGroupMembersPage) => void;
  api.groupMembers = vi.fn().mockReturnValueOnce(new Promise(resolve => { old = resolve; })).mockReturnValueOnce(new Promise(resolve => { latest = resolve; }));
  await render(); await click(heading('a')); await click(heading('a')); await click(heading('a'));
  await act(async () => old({ status: 'ready', group: group('a'), items: [recording('stale'), recording('old')], cursor: null }));
  expect(panel().querySelector('article')).toBeNull(); expect(api.groupMembers).toHaveBeenCalledTimes(2);
  await act(async () => latest({ status: 'ready', group: group('a', [recording('new')]), items: [recording('new')], cursor: null }));
  expect(panel()).toHaveTextContent('저장 영상 new'); expect(panel()).not.toHaveTextContent('stale');
});
it('selects only visible loaded files and prunes hidden selections without replacing playback', async () => {
  groupedApi(); await render(); await click(heading('a'));
  const cards = panel().querySelectorAll('article'); await click(cards[0]!); await click(cards[1]!, { shiftKey: true });
  expect(container.querySelector('.media-selection')).toHaveTextContent('2개 선택');
  await act(async () => cards[0]!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const node = container.querySelector('video'); expect(node).not.toBeNull();
  await click(heading('a')); expect(container.querySelector('.media-selection')).toBeNull(); expect(container.querySelector('video')).toBe(node); expect(api.close).not.toHaveBeenCalled();
  await click(heading('b')); expect(container.querySelector('video')).toBe(node);
  await render(false); expect(panel()).toBeNull(); expect(container.querySelector('video')).toBe(node);
});
it('limits Ctrl+A to loaded members plus VOD and discards the hidden range anchor on collapse', async () => {
  groupedApi();
  vi.mocked(api.groups!).mockResolvedValue({ status: 'ready', catalogRevision: 'one', entries: [group('a'), { type: 'video', key: 'video-v', item: item('v') }, group('b')], cursor: null });
  await render(); await click(heading('a'));
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true, bubbles: true, cancelable: true })));
  expect(container.querySelector('.media-selection')).toHaveTextContent('3개 선택');
  expect(api.groupMembers).toHaveBeenCalledTimes(1);
  await click(heading('a')); expect(container.querySelector('.media-selection')).toBeNull();
  expect(container.querySelector('article')).toHaveClass('is-focused');
  await click(heading('b')); await click(panel().querySelector('article')!, { shiftKey: true });
  expect(panel().querySelectorAll('.is-focused')).toHaveLength(1);
  expect(container.querySelector('article')).not.toHaveClass('is-focused');
});
it('discards pending member pages when switching providers and keeps provider preferences separate', async () => {
  groupedApi(); let finish!: (page: MediaGroupMembersPage) => void;
  api.groupMembers = vi.fn().mockReturnValueOnce(new Promise(resolve => { finish = resolve; })).mockResolvedValue({ status: 'ready', group: group('b'), items: [recording('b-1'), recording('b-2')], cursor: null });
  await render(); await click(heading('a')); saveOpenMediaGroup('soop', 'b');
  await act(async () => root.render(<PlatformMediaLibrary provider="soop" active privacyMode={false} api={api} />));
  await act(async () => finish({ status: 'ready', group: group('a'), items: [recording('stale'), recording('old')], cursor: null }));
  expect(heading('b')).toHaveAttribute('aria-expanded', 'true'); expect(panel()).not.toHaveTextContent('stale');
  expect(api.groupMembers).toHaveBeenLastCalledWith('soop', 'b', undefined);
  expect(readOpenMediaGroup('youtube')).toBe('a'); expect(readOpenMediaGroup('soop')).toBe('b');
});
it('keeps loaded member pages unchanged if a later page reports a new revision', async () => {
  const files = [recording('one'), recording('two')], value = group('a', files);
  groupedApi([value]);
  api.groupMembers = vi.fn().mockImplementation(async (_provider, _key, cursor) => cursor ? { status: 'changed', groupRevision: 'new' } : { status: 'ready', group: value, items: files.slice(0, 1), cursor: 'p2' });
  await render(); await click(heading('a')); await click(panelButton('파일 더 보기'));
  expect(panel().querySelectorAll('article')).toHaveLength(1); expect(panel()).toHaveTextContent('녹화 파일 목록이 변경');
  expect(api.groupMembers).toHaveBeenCalledTimes(3); expect(heading('a')).toHaveTextContent('파일 2개');
});
it('uses only actual selected file ids and revisions for deletion and removes a missing group', async () => {
  groupedApi(); await render(); await click(heading('a'));
  const cards = panel().querySelectorAll('article'); await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
  await click(button('삭제'));
  api.groupMembers = vi.fn().mockResolvedValue({ status: 'missing' });
  await click(container.querySelector('.platform-media-confirm .danger')!);
  expect(api.delete).toHaveBeenCalledTimes(2); expect(api.delete).toHaveBeenCalledWith('a-1', 6); expect(api.delete).toHaveBeenCalledWith('a-2', 6);
  expect(heading('a')).toBeNull(); expect(panel()).toBeNull(); expect(readOpenMediaGroup('youtube')).toBeNull();
});
it('does not start new reads during a multi-delete or resurrect its already deleted first file', async () => {
  vi.useFakeTimers(); groupedApi(); await render(); await click(heading('a'));
  const cards = panel().querySelectorAll('article'); await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
  let finish!: () => void;
  api.delete = vi.fn().mockResolvedValueOnce(undefined).mockReturnValueOnce(new Promise<void>(resolve => { finish = resolve; }));
  await click(button('삭제')); await click(container.querySelector('.platform-media-confirm .danger')!);
  expect(api.delete).toHaveBeenCalledTimes(2); expect(panel().querySelector('[data-library-id="a-1"]')).toBeNull();
  const reads = vi.mocked(api.groupMembers!).mock.calls.length;
  await click(heading('a')); await click(heading('a')); await tick();
  expect(api.groupMembers).toHaveBeenCalledTimes(reads); expect(panel().querySelector('article')).toBeNull();
  api.groupMembers = vi.fn().mockResolvedValue({ status: 'missing' });
  await act(async () => finish());
  expect(api.groupMembers).toHaveBeenCalledExactlyOnceWith('youtube', 'a', undefined);
  expect(heading('a')).toBeNull(); expect(panel()).toBeNull();
});
it('masks group identity and all member cards without changing count or card geometry', async () => {
  groupedApi(); await render(true, true); await click(heading('a'));
  expect(container).not.toHaveTextContent('채널'); expect(panel()).not.toHaveTextContent('채널'); expect(panel().querySelector('img')).toBeNull();
  expect(heading('a')).toHaveTextContent('파일 2개'); expect(panel().querySelectorAll('.platform-media-channel-avatar')).toHaveLength(2);
  expect(api.thumbnail).not.toHaveBeenCalled();
});
it('clears stale channel portraits immediately when a compact group representative changes', async () => {
  vi.useFakeTimers(); groupedApi(); api.profile = vi.fn().mockResolvedValue('data:image/png;base64,AA=='); await render(); expect(heading('a').querySelector('.platform-media-channel-avatar img')).not.toBeNull();
  let finish!: (value: string | null) => void;
  vi.mocked(api.profile!).mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  vi.mocked(api.groups!).mockResolvedValue({ status: 'ready', catalogRevision: 'new', entries: [{ ...group('a'), representative: recording('new') }], cursor: null });
  await tick(); expect(heading('a').querySelector('.platform-media-channel-avatar img')).toBeNull();
  await act(async () => finish('data:image/png;base64,AA==')); expect(heading('a').querySelector('.platform-media-channel-avatar img')).toHaveAttribute('src', 'data:image/png;base64,AA==');
  expect(api.thumbnail).not.toHaveBeenCalled();
});
it('makes departing group content inert while preserving the established close animation', async () => {
  const animations: { done: () => void; cancel: ReturnType<typeof vi.fn> }[] = [];
  const animate = vi.fn().mockImplementation(() => { let done!: () => void; const finished = new Promise<void>(resolve => { done = resolve; }); const cancel = vi.fn(); animations.push({ done, cancel }); return { finished, cancel }; });
  vi.stubGlobal('matchMedia', () => ({ matches: false }));
  Object.defineProperty(HTMLElement.prototype, 'animate', { configurable: true, value: animate });
  try {
    groupedApi(); await render(); await click(heading('a')); await click(panelButton('녹화 묶음 닫기'));
    expect(heading('a')).toHaveAttribute('aria-expanded', 'false');
    expect(panel().closest('.window-motion')).toHaveAttribute('inert'); expect(panel().closest('.window-motion')).toHaveAttribute('aria-hidden', 'true');
    expect(readOpenMediaGroup('youtube')).toBeNull();
    await act(async () => animations.at(-1)!.done()); expect(panel()).toBeNull();
  } finally { delete (HTMLElement.prototype as { animate?: unknown }).animate; }
});
it('repositions the real portal on viewport resize and ancestor scrolling, then removes inactive listeners', async () => {
  let rect = { left: 300, top: 60, bottom: 300, width: 260 }, measures = 0;
  const original = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('platform-media-group-heading')) { measures++; return { ...rect, right: rect.left + rect.width, x: rect.left, y: rect.top, height: rect.bottom - rect.top, toJSON: () => ({}) } as DOMRect; }
    return original.call(this);
  });
  groupedApi(); await render(); await click(heading('a'));
  const expected = () => groupPanelPosition(rect, window.innerWidth, window.innerHeight);
  expect(panel().style.left).toBe(`${expected().left}px`); expect(panel().style.top).toBe(`${expected().top}px`);
  rect = { left: 60, top: 390, bottom: 620, width: 260 };
  await act(async () => window.dispatchEvent(new Event('resize')));
  expect(panel().style.left).toBe(`${expected().left}px`); expect(panel().style.top).toBe(`${expected().top}px`);
  rect = { left: 60, top: 200, bottom: 430, width: 260 };
  await act(async () => container.querySelector('.platform-media-library')!.dispatchEvent(new Event('scroll')));
  expect(panel().style.top).toBe(`${expected().top}px`);
  await render(false); const previousMeasures = measures;
  await act(async () => { window.dispatchEvent(new Event('resize')); document.dispatchEvent(new Event('scroll')); });
  expect(measures).toBe(previousMeasures); expect(panel()).toBeNull();
});
it('moves the portal with a changed jobs block before a pending member refresh completes', async () => {
  vi.useFakeTimers(); let top = 40;
  const original = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    return this.classList.contains('platform-media-group-heading') ? { left: 20, top, bottom: top + 200, width: 250, height: 200, right: 270, x: 20, y: top, toJSON: () => ({}) } as DOMRect : original.call(this);
  });
  const observed: Element[] = [], callbacks: ResizeObserverCallback[] = [];
  vi.stubGlobal('ResizeObserver', class { constructor(callback: ResizeObserverCallback) { callbacks.push(callback); } observe(node: Element) { observed.push(node); } disconnect() {} });
  groupedApi(); await render(); await click(heading('a'));
  const firstTop = panel().style.top;
  let finish!: (page: MediaGroupMembersPage) => void;
  vi.mocked(api.groupMembers!).mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  vi.mocked(api.jobs).mockResolvedValue([{ jobId: 'job', requestId: 'job', reference: item('v').reference, revision: 1, state: 'downloading', title: '새 작업', channelName: '', createdAt: 1, receivedBytes: 1000 }]);
  top = 360; await tick();
  expect(container.querySelector('.platform-media-work')).not.toBeNull(); expect(panel().style.top).not.toBe(firstTop);
  expect(panel().style.top).toBe(`${groupPanelPosition({ left: 20, top, bottom: top + 200, width: 250 }, innerWidth, innerHeight).top}px`);
  expect(observed).toContain(container.querySelector('.platform-media-work'));
  top = 200;
  await act(async () => callbacks.at(-1)!([], {} as ResizeObserver));
  expect(panel().style.top).toBe(`${groupPanelPosition({ left: 20, top, bottom: top + 200, width: 250 }, innerWidth, innerHeight).top}px`);
  await act(async () => finish({ status: 'ready', group: group('a'), items: [recording('a-1'), recording('a-2')], cursor: null }));
});
it('immediately discards an unmasked departing portal on privacy without remounting playback', async () => {
  const previous = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'animate');
  Object.defineProperty(HTMLElement.prototype, 'animate', { configurable: true, value: vi.fn().mockImplementation(() => ({ finished: new Promise<void>(() => undefined), cancel: vi.fn() })) });
  vi.stubGlobal('matchMedia', () => ({ matches: false }));
  try {
    groupedApi(); await render(); await click(heading('a'));
    await act(async () => panel().querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    const node = container.querySelector('video');
    await click(heading('a')); expect(panel()).toHaveTextContent('채널');
    expect(panel().closest('.window-motion')).toHaveAttribute('inert');
    await render(true, true); expect(panel()).toBeNull(); expect(container.querySelector('video')).toBe(node); expect(api.close).not.toHaveBeenCalled();
    await click(heading('a')); expect(panel()).not.toHaveTextContent('채널'); expect(panel()).toHaveTextContent('프라이버시 모드'); expect(panel().querySelector('img')).toBeNull();
    await render(true, false); expect(panel()).toHaveTextContent('채널');
    await render(true, true); expect(panel()).not.toHaveTextContent('채널'); expect(panel().querySelector('img')).toBeNull();
    expect(container.querySelector('video')).toBe(node); expect(api.close).not.toHaveBeenCalled();
  } finally { if (previous) Object.defineProperty(HTMLElement.prototype, 'animate', previous); else delete (HTMLElement.prototype as { animate?: unknown }).animate; }
});
