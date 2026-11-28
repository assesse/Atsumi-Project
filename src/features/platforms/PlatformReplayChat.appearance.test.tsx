import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type PlatformReplayChatProps, type ReplayChatApi, type ReplayChatItem, type ReplayChatPage } from './PlatformReplayChat';
import type { SoopChatMetadata, SoopPersonalconMetadata } from './platformMedia';

const assetId = (n: number) => n.toString(16).padStart(64, '0');
const image = 'data:image/png;base64,aGVsbG8=';
const row = (id: string): ReplayChatItem => ({ id, timeMs: 1000, kind: 'chat', author: '저장된 사용자', text: '저장된 본문', amount: null, badges: [], color: null });
const page = (items: ReplayChatItem[]): ReplayChatPage => ({ items, state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: items.length, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false });
const metadata = (changes: Partial<SoopChatMetadata> = {}): SoopChatMetadata => ({ flag1: 32, flag2: 262144, role: 'subscribe', badge: 'fan', subscriptionTier: 1, subscriptionMonths: 12, accumulatedSubscriptionMonths: 14, ...changes });
const personalcon = (changes: Partial<SoopPersonalconMetadata> = {}): SoopPersonalconMetadata => ({ imageAssetId: assetId(1), tier: 1, months: 14, tierNickname: '저장된 구독 이름', source: 'connectionMetadata', ...changes });
let host: HTMLDivElement, root: Root;
async function render(api: ReplayChatApi, changes: Partial<PlatformReplayChatProps> = {}) {
  await act(async () => root.render(<PlatformReplayChat provider="soop" libraryId="archive" currentTime={2} active privacyMode={false} api={api} {...changes} />));
}
function content() { return host.querySelector('aside')?.shadowRoot ?? host; }
function chatRows() { return [...content().querySelectorAll<HTMLElement>('[data-chat-row-id]')]; }
class Visibility {
  static instances: Visibility[] = [];
  nodes = new Set<Element>(); disconnected = false;
  constructor(readonly callback: IntersectionObserverCallback, readonly options: IntersectionObserverInit) { Visibility.instances.push(this); }
  observe(node: Element) { this.nodes.add(node); }
  disconnect() { this.disconnected = true; }
  emit(visible: Element[]) {
    this.callback([...this.nodes].map(target => ({ target, isIntersecting: visible.includes(target), intersectionRatio: visible.includes(target) ? 1 : 0 }) as IntersectionObserverEntry), this as unknown as IntersectionObserver);
  }
}
async function visible(observer: Visibility, nodes: Element[]) { await act(async () => observer.emit(nodes)); }

