import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PlatformScreenshotLibrary } from './PlatformScreenshotLibrary';
import type { PlatformScreenshotItem, PlatformScreenshotPage, PlatformScreenshotsApi } from './platformScreenshots';
import type { MediaProvider } from './platformMedia';

const png = 'data:image/png;base64,AA==';
const item = (id: string, provider: MediaProvider = 'youtube'): PlatformScreenshotItem => ({ id, provider, reference: { provider, kind: 'live', externalId: 'fixture' }, title: `Original | 저장 화면 ${id}`, channelName: '보관된 채널', fileName: `${id}.png`, createdAt: 1_791_800_000_000, bytes: 1000, width: 1920, height: 1080 });
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; };
let container: HTMLDivElement, root: Root, api: PlatformScreenshotsApi;
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
  api = {
    begin: vi.fn(), chunk: vi.fn(), finish: vi.fn(), abort: vi.fn(), undo: vi.fn(),
    list: vi.fn().mockResolvedValue({ items: [item('a'), item('b')], cursor: null }),
    image: vi.fn().mockResolvedValue(png), folder: vi.fn().mockResolvedValue(undefined),
    delete: vi.fn().mockImplementation(async (_provider, ids: string[]) => ({ deleted: ids, failed: [] })),
  };
});
afterEach(async () => {
  await act(async () => root.unmount()); container.remove(); delete document.documentElement.dataset.tutorialOpen;
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});
const render = async (options: { active?: boolean; privacyMode?: boolean; provider?: MediaProvider } = {}) => {
  await act(async () => root.render(<PlatformScreenshotLibrary provider={options.provider ?? 'youtube'} active={options.active ?? true} privacyMode={options.privacyMode ?? false} api={api} />));
};
const cards = () => [...container.querySelectorAll<HTMLElement>('article')];
const button = (label: string, scope: ParentNode = document) => [...scope.querySelectorAll<HTMLButtonElement>('button')].find(node => node.textContent === label || node.getAttribute('aria-label') === label)!;
const click = async (node: Element, options: MouseEventInit = {}) => { await act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true, ...options }))); };
const double = async (node: Element) => { await act(async () => node.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))); };
const key = async (value: string, options: KeyboardEventInit = {}) => { await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true, ...options }))); };
const context = async (node: Element) => { await act(async () => node.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 100, clientY: 100 }))); };

