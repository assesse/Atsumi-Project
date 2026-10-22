import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PlatformMediaLibrary } from './PlatformMediaLibrary';
import { PlatformScreenshotLibrary } from './PlatformScreenshotLibrary';
import { mediaGroupStorageKey, readOpenMediaGroup } from './mediaGroupPaging';
import type { MediaLibraryItem, MediaProvider, MediaRecordingGroup, PlatformMediaApi } from './platformMedia';
import type { PlatformScreenshotItem, PlatformScreenshotsApi } from './platformScreenshots';
import mediaSource from './PlatformMediaLibrary.tsx?raw';
import screenshotSource from './PlatformScreenshotLibrary.tsx?raw';
import mediaCss from './PlatformMediaLibrary.css?raw';
import screenshotCss from './PlatformScreenshotLibrary.css?raw';

const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
const media = (provider: MediaProvider): MediaLibraryItem => ({ libraryId: 'same-id', revision: 1, reference: { provider, kind: 'video', externalId: 'same-external-id' }, title: `${provider} | ${provider === 'soop' ? '숲' : '영상'} 저장본`, channelName: `${provider} 채널`, bytes: 12345678, duration: 80, completedAt: 1791800000, sha256: 'a'.repeat(64) });
const mediaApi = (provider: MediaProvider): PlatformMediaApi => ({
  download: vi.fn(), jobs: vi.fn().mockResolvedValue([]), cancel: vi.fn(), retry: vi.fn(),
  library: vi.fn().mockResolvedValue({ items: [media(provider)] }), profile: vi.fn().mockResolvedValue(null), thumbnail: vi.fn(),
  open: vi.fn().mockResolvedValue({ token: `${provider}-lease`, url: `http://platform-media.localhost/${provider}-lease/media.mp4`, item: media(provider) }),
  close: vi.fn().mockResolvedValue(undefined), openFolder: vi.fn().mockResolvedValue(undefined), delete: vi.fn().mockResolvedValue(undefined),
});
const shot = (provider: MediaProvider): PlatformScreenshotItem => ({ id: 'same-shot', provider, reference: { provider, kind: 'live', externalId: 'same-external-id' }, title: `${provider} 화면`, channelName: `${provider} 채널`, fileName: 'screen.png', createdAt: 1791800000000, bytes: 50000, width: 1920, height: 1080 });
const shotsApi = (): PlatformScreenshotsApi => ({ begin: vi.fn(), chunk: vi.fn(), finish: vi.fn(), abort: vi.fn(), undo: vi.fn(), list: vi.fn().mockImplementation(async (provider: MediaProvider) => ({ items: [shot(provider)], cursor: null })), image: vi.fn().mockResolvedValue('data:image/png;base64,AA=='), folder: vi.fn().mockResolvedValue(undefined), delete: vi.fn().mockImplementation(async (_provider, ids) => ({ deleted: ids, failed: [] })) });
let host: HTMLDivElement, root: Root;
beforeEach(() => { localStorage.clear(); native.invoke.mockClear(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(); vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => undefined); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const click = async (node: Element) => act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
const button = (scope: ParentNode, label: string) => [...scope.querySelectorAll<HTMLButtonElement>('button')].find(node => node.textContent === label || node.getAttribute('aria-label') === label)!;

it('keeps matching file IDs, selection, folder actions and background leases local to each provider', async () => {
  const youtube = mediaApi('youtube'), soop = mediaApi('soop');
  const render = async (active: MediaProvider) => act(async () => root.render(<>
    <PlatformMediaLibrary provider="youtube" active={active === 'youtube'} privacyMode={false} api={youtube} />
    <PlatformMediaLibrary provider="soop" active={active === 'soop'} privacyMode={false} api={soop} />
  </>));
  await render('youtube');
  const yt = host.querySelector<HTMLElement>('[data-media-provider="youtube"]')!, sp = host.querySelector<HTMLElement>('[data-media-provider="soop"]')!;
  expect(youtube.library).toHaveBeenCalledExactlyOnceWith('youtube'); expect(soop.library).not.toHaveBeenCalled();
  await click(yt.querySelector('article')!); await click(button(yt, '저장 폴더 열기'));
  expect(youtube.openFolder).toHaveBeenCalledExactlyOnceWith('same-id'); expect(soop.openFolder).not.toHaveBeenCalled();
  await act(async () => yt.querySelector('article')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const video = yt.querySelector('video'); expect(video).not.toBeNull();
  await render('soop'); expect(yt).toHaveAttribute('hidden'); expect(soop.library).toHaveBeenCalledExactlyOnceWith('soop');
  expect(sp).toHaveTextContent('soop 채널'); expect(sp).not.toHaveTextContent('youtube 채널');
  expect(sp.querySelector('.is-focused')).toBeNull(); expect(yt.querySelector('video')).toBe(video);
  expect(youtube.close).not.toHaveBeenCalled(); expect(soop.close).not.toHaveBeenCalled();
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  expect(youtube.close).not.toHaveBeenCalled();
  await render('youtube'); await click(button(yt, '재생 닫기'));
  expect(youtube.close).toHaveBeenCalledExactlyOnceWith('youtube-lease'); expect(soop.close).not.toHaveBeenCalled();
  expect(native.invoke).not.toHaveBeenCalled();
});

it('uses independent expansion keys and unique portal IDs for matching broadcast keys', async () => {
  const api = (provider: MediaProvider) => {
    const result = mediaApi(provider), item = media(provider);
    const group: MediaRecordingGroup = { type: 'recordingGroup', key: 'same-group', representative: item, groupRevision: 'r1', fileCount: 1, totalBytes: item.bytes, latestCompletedAt: item.completedAt };
    result.groups = vi.fn().mockResolvedValue({ status: 'ready', catalogRevision: provider, entries: [group], cursor: null });
    result.groupMembers = vi.fn().mockResolvedValue({ status: 'ready', group, items: [item], cursor: null });
    return result;
  };
  const youtube = api('youtube'), soop = api('soop');
  await act(async () => root.render(<><PlatformMediaLibrary provider="youtube" active privacyMode={false} api={youtube} /><PlatformMediaLibrary provider="soop" active privacyMode={false} api={soop} /></>));
  const yt = host.querySelector('[data-media-provider="youtube"]')!, sp = host.querySelector('[data-media-provider="soop"]')!;
  await click(yt.querySelector('.platform-media-group-heading')!); await click(sp.querySelector('.platform-media-group-heading')!);
  const portals = [...document.querySelectorAll<HTMLElement>('.platform-media-group-panel')];
  expect(portals).toHaveLength(2); expect(new Set(portals.map(panel => panel.id)).size).toBe(2);
  expect(youtube.groupMembers).toHaveBeenCalledExactlyOnceWith('youtube', 'same-group', undefined);
  expect(soop.groupMembers).toHaveBeenCalledExactlyOnceWith('soop', 'same-group', undefined);
  expect(readOpenMediaGroup('youtube')).toBe('same-group'); expect(readOpenMediaGroup('soop')).toBe('same-group');
  await click(button(portals.find(panel => panel.dataset.mediaProvider === 'youtube')!, '녹화 묶음 닫기'));
  expect(readOpenMediaGroup('youtube')).toBeNull(); expect(readOpenMediaGroup('soop')).toBe('same-group');
  expect(localStorage.getItem('atsumi.recording-library.expanded.v1')).toBeNull();
  expect(mediaGroupStorageKey('youtube')).not.toBe(mediaGroupStorageKey('soop'));
  expect(native.invoke).not.toHaveBeenCalled();
});

it('filters foreign screenshot entries and never sends matching IDs to the wrong provider', async () => {
  const api = shotsApi();
  vi.mocked(api.list).mockImplementation(async (provider: MediaProvider) => ({ items: [shot(provider), shot(provider === 'youtube' ? 'soop' : 'youtube')], cursor: null }));
  const render = async (active: MediaProvider) => act(async () => root.render(<><PlatformScreenshotLibrary provider="youtube" active={active === 'youtube'} privacyMode={false} api={api} /><PlatformScreenshotLibrary provider="soop" active={active === 'soop'} privacyMode={false} api={api} /></>));
  await render('youtube'); const yt = host.querySelector('[data-media-provider="youtube"]')!, sp = host.querySelector('[data-media-provider="soop"]')!;
  expect(yt.querySelectorAll('article')).toHaveLength(1); expect(yt).not.toHaveTextContent('soop 채널');
  await click(yt.querySelector('article')!); await click(button(yt, '저장 폴더 열기'));
  expect(api.folder).toHaveBeenCalledExactlyOnceWith('youtube', 'same-shot');
  await render('soop'); expect(sp.querySelectorAll('article')).toHaveLength(1); expect(sp).not.toHaveTextContent('youtube 채널');
  expect(sp.querySelector('.is-focused')).toBeNull(); await click(sp.querySelector('article')!);
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true, cancelable: true })));
  await click(button(document.querySelector('[role="alertdialog"]')!, '삭제'));
  expect(api.delete).toHaveBeenCalledExactlyOnceWith('soop', ['same-shot']);
  expect(yt.querySelectorAll('article')).toHaveLength(1); expect(sp.querySelectorAll('article')).toHaveLength(0);
  expect(api.image).toHaveBeenCalledWith('youtube', 'same-shot', false); expect(api.image).toHaveBeenCalledWith('soop', 'same-shot', false);
  expect(native.invoke).not.toHaveBeenCalled();
});

it('copies presentation without importing CHZZK APIs, styles, events, storage or YouTube-only loading CSS', () => {
  for (const source of [mediaSource, screenshotSource]) {
    expect(source).not.toMatch(/from\s+['"][^'"]*(?:streaming\/|api\/(?:officialBrowser|multiview|liveChannels|recordingProfile))/);
    expect(source).not.toMatch(/chzzk-presentation|atsumi\.chzzk|atsumi\.mado|youtube-spinner/);
  }
  expect(mediaCss).toContain('.platform-media-library-layout{display:grid;grid-template-columns:minmax(0,1fr) minmax(290px,400px)');
  expect(mediaCss).toContain('.platform-media-spinner{');
  expect(screenshotCss).toContain('minmax(min(100%,260px),1fr)');
  expect(screenshotCss).toContain('.platform-shot-card-caption{display:grid;gap:5px');
  expect(mediaCss + screenshotCss).not.toMatch(/(?:^|[}\n])\s*(?:button|h1|article|input|\.recording-library|\.streaming-workspace)\s*[{,]/);
});
