import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SoopDiscovery } from './SoopDiscovery';
import type { SoopApi, SoopSearchItem } from '../../api/soop';
import type { SoopVodApi, SoopVodSnapshot } from '../../api/soopVod';
import type { PlatformMediaApi } from '../platforms/platformMedia';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('../platforms/nativePlayerGeometry', () => ({
  measureNativePlayerGeometry: (player: unknown, chat: unknown, options: object) => ({ ...options, player: player ? { x: 0, y: 0, width: 640, height: 360 } : null, chat: chat ? { x: 647, y: 0, width: 240, height: 360 } : null, occlusions: [] }),
  observeNativePlayerGeometry: () => () => {},
}));
let host: HTMLDivElement, root: Root;
beforeEach(() => { localStorage.clear(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
const items: SoopSearchItem[] = ['209244227', '209244228'].map((id, index) => ({ id, kind: 'video', title: `영상 ${index + 1}`, channelId: 'artist', channelName: '작가', thumbnailKey: null, duration: '0:09', views: 100, restricted: false }));
function setup() {
  const api: SoopApi = { inspect: vi.fn(), home: vi.fn().mockResolvedValue({ sections: [{ title: '영상', items }], personalized: false, notice: null }) };
  const open = vi.fn<SoopVodApi['open']>().mockImplementation(async videoId => ({ token: `lease-${videoId}`, videoId, state: 'playing', chatState: 'available', fullscreen: false, error: null }));
  const vodApi: SoopVodApi = { open, snapshot: vi.fn(), viewport: vi.fn().mockResolvedValue(undefined), close: vi.fn().mockResolvedValue(undefined), fullscreen: vi.fn().mockResolvedValue(undefined) };
  const download = vi.fn().mockResolvedValue({ jobId: 'download' }), onLive = vi.fn();
  const render = async (active = true, privacyMode = false) => act(async () => root.render(<SoopDiscovery api={api} active={active} privacyMode={privacyMode} onLive={onLive} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} vodApi={vodApi} mediaApi={{ download } as unknown as PlatformMediaApi} />));
  return { api, vodApi, download, onLive, render };
}
const click = async (node: Element) => act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
const openCard = async (index = 0) => act(async () => host.querySelectorAll('article')[index]!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));

describe('SOOP remote VOD detail integration', () => {
  it('only opens original playback after explicit play and preserves separate download/favorite actions', async () => {
    const app = setup(); await app.render(); await openCard();
    expect(app.vodApi.open).not.toHaveBeenCalled(); expect(app.download).not.toHaveBeenCalled();
    await click(host.querySelector('[aria-label="원본 영상 재생"]')!);
    expect(app.vodApi.open).toHaveBeenCalledWith('209244227', expect.any(String)); expect(app.download).not.toHaveBeenCalled();
    await click([...host.querySelectorAll('.soop-detail-actions button')].find(node => node.textContent === '다운로드')!);
    expect(app.download).toHaveBeenCalledWith({ provider: 'soop', kind: 'video', externalId: '209244227' }, expect.any(String));
    await click(host.querySelector('.soop-detail-actions button')!);
    expect(JSON.parse(localStorage.getItem('atsumi:soop:video-favorites:v1')!)[0].id).toBe('209244227');
    expect(app.vodApi.open).toHaveBeenCalledTimes(1);
  });
  it('retains playback on provider/tab hide and privacy, and releases the lease on explicit close', async () => {
    const app = setup(); await app.render(); await openCard(); await click(host.querySelector('[aria-label="원본 영상 재생"]')!);
    const player = host.querySelector('.soop-vod-pair');
    await app.render(false); expect(host.querySelector('.soop-detail-shade')).toHaveAttribute('hidden');
    expect(host.querySelector('.soop-vod-pair')).toBe(player); expect(app.vodApi.close).not.toHaveBeenCalled();
    await app.render(true, true); expect(app.vodApi.viewport).toHaveBeenLastCalledWith('lease-209244227', expect.objectContaining({ visible: false, muted: true }));
    await app.render(); expect(host.querySelector('.soop-vod-pair')).toBe(player); expect(app.vodApi.open).toHaveBeenCalledTimes(1);
    await click(host.querySelector('.soop-detail-close')!);
    expect(app.vodApi.close).toHaveBeenCalledWith('lease-209244227'); expect(host.querySelector('.soop-detail')).toBeNull();
  });
  it('replacing detail closes only the old viewer and does not autoplay the replacement', async () => {
    const app = setup(); await app.render(); await openCard(); await click(host.querySelector('[aria-label="원본 영상 재생"]')!);
    await openCard(1);
    expect(app.vodApi.close).toHaveBeenCalledWith('lease-209244227'); expect(app.vodApi.open).toHaveBeenCalledTimes(1);
    expect(host.querySelector('.soop-detail')).toHaveTextContent('영상 2');
    await click(host.querySelector('[aria-label="원본 영상 재생"]')!);
    expect(app.vodApi.open).toHaveBeenLastCalledWith('209244228', expect.any(String));
  });
  it('closing before a delayed open resolves cannot leave a hidden session behind', async () => {
    const app = setup(); let resolve!: (value: SoopVodSnapshot) => void;
    vi.mocked(app.vodApi.open).mockImplementationOnce(() => new Promise(res => { resolve = res; }));
    await app.render(); await openCard(); await click(host.querySelector('[aria-label="원본 영상 재생"]')!);
    await click(host.querySelector('.soop-detail-close')!);
    await act(async () => resolve({ token: 'late', videoId: '209244227', state: 'playing', chatState: 'available', fullscreen: false, error: null }));
    expect(app.vodApi.close).toHaveBeenCalledWith('late'); expect(host.querySelector('.soop-detail')).toBeNull();
  });
  it('leaves live viewing on its existing channel path without granting VOD a recording API', async () => {
    const app = setup(); vi.mocked(app.api.home!).mockResolvedValue({ sections: [{ title: 'LIVE', items: [{ ...items[0]!, kind: 'live' }] }], personalized: false, notice: null });
    await app.render(); await openCard();
    expect(host.querySelector('[aria-label="원본 영상 재생"]')).toBeNull();
    await click([...host.querySelectorAll('.soop-detail-actions button')].find(node => node.textContent === '시청')!);
    expect(app.onLive).toHaveBeenCalledTimes(1); expect(app.vodApi.open).not.toHaveBeenCalled(); expect(app.download).not.toHaveBeenCalled();
  });
  it('exits fullscreen from splitter Escape before a second Escape closes detail', async () => {
    const app = setup();
    vi.mocked(app.vodApi.open).mockResolvedValue({ token: 'lease-209244227', videoId: '209244227', state: 'playing', chatState: 'available', fullscreen: true, error: null });
    await app.render(); await openCard(); await click(host.querySelector('[aria-label="원본 영상 재생"]')!);
    const separator = host.querySelector<HTMLElement>('[role="separator"]')!; separator.focus();
    await act(async () => separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
    expect(app.vodApi.fullscreen).toHaveBeenCalledWith('lease-209244227', false);
    expect(app.vodApi.close).not.toHaveBeenCalled(); expect(host.querySelector('.soop-detail')).not.toBeNull();
    expect(host.querySelector('.is-native-fullscreen')).toBeNull();
    await act(async () => separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
    expect(app.vodApi.close).toHaveBeenCalledWith('lease-209244227'); expect(host.querySelector('.soop-detail')).toBeNull();
  });
});
