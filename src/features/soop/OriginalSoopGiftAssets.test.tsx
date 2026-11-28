import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type PlatformReplayChatProps, type ReplayChatApi, type ReplayChatItem, type ReplayChatPage } from '../platforms/PlatformReplayChat';

const id = 'a'.repeat(64);
const image = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aRZkAAAAASUVORK5CYII=';
const gift: ReplayChatItem = { id: 'captured', timeMs: 1000, kind: 'donation', author: 'saved viewer', text: '', amount: null, badges: [], color: null,
  soopGift: { kind: 'balloon', count: 50, relay: false, isDefault: false, imageAssetId: id } };
const page = (items: ReplayChatItem[]): ReplayChatPage => ({ items, state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: items.length, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false });
let host: HTMLDivElement, root: Root;
async function render(api: ReplayChatApi, overrides: Partial<PlatformReplayChatProps> = {}) {
  await act(async () => root.render(<PlatformReplayChat provider="soop" libraryId="captured-recording" currentTime={2} active privacyMode={false} api={api} {...overrides} />));
}
function content(): HTMLElement { return host.querySelector('aside')?.shadowRoot?.querySelector<HTMLElement>('.soop-original-chat-root') ?? host; }

describe('captured custom SOOP balloon images', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

  it('reads an opaque local asset once and keeps the official custom image shape and separate gift count', async () => {
    const api = { page: vi.fn().mockResolvedValue(page([gift])), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: image }) };
    await render(api);
    expect(api.asset).toHaveBeenCalledExactlyOnceWith('captured-recording', id);
    expect(content().querySelector('.custom-mobile-img img')).toHaveAttribute('src', image);
    expect(content().querySelector('.count-box,.info-box small')).toBeNull();
    expect(content().querySelector('.money')).toHaveTextContent('별풍선 50개');
    expect(content().querySelector('img[src^="http"]')).toBeNull();
    await render(api, { currentTime: 3 }); expect(api.asset).toHaveBeenCalledOnce();
  });

  it('restores the tier image, count and qualifier on decode failure without retrying online', async () => {
    const api = { page: vi.fn().mockResolvedValue(page([gift])), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: image }) };
    await render(api);
    await act(async () => content().querySelector('.ceremony-img img')!.dispatchEvent(new Event('error')));
    expect(content().querySelector('.custom-mobile-img')).toBeNull();
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', '/assets/soop/gifts/ba_step3.png');
    expect(content().querySelector('.count-box')).toHaveTextContent('50');
    expect(content().querySelector('.info-box small')).toHaveTextContent('기본 이미지');
    expect(api.asset).toHaveBeenCalledOnce();
    await render(api, { libraryId: 'another-recording' });
    expect(content().querySelector('.custom-mobile-img img')).toHaveAttribute('src', image);
  });

  it('does not collect corrupt or other-provider gift IDs', async () => {
    const items = [
      { ...gift, id: 'wrong-kind', kind: 'chat' },
      { ...gift, id: 'empty-count', soopGift: { ...gift.soopGift!, count: 0 } },
      { ...gift, id: 'invalid', soopGift: { ...gift.soopGift!, imageAssetId: 'https://res.sooplive.com/image.png' } },
      { ...gift, id: 'bad-default', soopGift: { ...gift.soopGift!, isDefault: 'true' as unknown as boolean } },
    ];
    const api = { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn() };
    await render(api); expect(api.asset).not.toHaveBeenCalled();
    expect(content().querySelector('img[src^="http"]')).toBeNull();
    api.page.mockResolvedValue(page([gift]));
    await render(api, { provider: 'youtube', libraryId: 'youtube-recording' });
    expect(api.asset).not.toHaveBeenCalled(); expect(content().querySelector('.donation-container')).toBeNull();
  });

  it.each([
    ['balloon', true, false, 1, 1], ['balloon', true, false, 211, 4], ['videoBalloon', true, true, 50, 3], ['videoBalloon', false, true, 50, 3],
  ] as const)('uses captured %s default=%s bytes without changing original count/class semantics', async (kind, isDefault, customClass, count, step) => {
    const saved: ReplayChatItem = { ...gift, soopGift: { ...gift.soopGift!, kind, isDefault, count } };
    const api = { page: vi.fn().mockResolvedValue(page([saved])), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: image }) };
    await render(api);
    expect(api.asset).toHaveBeenCalledExactlyOnceWith('captured-recording', id);
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', image);
    expect(content().querySelector('.ceremony-img')?.classList.contains('custom-mobile-img')).toBe(customClass);
    expect(content().querySelector('.count-box em')).toHaveAttribute('count', String(count));
    expect(content().querySelector('.count-box')).toHaveAttribute('aria-hidden', 'true');
    expect(content().querySelector('.info-box small')).toBeNull();
    await act(async () => content().querySelector('.ceremony-img img')!.dispatchEvent(new Event('error')));
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', kind === 'videoBalloon' ? '/assets/soop/gifts/m_video_balloon.png' : `/assets/soop/gifts/ba_step${step}.png`);
    expect(content().querySelector('.count-box em')).toHaveAttribute('count', String(count));
    expect(content().querySelector('.ceremony-img')?.classList.contains('custom-mobile-img')).toBe(customClass);
    expect(content().querySelector('.info-box small')).toHaveTextContent('기준 이미지');
    expect(content().querySelector('.info-box small')).toHaveAttribute('title', '당시 이미지가 없거나 읽을 수 없어 앱에 포함된 기준 이미지로 표시합니다.');
    expect(api.asset).toHaveBeenCalledOnce();
  });

  it('keeps separately recorded standard responses distinct across replay scopes and missing legacy history qualified', async () => {
    const secondId = 'b'.repeat(64), secondImage = 'data:image/png;base64,YWJj';
    const first = { ...gift, soopGift: { ...gift.soopGift!, isDefault: true } };
    const second = { ...first, soopGift: { ...first.soopGift!, imageAssetId: secondId } };
    const api = { page: vi.fn().mockImplementation(async (scope: string) => page([scope === 'second' ? second : first])),
      asset: vi.fn().mockImplementation(async (_scope: string, asset: string) => ({ state: 'ready', dataUrl: asset === id ? image : secondImage })) };
    await render(api); expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', image);
    await render(api, { libraryId: 'second' }); expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', secondImage);
    api.page.mockResolvedValue(page([{ ...first, soopGift: { ...first.soopGift!, imageAssetId: undefined } }]));
    await render(api, { libraryId: 'legacy' });
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', '/assets/soop/gifts/ba_step3.png');
    expect(content().querySelector('.info-box small')).toHaveTextContent('기준 이미지');
    expect(api.asset).toHaveBeenCalledTimes(2);
  });

  it('falls back for missing or unsafe returned bytes without replay network recovery', async () => {
    const api = { page: vi.fn().mockResolvedValue(page([gift])), asset: vi.fn().mockResolvedValue({ state: 'missing', dataUrl: null }) };
    await render(api); expect(content().querySelector('.info-box small')).toHaveTextContent('기본 이미지');
    api.asset.mockResolvedValue({ state: 'ready', dataUrl: 'https://res.sooplive.com/image.png' });
    await render(api, { libraryId: 'unsafe-recording' });
    expect(content().querySelector('.custom-mobile-img')).toBeNull();
    expect(content().querySelector('.money')).toHaveTextContent('별풍선 50개');
    expect(content().querySelector('img[src^="http"]')).toBeNull();
  });

  it('does not load while private or expose an old image after a scope change', async () => {
    let finish!: (value: { state: 'ready'; dataUrl: string }) => void;
    const api = { page: vi.fn().mockResolvedValue(page([gift])), asset: vi.fn().mockImplementation(() => new Promise(done => { finish = done; })) };
    await render(api, { privacyMode: true }); expect(api.asset).not.toHaveBeenCalled();
    await render(api); expect(api.asset).toHaveBeenCalledOnce();
    await render(api, { privacyMode: true, libraryId: 'next-recording' });
    await act(async () => finish({ state: 'ready', dataUrl: image }));
    expect(content().querySelector('.donation-container,img')).toBeNull();
  });
});
