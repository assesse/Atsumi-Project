import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type PlatformReplayChatProps, type ReplayChatApi, type ReplayChatItem, type ReplayChatPage } from '../platforms/PlatformReplayChat';
import type { SoopSubscriptionMetadata } from '../platforms/platformMedia';
import { soopSubscriptionNickname, validSoopSubscription } from './soopSubscription';

const primaryId = 'a'.repeat(64), fallbackId = 'b'.repeat(64);
const primaryImage = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aRZkAAAAASUVORK5CYII=';
const fallbackImage = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
const newSubscription: SoopSubscriptionMetadata = { event: 'new', displayTier: 1, fromVod: false, productMonths: 3 };
const row = (value: SoopSubscriptionMetadata = newSubscription): ReplayChatItem => ({ id: 'subscription', timeMs: 1000, kind: 'membership', author: 'saved viewer', text: '', amount: null, badges: [], color: null, soopSubscription: value });
const page = (items: ReplayChatItem[]): ReplayChatPage => ({ items, state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: items.length, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false });
let host: HTMLDivElement, root: Root;
async function render(api: ReplayChatApi, overrides: Partial<PlatformReplayChatProps> = {}) {
  await act(async () => root.render(<PlatformReplayChat provider="soop" libraryId="recording" currentTime={2} active privacyMode={false} api={api} {...overrides} />));
}
function content(): HTMLElement { return host.querySelector('aside')?.shadowRoot?.querySelector<HTMLElement>('.soop-original-chat-root') ?? host; }
function apiFor(items: ReplayChatItem[]) { return { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn().mockResolvedValue({ state: 'missing', dataUrl: null }) }; }

