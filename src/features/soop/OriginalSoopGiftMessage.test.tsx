import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type PlatformReplayChatProps, type ReplayChatApi, type ReplayChatItem, type ReplayChatPage } from '../platforms/PlatformReplayChat';

// Display contracts are from the pinned official LiveView balloon / videoBalloon renderers.
// These are synthetic DTO tests, not evidence of captured or synchronized donations.
const row: ReplayChatItem = {
  id: 'saved-gift', timeMs: 1000, kind: 'donation', author: '<img src=x onerror=bad> 닉네임',
  text: '', amount: null, badges: [], color: null,
  soopGift: { kind: 'balloon', count: 100, relay: false, isDefault: true },
};
const page = (items: ReplayChatItem[]): ReplayChatPage => ({ items, state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: items.length, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false });
let host: HTMLDivElement, root: Root;
async function render(api: ReplayChatApi, overrides: Partial<PlatformReplayChatProps> = {}) {
  await act(async () => root.render(<PlatformReplayChat provider="soop" libraryId="saved-soop" currentTime={2} active privacyMode={false} api={api} {...overrides} />));
}
function content(): HTMLElement { return host.querySelector('aside')?.shadowRoot?.querySelector<HTMLElement>('.soop-original-chat-root') ?? host; }
async function click(node: Element) { await act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }))); }

