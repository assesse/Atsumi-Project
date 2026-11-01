import { act, type ComponentProps } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { XMediaSearchPanel } from './XMediaSearchPanel';
import type { XCreatorLinks, XMediaSearchApi, XProviderResult, XSearchSnapshot } from './xMediaSearchApi';

const ID = '11111111-1111-4111-8111-111111111111';
const NEXT = '22222222-2222-4222-8222-222222222222';
const SOURCE = 'https://x.com/fixture/status/123456789';
const RESULT = { title: 'Observed candidate', creator: null, similarity: 53.25, thumbnailData: null, sourceMetadata: [], links: [{ label: 'Observed source', url: 'https://example.org/observed' }] };
const profile = (name = 'fixture'): XCreatorLinks => ({ profileUrl: `https://x.com/${name}`, observedAt: '2026-10-24T12:00:00Z', message: 'Profile-declared link, separate from similarity candidates.', links: [{ label: name, url: 'https://www.pixiv.net/users/123' }] });
const snapshot = (change: Partial<XSearchSnapshot> = {}): XSearchSnapshot => ({ id: ID, source: 'surface', archiveId: null, assetName: null, localContext: null, generation: 7, accountId: 'legacy', feed: 'foryou', pane: 'feed', kind: 'image', provider: 'saucenao', phase: 'consent', imageUrl: 'https://pbs.twimg.com/media/Fixture?format=jpg&name=small', previewData: null, message: '', candidates: [], originalPageState: 'unavailable', sourcePostUrl: SOURCE, providers: [], ...change });
function dual(sauce: Partial<XProviderResult> = {}, lens: Partial<XProviderResult> = {}, change: Partial<XSearchSnapshot> = {}): XSearchSnapshot {
  const providers: XProviderResult[] = [{ provider: 'saucenao', phase: 'searching', message: '', candidates: [], originalPageState: 'available', ...sauce }, { provider: 'lens', phase: 'searching', message: '', candidates: [], originalPageState: 'available', ...lens }];
  const phase = providers.some(item => item.phase === 'searching') ? 'searching' : providers.some(item => item.phase === 'verification') ? 'verification' : providers.every(item => item.phase === 'failed') ? 'failed' : 'complete';
  return snapshot({ phase, providers, ...change });
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function fixture(initial = snapshot()) {
  let handler: ((value: XSearchSnapshot | null) => void) | undefined;
  const pending = deferred<XCreatorLinks>();
  const api: XMediaSearchApi = {
    status: vi.fn().mockResolvedValue(initial), subscribe: vi.fn(async next => { handler = next; return () => {}; }),
    confirm: vi.fn().mockResolvedValue(dual()), resume: vi.fn().mockResolvedValue(dual({ phase: 'complete', candidates: [RESULT] })),
    cancel: vi.fn().mockResolvedValue(undefined), resetConsent: vi.fn().mockResolvedValue(undefined),
    openResult: vi.fn().mockResolvedValue(undefined), openOriginal: vi.fn().mockResolvedValue(undefined), startArchive: vi.fn().mockResolvedValue(undefined),
    openLensSettings: vi.fn().mockResolvedValue(undefined), refreshLensResults: vi.fn().mockResolvedValue(initial),
    creatorLinks: vi.fn().mockReturnValue(pending.promise), openCreatorLink: vi.fn().mockResolvedValue(undefined),
  };
  return { api, pending, emit: (value: XSearchSnapshot | null) => act(async () => handler?.(value)) };
}
let host: HTMLDivElement, root: Root;
beforeEach(() => { vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const render = (api: XMediaSearchApi, props: Partial<ComponentProps<typeof XMediaSearchPanel>> = {}) => act(async () => root.render(<XMediaSearchPanel runtime="browser-mock" active privacyMode={false} accountId="legacy" generation={7} feed="foryou" onNotice={() => {}} api={api} {...props} />));
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');
const group = () => dialog()?.querySelector<HTMLElement>('.x-media-search-creator');
const button = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(node => node.textContent === name || node.getAttribute('aria-label') === name)!;
const click = (name: string) => act(async () => button(name).click());

it('keeps one pending original lookup through consent, searching, verification and completion without blocking provider actions', async () => {
  const test = fixture();
  await render(test.api);
  expect(test.api.creatorLinks).toHaveBeenCalledExactlyOnceWith(ID);
  expect(button('확인 후 전송')).not.toBeDisabled();
  await click('확인 후 전송');
  expect(test.api.confirm).toHaveBeenCalledExactlyOnceWith(ID, false);
  await test.emit(dual({ phase: 'complete', candidates: [RESULT] }, { phase: 'verification' }));
  expect(group()).toHaveAttribute('aria-busy', 'true');
  const sourceButton = dialog()!.querySelector<HTMLButtonElement>('button[title="https://example.org/observed"]')!;
  expect(sourceButton).not.toBeDisabled();
  await act(async () => sourceButton.click());
  expect(test.api.openResult).toHaveBeenCalledExactlyOnceWith(ID, 0, 0, 'saucenao');
  await click('SauceNAO 원본 검색 화면');
  expect(test.api.openOriginal).toHaveBeenCalledExactlyOnceWith(ID, 'saucenao');
  await click('확인 완료 후 계속');
  expect(test.api.resume).toHaveBeenCalledExactlyOnceWith(ID, 'lens');
  await test.emit(dual({ phase: 'complete', candidates: [RESULT] }, { phase: 'complete' }));
  await act(async () => test.pending.resolve(profile()));
  expect(group()).toHaveAttribute('aria-busy', 'false');
  expect(group()).toHaveTextContent('fixture');
  expect(dialog()).toHaveTextContent('Observed candidate');
  await test.emit(dual({ phase: 'complete', candidates: [RESULT], originalPageState: 'expired' }, { phase: 'complete', originalPageState: 'closed' }));
  expect(test.api.creatorLinks).toHaveBeenCalledOnce();
  expect(test.api.openCreatorLink).not.toHaveBeenCalled();
});

it('never queries an unrelated candidate author when original provenance is missing', async () => {
  const weak = { ...RESULT, links: [{ label: 'X candidate', url: 'https://x.com/unrelated/status/123456789' }] };
  const test = fixture(dual({ phase: 'complete', candidates: [weak] }, { phase: 'complete' }, { sourcePostUrl: null }));
  await render(test.api);
  expect(group()).toBeNull();
  expect(test.api.creatorLinks).not.toHaveBeenCalled();
  expect(test.api.openResult).not.toHaveBeenCalled();
});

it.each([{ privacyMode: true }, { active: false }, { accountId: 'a'.repeat(32) }, { generation: 8 }, { feed: 'bookmarks' }])('discards pending metadata after context invalidation: %j', async change => {
  const test = fixture();
  await render(test.api);
  vi.mocked(test.api.status).mockResolvedValue(null);
  await render(test.api, change);
  await act(async () => test.pending.resolve(profile('late')));
  expect(dialog()).toBeNull();
  expect(test.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
  expect(test.api.openCreatorLink).not.toHaveBeenCalled();
});

it('does not let a previous job replace the next job metadata while the first response unwinds', async () => {
  const test = fixture();
  await render(test.api);
  await click('검색 창 닫기');
  const second = deferred<XCreatorLinks>();
  vi.mocked(test.api.creatorLinks).mockReturnValue(second.promise);
  await test.emit(snapshot({ id: NEXT, sourcePostUrl: 'https://x.com/next/status/987654321' }));
  await act(async () => second.resolve(profile('next')));
  await act(async () => test.pending.resolve(profile('late')));
  expect(group()).toHaveTextContent('@next');
  expect(group()).not.toHaveTextContent('late');
  expect(test.api.creatorLinks).toHaveBeenCalledTimes(2);
  expect(test.api.creatorLinks).toHaveBeenNthCalledWith(2, NEXT);
  expect(test.api.cancel).toHaveBeenCalledWith(ID);
});

it('keeps a profile failure separate from both services and does not blindly retry on status updates', async () => {
  const test = fixture(dual({ phase: 'complete', candidates: [RESULT] }, { phase: 'failed', message: 'Lens failed' }));
  await render(test.api);
  await act(async () => test.pending.reject(new Error('Profile unavailable')));
  expect(group()).toHaveTextContent('프로필 링크를 불러오지 못함');
  expect(group()).not.toHaveTextContent('등록된 링크 없음');
  expect(dialog()).toHaveTextContent('Observed candidate');
  await click('Google Lens 원본 검색 화면');
  expect(test.api.openOriginal).toHaveBeenCalledExactlyOnceWith(ID, 'lens');
  await test.emit(dual({ phase: 'complete', candidates: [RESULT] }, { phase: 'failed', message: 'Lens failed', originalPageState: 'closed' }));
  expect(test.api.creatorLinks).toHaveBeenCalledOnce();
  expect(test.api.confirm).not.toHaveBeenCalled();
});

it('cancels a pending source lookup when the selected archive changes without needing an X account', async () => {
  const test = fixture(snapshot({ source: 'archive', archiveId: ID, assetName: 'fixture.jpg', localContext: ID, accountId: null, generation: null, feed: null, pane: 'archive', imageUrl: null, previewData: 'data:image/jpeg;base64,/9j/2Q==' }));
  await render(test.api, { accountId: null, generation: null, localContext: ID });
  expect(test.api.creatorLinks).toHaveBeenCalledExactlyOnceWith(ID);
  vi.mocked(test.api.status).mockResolvedValue(null);
  await render(test.api, { accountId: null, generation: null, localContext: NEXT });
  await act(async () => test.pending.resolve(profile('late')));
  expect(dialog()).toBeNull();
  expect(test.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
});