describe('original SOOP saved subscription ceremonies', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

  it.each([
    [false, undefined, '베이직 구독하였습니다.', null],
    [true, undefined, 'VOD에서 베이직 구독하였습니다.', null],
    [false, '저장된 구독명', '베이직 저장된 구독명', '구독하였습니다.'],
    [true, '저장된 구독명', '베이직 저장된 구독명', 'VOD에서 구독하였습니다.'],
  ] as const)('matches the new subscription VOD=%s custom=%s branch without claiming package months are elapsed', async (fromVod, tierNickname, tierText, moneyText) => {
    await render(apiFor([row({ ...newSubscription, fromVod, tierNickname })]));
    expect(content().querySelector('.donation-container.subscribe > .donation-bubble > .ceremony-img.custom-mobile-img')).not.toBeNull();
    expect(content().querySelector('.tier_info')).toHaveTextContent(tierText);
    if (moneyText) expect(content().querySelector('.money')).toHaveTextContent(moneyText);
    else expect(content().querySelector('.money')).toBeNull();
    expect(content().querySelector('.tier_info em')?.textContent).toBe(tierNickname);
    expect(content().querySelector('.count-box,[class^="grade-badge"],.thumb,.add-img')).toBeNull();
    expect(content().textContent).not.toContain('3개월');
    expect(content().querySelector('.info-box small')).toHaveTextContent('기본 이미지');
  });

  it.each([undefined, '플러스', '별도 구독명'])('uses the explicit renewal month with the official plus tier label (%s)', async tierNickname => {
    await render(apiFor([row({ event: 'renewal', displayTier: 2, months: 17, tierNickname })]));
    expect(content().querySelector('.tier_info')).toHaveTextContent(tierNickname === '별도 구독명' ? '플러스 별도 구독명' : '플러스');
    expect(content().querySelector('.money')).toHaveTextContent('17개월째 구독중');
    expect(content().querySelector('.money em')).toHaveTextContent('17');
    expect(content().querySelector('.tier_info em')?.textContent).toBe(tierNickname === '별도 구독명' ? tierNickname : undefined);
  });

  it('uses only the captured local asset chain and keeps text through all decode failures', async () => {
    const api = apiFor([row({ ...newSubscription, imageAssetId: primaryId, fallbackAssetId: fallbackId })]);
    api.asset.mockImplementation(async (_scope: string, id: string) => ({ state: 'ready', dataUrl: id === primaryId ? primaryImage : fallbackImage }));
    await render(api);
    expect(api.asset).toHaveBeenCalledTimes(2);
    expect(api.asset).toHaveBeenCalledWith('recording', primaryId);
    expect(api.asset).toHaveBeenCalledWith('recording', fallbackId);
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', primaryImage);
    expect(content().querySelector('.info-box small')).toBeNull();
    await act(async () => content().querySelector('.ceremony-img img')!.dispatchEvent(new Event('error')));
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', fallbackImage);
    expect(content().querySelector('.info-box small')).toHaveTextContent('기본 이미지');
    await act(async () => content().querySelector('.ceremony-img img')!.dispatchEvent(new Event('error')));
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', '/assets/soop/subscriptions/gudok_1.png');
    await act(async () => content().querySelector('.ceremony-img img')!.dispatchEvent(new Event('error')));
    expect(content().querySelector('.ceremony-img img')).toBeNull();
    expect(content().querySelector('.tier_info')).toHaveTextContent('베이직 구독하였습니다.');
    expect(content().querySelector('.info-box small')).toHaveTextContent('이미지 없음');
    expect(api.asset).toHaveBeenCalledTimes(2);
    expect(content().querySelector('img[src^="http"]')).toBeNull();
    await render(api, { libraryId: 'different-recording' });
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', primaryImage);
  });

  it('ignores invalid optional image IDs and labels without erasing the valid ceremony', async () => {
    const api = apiFor([row({ ...newSubscription, imageAssetId: 'https://static.file.sooplive.com/injected.png', fallbackAssetId: '../file.png', tierNickname: 'bad\nlabel' })]);
    await render(api);
    expect(api.asset).not.toHaveBeenCalled();
    expect(content().querySelector('.tier_info')).toHaveTextContent('베이직 구독하였습니다.');
    expect(content().querySelector('.tier_info em,img[src^="http"]')).toBeNull();
  });

  it('does not infer ceremonies from ordinary subscriber metadata, malformed events or other platforms', async () => {
    const invalid: unknown[] = [
      { ...newSubscription, event: 'gift' }, { ...newSubscription, displayTier: 3 },
      { ...newSubscription, productMonths: 2 }, { ...newSubscription, fromVod: null },
      { ...newSubscription, months: 3 }, { event: 'renewal', displayTier: 1, months: 0 },
      { event: 'renewal', displayTier: 1, months: 1201 }, { event: 'renewal', displayTier: 1, months: 1.5 },
      { event: 'renewal', displayTier: 1, months: 3, fromVod: false },
      { event: 'renewal', displayTier: 1, months: 3, productMonths: 1 },
    ];
    const items = invalid.map((value, index) => ({ ...row({ ...value as SoopSubscriptionMetadata, imageAssetId: primaryId }), id: `invalid-${index}`, text: `saved text ${index}` }));
    items.push({ ...row({ ...newSubscription, imageAssetId: primaryId }), id: 'ordinary', kind: 'chat', text: 'ordinary subscriber chat' });
    const api = apiFor(items);
    await render(api);
    expect(api.asset).not.toHaveBeenCalled();
    expect(content().querySelector('.donation-container.subscribe')).toBeNull();
    expect(content()).toHaveTextContent('saved text 9');
    expect(content()).toHaveTextContent('ordinary subscriber chat');
    api.page.mockResolvedValue(page([row({ ...newSubscription, imageAssetId: primaryId })]));
    await render(api, { provider: 'youtube', libraryId: 'youtube-recording' });
    expect(api.asset).not.toHaveBeenCalled();
    expect(content().querySelector('.donation-container.subscribe')).toBeNull();
  });

  it('keeps the saved-profile action and read-only footer without account identifiers or purchase actions', async () => {
    await render(apiFor([row({ ...newSubscription, tierNickname: '<img src=x>' })]));
    expect(content().querySelector('.tier_info em')).toHaveTextContent('<img src=x>');
    expect(content().querySelector('.tier_info img,[user_id],[user_nick],.add-img,a[href]')).toBeNull();
    await act(async () => (content().querySelector('.info-box .name') as HTMLButtonElement).click());
    expect(content().querySelector('[aria-label="채팅 사용자 정보"]')).toHaveTextContent('saved viewer');
    expect(content().querySelector('[aria-label="채팅 사용자 정보"]')).toHaveTextContent('저장 당시 정보');
    expect(content().querySelectorAll('.actionbox button:not(:disabled)')).toHaveLength(0);
    expect(Array.from(content().querySelectorAll('.actionbox a')).every(button => button.getAttribute('aria-disabled') === 'true')).toBe(true);
  });

  it('keeps privacy and source changes from revealing late local image reads', async () => {
    const finish: Array<(value: { state: 'ready'; dataUrl: string }) => void> = [];
    const api = apiFor([row({ ...newSubscription, imageAssetId: primaryId, fallbackAssetId: fallbackId })]);
    api.asset.mockImplementation(() => new Promise(done => finish.push(done)));
    await render(api, { privacyMode: true }); expect(api.asset).not.toHaveBeenCalled();
    await render(api); expect(api.asset).toHaveBeenCalledTimes(2);
    await render(api, { privacyMode: true, libraryId: 'next-recording' });
    await act(async () => finish.forEach(done => done({ state: 'ready', dataUrl: primaryImage })));
    expect(content().querySelector('.donation-container,img')).toBeNull();
  });

  it('preserves scalar bounds, null optionals and engineering month limits', () => {
    expect(soopSubscriptionNickname('😀'.repeat(80))).toBe('😀'.repeat(80));
    expect(soopSubscriptionNickname('😀'.repeat(81))).toBeUndefined();
    expect(soopSubscriptionNickname('\ud800')).toBeUndefined();
    expect(soopSubscriptionNickname('\u0085bad')).toBeUndefined();
    expect(soopSubscriptionNickname('   ')).toBeUndefined();
    expect(validSoopSubscription({ ...newSubscription, months: null })).toBe(true);
    expect(validSoopSubscription({ event: 'renewal', displayTier: 1, months: 1200, fromVod: null, productMonths: null })).toBe(true);
  });
});