describe('SOOP original saved balloon presentation', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

  it('uses the official tier assets at every boundary without remote or live gifting actions', async () => {
    const thresholds: ReadonlyArray<readonly [count: number, step: number]> = [[1, 1], [9, 1], [10, 2], [49, 2], [50, 3], [99, 3], [100, 4], [499, 4], [500, 5], [999, 5], [1000, 6], [0xffffffff, 6]];
    const items = thresholds.map(([count]) => ({ ...row, id: String(count), soopGift: { ...row.soopGift!, count } }));
    const api = { page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn() }; await render(api);
    const bubbles = content().querySelectorAll('.chatting-list-item > .donation-container > .donation-bubble');
    expect(bubbles).toHaveLength(thresholds.length);
    bubbles.forEach((bubble, index) => {
      const [count, step] = thresholds[index]!;
      expect(bubble.querySelector('.ceremony-img > img')).toHaveAttribute('src', `/assets/soop/gifts/ba_step${step}.png`);
      expect(bubble.querySelector('.count-box em')).toHaveAttribute('count', String(count));
      expect(bubble.querySelector('.count-box')).toHaveAttribute('aria-hidden', 'true');
      expect(bubble.querySelector('.money')).toHaveTextContent(`별풍선 ${count}개`);
      expect(bubble.querySelector('.name')).toHaveTextContent(row.author);
      expect(bubble.querySelectorAll('button')).toHaveLength(1);
      expect(bubble.querySelector('.info-box small')).toHaveTextContent('기준 이미지');
    });
    expect(content().querySelectorAll('img[src^="https:"],img[src^="http:"],iframe,script,a[href],.add-screen-button,.soop-replay-gift,.donation-state')).toHaveLength(0);
    expect(api.asset).not.toHaveBeenCalled(); expect(api.page).toHaveBeenCalledExactlyOnceWith('saved-soop', 2000, null, 120);
  });

  it('keeps video and relay semantics, uses explicit-only fan notices and opens the existing stored profile', async () => {
    const item: ReplayChatItem = { ...row, soopGift: { ...row.soopGift!, kind: 'videoBalloon', count: 10, relay: true, fanOrder: 7, topFan: true } };
    const api = { page: vi.fn().mockResolvedValue(page([item])) }; await render(api);
    expect(content().querySelector('.ceremony-img.custom-mobile-img > img')).toHaveAttribute('src', '/assets/soop/gifts/m_video_balloon.png');
    expect(content().querySelector('.money')).toHaveTextContent('중계방 영상풍선 10개');
    expect(content().querySelector('.money em')).toHaveTextContent('10개');
    expect(content().querySelector('[user-type="fan"] .donation-state')).toHaveTextContent('7번째 팬클럽이 되셨습니다.');
    expect(content().querySelector('[user-type="vip"] .donation-state')).toHaveTextContent('열혈팬이 되셨습니다.');
    expect(content().querySelectorAll('img')).toHaveLength(1);
    await click(content().querySelector('.info-box .name')!);
    const profile = content().querySelector('[aria-label="채팅 사용자 정보"]');
    expect(profile).toHaveTextContent(row.author); expect(profile).toHaveTextContent('저장 당시 정보'); expect(profile?.querySelector('img')).toBeNull();
    expect(api.page).toHaveBeenCalledOnce();
  });

  it('qualifies missing custom images and does not turn money into a guessed fan or membership badge', async () => {
    const custom: ReplayChatItem = { ...row, soopGift: { ...row.soopGift!, count: 10000, isDefault: false, fanOrder: 0, topFan: false } };
    const api = { page: vi.fn().mockResolvedValue(page([custom])) }; await render(api);
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src', '/assets/soop/gifts/ba_step6.png');
    expect(content().querySelector('.ceremony-img img')).toHaveAttribute('alt', '별풍선 기본 이미지');
    expect(content().querySelector('.info-box small')).toHaveTextContent('기본 이미지');
    expect(content().querySelector('.info-box small')).toHaveAttribute('title', '당시 맞춤 이미지는 저장되지 않아 기본 이미지로 표시합니다.');
    expect(content().querySelector('[class^="grade-badge-"],.donation-state,.thumb')).toBeNull();
  });

  it('leaves legacy or corrupt rich metadata readable without trusting it as an official gift', async () => {
    const invalid = [
      undefined, { ...row.soopGift, count: 0 }, { ...row.soopGift, count: -1 }, { ...row.soopGift, count: 1.5 },
      { ...row.soopGift, count: 0x100000000 }, { ...row.soopGift, count: Number.NaN }, { ...row.soopGift, count: '100' },
      { ...row.soopGift, kind: 'mission' }, { ...row.soopGift, relay: '1' }, { ...row.soopGift, isDefault: 'true' },
    ];
    const items: ReplayChatItem[] = invalid.map((soopGift, index) => ({ ...row, id: String(index), text: '기존 보존 본문', amount: '저장된 금액', soopGift: soopGift as ReplayChatItem['soopGift'] }));
    items.push({ ...row, id: 'wrong-kind', kind: 'membership', text: '별도 회원 이벤트' });
    const api = { page: vi.fn().mockResolvedValue(page(items)) }; await render(api);
    expect(content().querySelectorAll('.platform-replay-chat-row')).toHaveLength(items.length);
    expect(content().querySelectorAll('.donation-container,img,.donation-state')).toHaveLength(0);
    expect(content().textContent).toContain('기존 보존 본문'); expect(content().textContent).toContain('별도 회원 이벤트');
  });

  it('uses only source flags for notices even when optional metadata is corrupt', async () => {
    const corrupt = { ...row.soopGift!, fanOrder: -1, topFan: 'true' } as unknown as NonNullable<ReplayChatItem['soopGift']>;
    const api = { page: vi.fn().mockResolvedValue(page([{ ...row, soopGift: corrupt }])) }; await render(api);
    expect(content().querySelector('.donation-container')).not.toBeNull(); expect(content().querySelector('.donation-state')).toBeNull();
  });

  it('honors privacy and the parent seek clock without a separate donor timer or reread on profile actions', async () => {
    const api = { page: vi.fn().mockImplementation(async (_id, ms) => page(ms < 1000 ? [] : [row])) };
    await render(api, { privacyMode: true }); expect(api.page).not.toHaveBeenCalled(); expect(content().querySelector('.donation-container')).toBeNull();
    await render(api); expect(content().querySelector('.donation-container')).not.toBeNull();
    await render(api, { privacyMode: true }); expect(content().querySelectorAll('img,.donation-container')).toHaveLength(0); expect(api.page).toHaveBeenCalledOnce();
    await render(api, { currentTime: 0 }); expect(api.page).toHaveBeenLastCalledWith('saved-soop', 0, null, 120); expect(content().querySelector('.donation-container')).toBeNull();
    await render(api, { currentTime: 2 }); expect(content().querySelector('.donation-container')).not.toBeNull(); expect(api.page).toHaveBeenLastCalledWith('saved-soop', 2000, null, 120);
  });

  it('does not activate SOOP gift presentation on YouTube', async () => {
    const api = { page: vi.fn().mockResolvedValue(page([{ ...row, text: '보존된 본문', amount: '보존된 금액' }])) }; await render(api, { provider: 'youtube' });
    expect(host.querySelector('aside')?.shadowRoot).toBeNull(); expect(content().querySelector('.donation-container,img')).toBeNull(); expect(content().textContent).toContain('보존된 본문');
  });
});