describe('SOOP captured appearance and visible local artwork', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); Visibility.instances = []; });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

  it('lets original role/palette rules win over the legacy body-color field while retaining saved message color', async () => {
    const base = { flag1: 32, flag2: 0, role: 'fan' as const, badge: 'fan' as const, subscriptionTier: null, subscriptionMonths: null, accumulatedSubscriptionMonths: null, nicknameColorKey: 'E12E2E', messageColor: '#113355' };
    const items = [{ ...row('fan'), color: '#55AAFF', soop: base },
      { ...row('streamer'), color: '#55AAFF', soop: { ...base, role: 'streamer' as const, badge: 'streamer' as const } },
      { ...row('nightbot'), color: '#55AAFF', soop: { ...base, role: 'nightbot' as const, badge: 'nightbot' as const } }];
    const api = { page: vi.fn().mockResolvedValue(page(items)) }; await render(api);
    expect(chatRows()[0]?.querySelector('.author')).toHaveClass('random-color1');
    expect(chatRows()[1]?.querySelector('.author')).not.toHaveClass('random-color1');
    expect(chatRows()[2]?.querySelector('.author')).not.toHaveClass('random-color1');
    for (const node of chatRows()) {
      expect((node.querySelector('.author') as HTMLElement).style.color).toBe('');
      expect(node.querySelector('.msg')).toHaveStyle({ color: '#113355' });
    }
    await render(api, { provider: 'youtube' });
    expect(host.querySelector('aside')?.shadowRoot).toBeNull();
    expect(host.querySelector('.platform-replay-chat-author')).toHaveStyle({ color: '#55AAFF' });
  });

  it('loads the mounted shadow viewport only, and rereads evicted local history on re-entry without retrying missing artwork', async () => {
    vi.stubGlobal('IntersectionObserver', Visibility);
    const items = Array.from({ length: 13 }, (_, n) => ({ ...row(`row-${n}`), parts: [{ type: 'emoji' as const, assetId: assetId(n + 1), alt: `image-${n}` }] }));
    const large = `data:image/png;base64,${'a'.repeat(1_200_000)}`;
    const api = { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn(async (_scope: string, id: string) => ({ state: 'ready' as const, dataUrl: id === assetId(13) ? null : large })) };
    await render(api);
    expect(api.asset).not.toHaveBeenCalled();
    const observer = Visibility.instances.at(-1)!;
    expect(observer.options.root).toBe(content().querySelector('.platform-replay-chat-messages'));
    expect(observer.nodes.size).toBe(13);
    const nodes = chatRows();
    for (let offset = 0; offset < 12; offset += 3) {
      await visible(observer, nodes.slice(offset, offset + 3));
      for (const node of nodes.slice(offset, offset + 3)) expect(node.querySelector('img')).toHaveAttribute('src', large);
      expect(api.asset).toHaveBeenCalledTimes(offset + 3);
    }
    expect(nodes[0]?.querySelector('img')).toBeNull(); // Old RAM entry really was evicted.
    await visible(observer, nodes.slice(0, 3));
    expect(api.asset).toHaveBeenCalledTimes(15);
    for (const node of nodes.slice(0, 3)) expect(node.querySelector('img')).toHaveAttribute('src', large);
    await visible(observer, [nodes[12]!]); expect(api.asset).toHaveBeenCalledTimes(16);
    await visible(observer, []); await visible(observer, [nodes[12]!]); expect(api.asset).toHaveBeenCalledTimes(16);
    expect(api.page).toHaveBeenCalledExactlyOnceWith('archive', 2000, null, 120);
  });

  it('fences old viewport callbacks after source changes, privacy, inactivity and scroll-root remounts', async () => {
    vi.stubGlobal('IntersectionObserver', Visibility);
    const item = { ...row('same-id'), parts: [{ type: 'emoji' as const, assetId: assetId(1), alt: 'image' }] };
    const api = { page: vi.fn().mockResolvedValue(page([item])), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: image }) };
    await render(api); const old = Visibility.instances.at(-1)!, oldNodes = chatRows();
    await render(api, { libraryId: 'next' }); expect(old.disconnected).toBe(true);
    await visible(old, oldNodes); expect(api.asset).not.toHaveBeenCalled();
    let current = Visibility.instances.at(-1)!;
    await visible(current, chatRows()); expect(api.asset).toHaveBeenCalledExactlyOnceWith('next', assetId(1));
    await render(api, { libraryId: 'next', privacyMode: true }); expect(current.disconnected).toBe(true);
    await visible(current, oldNodes); expect(api.asset).toHaveBeenCalledTimes(1); expect(chatRows()).toHaveLength(0);
    await render(api, { libraryId: 'next' }); current = Visibility.instances.at(-1)!;
    expect(current.options.root).toBe(content().querySelector('.platform-replay-chat-messages'));
    await visible(current, chatRows()); expect(chatRows()[0]?.querySelector('img')).toHaveAttribute('src', image);
    await render(api, { libraryId: 'next', active: false }); expect(current.disconnected).toBe(true);
    await visible(current, chatRows()); expect(api.asset).toHaveBeenCalledTimes(1);
  });

  it('renders captured subscriber and broadcaster representative icons from local IDs without online profile actions', async () => {
    const items = [{ ...row('subscriber'), soop: metadata({ personalcon: personalcon() }) },
      { ...row('broadcaster'), soop: metadata({ role: 'streamer', badge: 'streamer', subscriptionTier: null, personalcon: personalcon({ imageAssetId: assetId(2), months: 24 }) }) }];
    const api = { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: image }) };
    await render(api);
    expect(api.asset.mock.calls).toEqual([['archive', assetId(1)], ['archive', assetId(2)]]);
    for (const node of chatRows()) expect(node.querySelector('.thumb img')).toHaveAttribute('src', image);
    expect(chatRows()[0]?.querySelector('.thumb')).toHaveAttribute('title', '저장된 구독 퍼스나콘');
    expect(chatRows()[0]?.querySelector('.tier_tip .name')).toHaveTextContent('저장된 구독 이름');
    expect(chatRows()[0]?.querySelector('.tier_tip')).toHaveTextContent('누적 14개월');
    expect(chatRows()[1]?.querySelector('.tier_tip')).toBeNull();
    expect(content().querySelector('a[href],iframe')).toBeNull();
    expect(api.page).toHaveBeenCalledExactlyOnceWith('archive', 2000, null, 120);
  });

  it('qualifies missing or failed stored images while preserving the original text and unknown historical months', async () => {
    const items = [{ ...row('saved'), soop: metadata({ personalcon: personalcon({ months: -1 }) }) }, { ...row('legacy'), soop: metadata() }];
    const api = { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: image }) };
    await render(api);
    expect(chatRows()[0]?.querySelector('.tier_tip')).not.toHaveTextContent('개월');
    expect(chatRows()[1]?.querySelector('.thumb')).toHaveAttribute('title', '구독 기본 아이콘 · 당시 채널별 아이콘은 저장되지 않았습니다.');
    await act(async () => chatRows()[0]!.querySelector('img')!.dispatchEvent(new Event('error')));
    expect(chatRows()[0]?.querySelector('.thumb img')).toHaveAttribute('src', '/assets/soop/signature-default.svg');
    expect(chatRows()[0]?.querySelector('.thumb')).toHaveAttribute('title', '구독 기본 아이콘 · 저장된 채널별 아이콘을 불러오지 못했습니다.');
    expect(chatRows()[0]?.querySelector('.msg')).toHaveTextContent('저장된 본문');
    expect(api.asset).toHaveBeenCalledTimes(1);
  });

  it('does not admit remote IDs, unknown provenance or invalid selection fields as captured artwork', async () => {
    const bad = [personalcon({ imageAssetId: 'https://current-profile.test/icon.png' }), personalcon({ months: 0 }), personalcon({ months: 1201 }), personalcon({ months: -2 }), personalcon({ tier: 0 as 1 }), personalcon({ source: 'currentProfile' as 'connectionMetadata' })];
    const api = { page: vi.fn().mockResolvedValue(page(bad.map((value, n) => ({ ...row(`invalid-${n}`), soop: metadata({ personalcon: value }) })))), asset: vi.fn() };
    await render(api); expect(api.asset).not.toHaveBeenCalled();
    for (const node of chatRows()) expect(node.querySelector('.thumb img')).toHaveAttribute('src', '/assets/soop/signature-default.svg');
    await render(api, { provider: 'youtube' }); expect(api.asset).not.toHaveBeenCalled();
    expect(host.querySelector('.thumb')).toBeNull();
  });

  it('does not request the previous recording icon from a new recording or apply a late previous response', async () => {
    const finish = new Map<string, (value: { state: 'ready'; dataUrl: string }) => void>();
    const api = { page: vi.fn(async (scope: string) => page([{ ...row('same-id'), soop: metadata({ personalcon: personalcon({ imageAssetId: assetId(scope === 'archive' ? 1 : 2) }) }) }])),
      asset: vi.fn((scope: string, id: string) => new Promise<{ state: 'ready'; dataUrl: string }>(done => finish.set(`${scope}:${id}`, done))) };
    await render(api); expect(api.asset.mock.calls).toEqual([['archive', assetId(1)]]);
    await render(api, { libraryId: 'next' });
    expect(api.asset.mock.calls).toEqual([['archive', assetId(1)], ['next', assetId(2)]]);
    await act(async () => finish.get(`archive:${assetId(1)}`)!({ state: 'ready', dataUrl: image }));
    expect(chatRows()[0]?.querySelector('.thumb img')).toHaveAttribute('src', '/assets/soop/signature-default.svg');
    const nextImage = 'data:image/png;base64,d29ybGQ=';
    await act(async () => finish.get(`next:${assetId(2)}`)!({ state: 'ready', dataUrl: nextImage }));
    expect(chatRows()[0]?.querySelector('.thumb img')).toHaveAttribute('src', nextImage);
  });

  it('renders recorded staff and police notices with their original bubble roles and escaped public text', async () => {
    const items = (['staff', 'police'] as const).map(messageType => ({ ...row(messageType), author: '운영진', text: '<img src=x> 안내 본문', soop: metadata({ messageType, personalcon: personalcon(), nicknameColorKey: 'E12E2E' }) }));
    const api = { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn() }; await render(api);
    expect(chatRows()[0]?.querySelector('.bubble-container.notice .title .ic-notice')).not.toBeNull();
    expect(chatRows()[1]?.querySelector('.bubble-container.warning .title .ic-warning')).not.toBeNull();
    for (const node of chatRows()) {
      expect(node.querySelector('.title')).toHaveTextContent('운영진 안내');
      expect(node.querySelector('.msg')).toHaveTextContent('<img src=x> 안내 본문');
      expect(node.querySelector('img,.username,.grade-badge-fan')).toBeNull();
    }
    expect(api.asset).not.toHaveBeenCalled();
  });

  it('uses the original fixed Nightbot thumb and ordinary color rule even when fan or manager badges take priority', async () => {
    const items = [{ ...row('fan-bot'), soop: metadata({ flag1: (1 << 26) | 32, role: 'fan', badge: 'fan', nicknameColorKey: 'E12E2E', personalcon: personalcon() }) },
      { ...row('manager-bot'), soop: metadata({ flag1: (1 << 26) | 256, role: 'manager', badge: 'manager', nicknameColorKey: 'E12E2E' }) },
      { ...row('bot-ogq'), soop: metadata({ flag1: 1 << 26, role: 'nightbot', badge: 'nightbot', nicknameColorKey: 'E12E2E' }), soopEmoticon: { kind: 'ogq' as const, animated: false } }];
    const api = { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn() }; await render(api);
    for (const node of chatRows()) expect(node.querySelector('.thumb img')).toHaveAttribute('src', '/assets/soop/thumb_nightbot.svg');
    expect(chatRows()[0]?.querySelector('.grade-badge-fan')).not.toBeNull();
    expect(chatRows()[1]?.querySelector('.grade-badge-manager')).not.toBeNull();
    expect(chatRows()[0]?.querySelector('.author')).not.toHaveClass('random-color1');
    expect(chatRows()[1]?.querySelector('.author')).not.toHaveClass('random-color1');
    expect(chatRows()[2]?.querySelector('.author')).toHaveClass('random-color1');
    expect(content().querySelector('.tier_tip')).toBeNull(); expect(api.asset).not.toHaveBeenCalled();
  });

  it('uses only the bound recorded broadcaster avatar, with personalcon precedence and a qualified non-subscriber fallback', async () => {
    const avatar = { imageAssetId: assetId(3), kind: 'broadcaster' as const, source: 'connectionMetadata' as const };
    const broadcaster = metadata({ flag1: 4, role: 'streamer', badge: 'streamer', subscriptionTier: null, avatar });
    const items = [{ ...row('avatar'), soop: broadcaster },
      { ...row('representative'), soop: { ...broadcaster, personalcon: personalcon() } },
      { ...row('not-broadcaster'), soop: metadata({ subscriptionTier: null, avatar }) },
      { ...row('past-missing'), soop: { ...broadcaster, avatar: undefined } }];
    const api = { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: image }) }; await render(api);
    expect(api.asset.mock.calls).toEqual([['archive', assetId(3)], ['archive', assetId(1)]]);
    expect(chatRows()[0]?.querySelector('.thumb')).toHaveAttribute('title', '저장된 스트리머 프로필');
    expect(chatRows()[0]?.querySelector('.thumb img')).toHaveAttribute('alt', '프로필 이미지');
    expect(chatRows()[1]?.querySelector('.thumb')).toHaveAttribute('title', '저장된 구독 퍼스나콘');
    expect(chatRows()[2]?.querySelector('.thumb')).toBeNull(); expect(chatRows()[3]?.querySelector('.thumb')).toBeNull();
    await act(async () => chatRows()[0]!.querySelector('img')!.dispatchEvent(new Event('error')));
    expect(chatRows()[0]?.querySelector('img')).toHaveAttribute('src', '/assets/soop/thumb_profile.svg');
    expect(chatRows()[0]?.querySelector('.thumb')).toHaveAttribute('title', '프로필 기본 이미지 · 저장된 스트리머 이미지를 불러오지 못했습니다.');
    expect(chatRows()[0]?.querySelector('.tier_tip')).toBeNull(); expect(api.asset).toHaveBeenCalledTimes(2);
  });
});