it('uses local stored metadata, lazy thumbnails and Korean-first titles without remote lookup', async () => {
  await render();
  expect(api.list).toHaveBeenCalledWith('youtube', null, 24);
  expect(cards()).toHaveLength(2); expect(cards()[0]!.querySelector('.platform-shot-card-caption strong')).toHaveAttribute('title', '저장 화면 a'); expect(cards()[0]).toHaveTextContent('보관된 채널');
  expect(cards()[0]).not.toHaveTextContent('Original'); expect(cards()[0]).toHaveTextContent('1920×1080');
  expect(api.image).toHaveBeenCalledWith('youtube', 'a', false); expect(container.querySelectorAll('img')).toHaveLength(2);
  expect(button('저장 폴더 열기')).toBeDisabled(); expect(api.folder).not.toHaveBeenCalled();
});
it('does not select by checkbox on the first click and uses independent additive Ctrl/Shift selection', async () => {
  api.list = vi.fn().mockResolvedValue({ items: ['a', 'b', 'c', 'd', 'e'].map(id => item(id)), cursor: null }); await render();
  await click(cards()[0]!); expect(container.querySelector('.platform-shot-check')).toBeNull();
  await click(cards()[4]!, { ctrlKey: true }); await click(cards()[2]!, { shiftKey: true });
  expect(cards().map(card => card.classList.contains('is-focused'))).toEqual([true, false, true, true, true]);
  await click(cards()[3]!, { shiftKey: true }); expect(container.querySelector('.platform-shot-selection')).toHaveTextContent('4개 선택');
  expect(api.image).not.toHaveBeenCalledWith('youtube', 'a', true);
});
it('clears and blurs repeated solo selection; Ctrl deselect preserves unrelated cards', async () => {
  await render(); const card = cards()[0]!; card.focus(); await click(card); await click(card);
  expect(card).not.toHaveClass('is-focused'); expect(document.activeElement).not.toBe(card);
  await click(card); await click(cards()[1]!, { metaKey: true }); await click(card, { ctrlKey: true });
  expect(cards()[1]).toHaveClass('is-focused'); expect(container.querySelector('.platform-shot-check')).toBeNull();
});
it('opens one full image on realistic double-click and contains the independent local viewer', async () => {
  await render(); const card = cards()[0]!; await click(card, { detail: 1 }); await click(card, { detail: 2 }); await double(card);
  expect(api.image).toHaveBeenCalledWith('youtube', 'a', true);
  expect(vi.mocked(api.image).mock.calls.filter(call => call[2])).toHaveLength(1);
  expect(document.querySelector('[role=dialog]')).toHaveTextContent('보관된 채널');
  expect(document.querySelector('.platform-shot-original img')).toHaveAttribute('src', png);
  expect(container.querySelector('.platform-shot-selection')).toBeNull();
  await click(button('스크린샷 닫기')); expect(document.querySelector('[role=dialog]')).toBeNull();
});
it('moves through loaded images with arrows without fetching the next list or wrapping at edges', async () => {
  await render(); await double(cards()[0]!); expect(button('이전 스크린샷')).toBeDisabled();
  await key('ArrowRight'); expect(api.image).toHaveBeenCalledWith('youtube', 'b', true); expect(button('다음 스크린샷')).toBeDisabled();
  const count = vi.mocked(api.image).mock.calls.length; await key('ArrowRight'); expect(api.image).toHaveBeenCalledTimes(count); expect(api.list).toHaveBeenCalledTimes(1);
});
it('opens the keyboard-focused card rather than a different selected card', async () => {
  await render(); await click(cards()[0]!); const focused = cards()[1]!; focused.focus();
  await act(async () => focused.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })));
  expect(api.image).toHaveBeenCalledWith('youtube', 'b', true);
  expect(api.image).not.toHaveBeenCalledWith('youtube', 'a', true);
  expect(document.querySelector('[role=dialog]')).toHaveTextContent('저장 화면 b');
});
it('requires delete confirmation and uses only the selected batch, retaining failed entries', async () => {
  api.delete = vi.fn().mockResolvedValue({ deleted: ['a'], failed: [{ id: 'b', error: '파일이 변경됐습니다.' }] });
  await render(); await click(cards()[0]!); await click(cards()[1]!, { ctrlKey: true }); await click(button('삭제', container));
  expect(api.delete).not.toHaveBeenCalled(); const dialog = document.querySelector('[role=alertdialog]')!; expect(dialog).toHaveTextContent('2개 스크린샷');
  await click(button('삭제', dialog)); expect(api.delete).toHaveBeenCalledWith('youtube', ['a', 'b']);
  expect(cards()).toHaveLength(1); expect(cards()[0]).toHaveAttribute('data-screenshot-id', 'b'); expect(container).toHaveTextContent('1개 삭제 실패');
  expect(api.list).toHaveBeenCalledTimes(1);
});
it('keeps context selection on a member of the existing batch and deletes no unrelated entry', async () => {
  api.list = vi.fn().mockResolvedValue({ items: ['a', 'b', 'c'].map(id => item(id)), cursor: null });
  await render(); await click(cards()[0]!); await click(cards()[1]!, { ctrlKey: true }); await context(cards()[0]!);
  await click(button('삭제', document.querySelector('[role=menu]')!)); await click(button('삭제', document.querySelector('[role=alertdialog]')!));
  expect(api.delete).toHaveBeenCalledWith('youtube', ['a', 'b']); expect(cards()[0]).toHaveAttribute('data-screenshot-id', 'c');
});
it('uses the context target for folder action and a right-click outside the batch replaces selection', async () => {
  await render(); await click(cards()[0]!); await context(cards()[1]!); await click(button('폴더 열기', document.querySelector('[role=menu]')!));
  expect(api.folder).toHaveBeenCalledWith('youtube', 'b'); expect(cards()[0]).not.toHaveClass('is-focused'); expect(cards()[1]).toHaveClass('is-focused');
});
it('supports keyboard selection, deselection, open and delete without stealing foreign modal keys', async () => {
  await render(); await key('a', { ctrlKey: true }); expect(container.querySelector('.platform-shot-selection')).toHaveTextContent('2개 선택');
  await key('a', { ctrlKey: true, shiftKey: true }); expect(container.querySelector('.platform-shot-selection')).toBeNull();
  await click(cards()[0]!); document.documentElement.dataset.tutorialOpen = 'true'; await key('Delete'); expect(document.querySelector('[role=alertdialog]')).toBeNull();
  delete document.documentElement.dataset.tutorialOpen; await key('Enter'); expect(document.querySelector('[role=dialog]')).not.toBeNull();
  await key('Delete'); expect(document.querySelector('[role=alertdialog]')).toBeNull(); await key('Escape'); await key('Delete'); expect(document.querySelector('[role=alertdialog]')).not.toBeNull();
});
it('loads further pages explicitly, deduplicates rows and preserves selection', async () => {
  api.list = vi.fn().mockResolvedValueOnce({ items: [item('a')], cursor: 'next' }).mockResolvedValueOnce({ items: [item('a'), item('b')], cursor: null });
  await render(); await click(cards()[0]!); await click(button('더 보기'));
  expect(api.list).toHaveBeenLastCalledWith('youtube', 'next', 24); expect(cards()).toHaveLength(2); expect(cards()[0]).toHaveClass('is-focused');
});
it('refreshes all loaded pages atomically and keeps the old visible list on later-page failure', async () => {
  api.list = vi.fn().mockResolvedValueOnce({ items: [item('a')], cursor: 'page2' }).mockResolvedValueOnce({ items: [item('b')], cursor: null }).mockResolvedValueOnce({ items: [item('c')], cursor: 'new-page2' }).mockRejectedValueOnce(new Error('오프라인'));
  await render(); await click(button('더 보기')); await click(button('새로고침'));
  expect(cards().map(card => card.dataset.screenshotId)).toEqual(['a', 'b']); expect(container).toHaveTextContent('오프라인');
});
it('does not overlap list calls and discards late responses after provider change', async () => {
  const old = deferred<PlatformScreenshotPage>(); api.list = vi.fn().mockReturnValueOnce(old.promise).mockResolvedValueOnce({ items: [item('soop', 'soop')], cursor: null });
  await render(); expect(container.querySelector('[aria-label="불러오는 중"]')).not.toBeNull();
  await render({ provider: 'soop' }); await act(async () => old.resolve({ items: [item('late')], cursor: null }));
  expect(cards().map(card => card.dataset.screenshotId)).toEqual(['soop']);
});
it('hides metadata and prevents list/image/folder/preview work in privacy', async () => {
  await render({ privacyMode: true }); expect(api.list).not.toHaveBeenCalled(); expect(api.image).not.toHaveBeenCalled();
  await render(); await double(cards()[0]!); await render({ privacyMode: true });
  expect(document.querySelector('[role=dialog]')).toBeNull(); expect(document.querySelector('img')).toBeNull(); expect(container).not.toHaveTextContent('보관된 채널'); expect(container).not.toHaveTextContent('저장 화면');
  const reads = vi.mocked(api.image).mock.calls.length; await double(cards()[0]!); await context(cards()[0]!); expect(api.image).toHaveBeenCalledTimes(reads); expect(document.querySelector('[role=menu]')).toBeNull();
});
it('does not reveal a late original result after privacy or inactive route changes', async () => {
  const original = deferred<string>(); api.image = vi.fn().mockImplementation((_provider, _id, full) => full ? original.promise : Promise.resolve(png));
  await render(); await double(cards()[0]!); await render({ active: false }); await act(async () => original.resolve(png));
  expect(document.querySelector('.platform-shot-original')).toBeNull(); expect(container.querySelector('section')).toHaveAttribute('hidden');
});
it('rejects remote, vector or oversized image responses rather than exposing native strings', async () => {
  api.image = vi.fn().mockResolvedValueOnce('https://example.com/image.png').mockResolvedValueOnce('data:image/svg+xml;base64,AA==');
  await render(); expect(container.querySelector('img')).toBeNull(); expect(container).toHaveTextContent('이미지를 열 수 없습니다.');
});
it('uses the viewport observer and releases thumbnails after they leave it', async () => {
  const observed: { callback: IntersectionObserverCallback; target?: Element }[] = [];
  vi.stubGlobal('IntersectionObserver', class {
    record: { callback: IntersectionObserverCallback; target?: Element };
    constructor(callback: IntersectionObserverCallback) { this.record = { callback }; observed.push(this.record); }
    observe(target: Element) { this.record.target = target; } disconnect() {}
  });
  await render(); expect(api.image).not.toHaveBeenCalled();
  await act(async () => observed[0]!.callback([{ target: observed[0]!.target!, isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver));
  expect(api.image).toHaveBeenCalledTimes(1); expect(container.querySelectorAll('img')).toHaveLength(1);
  await act(async () => observed[0]!.callback([{ target: observed[0]!.target!, isIntersecting: false } as IntersectionObserverEntry], {} as IntersectionObserver));
  expect(container.querySelector('img')).toBeNull();
});
it('limits image reads to two including pending reads and cancels queued work on privacy', async () => {
  const first = deferred<string>(), second = deferred<string>();
  api.list = vi.fn().mockResolvedValue({ items: ['a', 'b', 'c'].map(id => item(id)), cursor: null }); api.image = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise).mockResolvedValue(png);
  await render(); expect(api.image).toHaveBeenCalledTimes(2); await render({ privacyMode: true });
  await act(async () => { first.resolve(png); second.resolve(png); }); expect(api.image).toHaveBeenCalledTimes(2); expect(container.querySelector('img')).toBeNull();
});
it('cannot restore a deleted screenshot from an older refresh response', async () => {
  const stale = deferred<PlatformScreenshotPage>(); api.list = vi.fn().mockResolvedValueOnce({ items: [item('a'), item('b')], cursor: null }).mockReturnValueOnce(stale.promise);
  await render(); await click(cards()[0]!); await click(button('새로고침')); await key('Delete'); await click(button('삭제', document.querySelector('[role=alertdialog]')!));
  await act(async () => stale.resolve({ items: [item('a'), item('b')], cursor: null }));
  expect(cards().map(card => card.dataset.screenshotId)).toEqual(['b']);
});
it('locks destructive confirmation while the authorized deletion is running', async () => {
  const deletion = deferred<{ deleted: string[]; failed: [] }>(); api.delete = vi.fn().mockReturnValue(deletion.promise);
  await render(); await click(cards()[0]!); await key('Delete'); const dialog = document.querySelector('[role=alertdialog]')!;
  await click(button('삭제', dialog)); await key('Escape'); expect(document.querySelector('[role=alertdialog]')).not.toBeNull(); expect(button('취소', dialog)).toBeDisabled();
  await act(async () => deletion.resolve({ deleted: ['a'], failed: [] })); expect(document.querySelector('[role=alertdialog]')).toBeNull();
});
