import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type PlatformReplayChatProps, type ReplayChatApi, type ReplayChatItem, type ReplayChatPage } from '../platforms/PlatformReplayChat';

const ids = ['a','b','c','d'].map(c => c.repeat(64));
const urls = ['AQ==','Ag==','Aw==','BA=='].map(bytes => `data:image/png;base64,${bytes}`);
const row: ReplayChatItem = { id: 'ogq', timeMs: 1000, kind: 'chat', author: '검증 닉네임', text: '이모티콘 뒤의 본문', amount: null, badges: [], color: null,
  soop: { flag1: 32, flag2: 0, role: 'fan', badge: 'fan', subscriptionTier: null, subscriptionMonths: null, accumulatedSubscriptionMonths: null, nicknameColorKey: 'E12E2E' },
  soopEmoticon: { kind: 'ogq', animated: true, staticImageAssetId: ids[0], smallStaticImageAssetId: ids[1], animatedImageAssetId: ids[2], smallAnimatedImageAssetId: ids[3] } };
const page = (items: ReplayChatItem[]): ReplayChatPage => ({ items, state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: items.length, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false });
const apiFor = (items = [row]) => ({ page: vi.fn().mockResolvedValue(page(items)), asset: vi.fn(async (_scope: string, id: string) => ({ state: 'ready' as const, dataUrl: urls[ids.indexOf(id)] ?? null })) });
let host: HTMLDivElement, root: Root;
async function render(api: ReplayChatApi, overrides: Partial<PlatformReplayChatProps> = {}) {
  await act(async () => root.render(<PlatformReplayChat provider="soop" libraryId="ogq-recording" currentTime={2} active privacyMode={false} api={api} {...overrides} />));
}
const content = () => host.querySelector('aside')?.shadowRoot?.querySelector<HTMLElement>('.soop-original-chat-root') ?? host;
const emote = () => content().querySelector<HTMLImageElement>('.emoticon');
async function click(label: string) { await act(async () => content().querySelector<HTMLElement>(`[aria-label="${label}"]`)!.click()); }
async function toggle(label: string) { await act(async () => Array.from(content().querySelectorAll('label')).find(node => node.textContent === label)!.click()); }
async function settings() { await click('채팅 표시 설정'); await click('이모티콘 표시 방법'); }

describe('captured SOOP OGQ/Gem replay', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

  it('keeps a separate original emote block, ordinary body/badge and explicit palette without donation controls', async () => {
    const api = apiFor(); await render(api);
    expect(api.asset).toHaveBeenCalledTimes(2);
    for (const id of [ids[0], ids[2]]) expect(api.asset).toHaveBeenCalledWith('ogq-recording', id);
    expect(emote()).toHaveAttribute('src', urls[2]);
    expect(emote()).toHaveAttribute('data-image-state', 'animated'); expect(emote()).not.toHaveAttribute('title');
    expect(content().querySelector('.emoticon-box.small')).toBeNull();
    expect(content().querySelector('.emoticon-box')?.nextElementSibling).toHaveClass('msg');
    expect(content().querySelector('.platform-replay-chat-text')).toHaveTextContent(row.text);
    expect(content().querySelector('.author')).toHaveClass('random-color1');
    expect(content().querySelector('.grade-badge-fan')).not.toBeNull();
    expect(content().querySelector('.donation-container,.soop-replay-gift,a[href],img[src^="http"]')).toBeNull();
  });

  it('switches normal/small and moving/static locally and hides only the image when disabled', async () => {
    const api = apiFor(); await render(api); await settings();
    await toggle('OGQ 이모티콘 작게 보기'); expect(content().querySelector('.emoticon-box.small')).not.toBeNull(); expect(emote()).toHaveAttribute('src', urls[3]);
    await toggle('움직이는 이모티콘 보기'); expect(emote()).toHaveAttribute('src', urls[1]);
    expect(emote()).toHaveAttribute('data-image-state', 'static'); expect(emote()).not.toHaveAttribute('title'); expect(emote()).toHaveAttribute('alt', 'OGQ 이모티콘');
    await toggle('OGQ 이모티콘 작게 보기'); expect(emote()).toHaveAttribute('src', urls[0]);
    await toggle('이모티콘 보기'); expect(emote()).toBeNull(); expect(content()).toHaveTextContent('/OGQ 이모티콘/');
    expect(content().querySelector('.platform-replay-chat-text')).toHaveTextContent(row.text);
    expect(api.page).toHaveBeenCalledOnce(); expect(api.asset).toHaveBeenCalledTimes(4);
  });

  it('falls back from failed animation to local static, official placeholder, then readable label', async () => {
    const api = apiFor(); await render(api);
    await act(async () => emote()!.dispatchEvent(new Event('error'))); expect(emote()).toHaveAttribute('src', urls[0]);
    expect(emote()).toHaveAttribute('data-image-state', 'static-fallback'); expect(emote()).toHaveAttribute('alt', 'OGQ 이모티콘 (정지 이미지)');
    expect(emote()).toHaveAttribute('title', '움직이는 이미지 대신 저장된 정지 이미지로 표시합니다.');
    await act(async () => emote()!.dispatchEvent(new Event('error'))); expect(emote()).toHaveAttribute('src', '/assets/soop/ogq_default_dark.svg');
    expect(emote()).toHaveAttribute('data-image-state', 'placeholder'); expect(emote()).toHaveAttribute('title', '저장된 이모티콘 이미지를 불러오지 못했습니다.');
    await act(async () => emote()!.dispatchEvent(new Event('error'))); expect(emote()).toBeNull(); expect(content()).toHaveTextContent('/OGQ 이모티콘/');
    expect(content()).toHaveTextContent(row.text); expect(api.asset).toHaveBeenCalledTimes(2);
  });

  it.each(['missing-id', 'missing-asset'] as const)('qualifies saved static fallback for %s without inventing past animation or adding a text row', async reason => {
    const storedRow = { ...row, soopEmoticon: { ...row.soopEmoticon!, animatedImageAssetId: reason === 'missing-id' ? undefined : ids[2] } };
    const api: ReplayChatApi = { page: vi.fn().mockResolvedValue(page([storedRow])), asset: vi.fn(async (_scope, id) => id === ids[2]
      ? { state: 'missing' as const, dataUrl: null } : { state: 'ready' as const, dataUrl: urls[ids.indexOf(id)]! }) };
    await render(api);
    expect(emote()).toHaveAttribute('src', urls[0]); expect(emote()).toHaveAttribute('data-image-state', 'static-fallback');
    expect(emote()).toHaveAttribute('alt', 'OGQ 이모티콘 (정지 이미지)'); expect(emote()).toHaveAttribute('title', '움직이는 이미지 대신 저장된 정지 이미지로 표시합니다.');
    expect(content().querySelector('.emoticon-box')?.textContent).toBe(''); expect(content().querySelector('.emoticon-box')?.nextElementSibling).toHaveClass('msg');
    expect(content().querySelector('.platform-replay-chat-text')).toHaveTextContent(row.text); expect(content().querySelector('.grade-badge-fan')).not.toBeNull();
    expect(content().querySelector('img[src^="http"]')).toBeNull(); expect(api.asset).toHaveBeenCalledTimes(reason === 'missing-id' ? 1 : 2);
  });

  it('removes the temporary static qualification when the saved animation arrives without retrying or changing the row', async () => {
    let resolveAnimation!: (value: { state: 'ready'; dataUrl: string }) => void;
    const animation = new Promise<{ state: 'ready'; dataUrl: string }>(done => { resolveAnimation = done; });
    const api: ReplayChatApi = { page: vi.fn().mockResolvedValue(page([row])), asset: vi.fn((_scope, id) => id === ids[2]
      ? animation : Promise.resolve({ state: 'ready' as const, dataUrl: urls[0]! })) };
    await render(api);
    const originalRow = content().querySelector('[data-chat-row-id="ogq"]');
    expect(emote()).toHaveAttribute('src', urls[0]); expect(emote()).toHaveAttribute('data-image-state', 'static-fallback');
    expect(emote()?.title).not.toContain('저장되지');
    await act(async () => resolveAnimation({ state: 'ready', dataUrl: urls[2]! }));
    expect(emote()).toHaveAttribute('src', urls[2]); expect(emote()).toHaveAttribute('data-image-state', 'animated');
    expect(emote()).toHaveAttribute('alt', 'OGQ 이모티콘'); expect(emote()).not.toHaveAttribute('title');
    expect(content().querySelector('[data-chat-row-id="ogq"]')).toBe(originalRow); expect(api.asset).toHaveBeenCalledTimes(2);
  });

  it('keeps an originally static captured emote unqualified even when animation display is enabled', async () => {
    const api = apiFor([{ ...row, soopEmoticon: { ...row.soopEmoticon!, animated: false } }]); await render(api);
    expect(emote()).toHaveAttribute('src', urls[0]); expect(emote()).toHaveAttribute('data-image-state', 'static');
    expect(emote()).toHaveAttribute('alt', 'OGQ 이모티콘'); expect(emote()).not.toHaveAttribute('title'); expect(api.asset).toHaveBeenCalledTimes(1);
  });

  it('preserves empty-body emotes and Gem fallback without constructing remote URLs', async () => {
    const api = apiFor([{ ...row, text: '', soopEmoticon: { kind: 'gem', animated: false, staticImageAssetId: 'https://invalid.test/a.png', animatedImageAssetId: ids[2] } }]);
    await render(api); expect(api.asset).not.toHaveBeenCalled(); expect(emote()).toHaveAttribute('alt', 'Gem 이모티콘');
    expect(emote()).toHaveAttribute('src', '/assets/soop/ogq_default_dark.svg'); expect(content().querySelector('.author')).toHaveTextContent(row.author);
  });

  it('does not admit another provider, non-chat kind, malformed metadata or unsupported palette', async () => {
    const api = apiFor([{ ...row, soop: { ...row.soop!, nicknameColorKey: 'random-color1 injected' }, soopEmoticon: { ...row.soopEmoticon!, kind: 'other' as 'ogq' } }]);
    await render(api); expect(api.asset).not.toHaveBeenCalled(); expect(emote()).toBeNull(); expect(content().querySelector('.author')?.className.trim()).toBe('author');
    api.page.mockResolvedValue(page([{ ...row, kind: 'donation' }])); await render(api, { libraryId: 'not-chat' }); expect(api.asset).not.toHaveBeenCalled(); expect(emote()).toBeNull();
    api.page.mockResolvedValue(page([row])); await render(api, { provider: 'youtube', libraryId: 'youtube' }); expect(api.asset).not.toHaveBeenCalled(); expect(emote()).toBeNull();
    api.page.mockResolvedValue(page([{ ...row, soop: { ...row.soop!, role: 'streamer' } }])); await render(api, { libraryId: 'streamer' }); expect(content().querySelector('.author')).not.toHaveClass('random-color1');
  });

  it('does not read in privacy mode and rejects stale completion after changing recordings', async () => {
    const pending: Array<(value: { state: 'ready'; dataUrl: string }) => void> = [];
    const api = { page: vi.fn().mockResolvedValue(page([row])), asset: vi.fn(() => new Promise<{ state: 'ready'; dataUrl: string }>(done => pending.push(done))) };
    await render(api, { privacyMode: true }); expect(api.asset).not.toHaveBeenCalled();
    await render(api); expect(api.asset).toHaveBeenCalledTimes(2);
    await render(api, { privacyMode: true, libraryId: 'different' });
    await act(async () => pending.forEach(done => done({ state: 'ready', dataUrl: urls[0]! })));
    expect(content().querySelector('img,.platform-replay-chat-row')).toBeNull(); expect(api.asset).toHaveBeenCalledTimes(2);
  });

  it('keeps the official ordinary Nightbot color while preserving the separate OGQ palette rule', async () => {
    const rows: ReplayChatItem[] = [
      { ...row, id: 'ordinary-fan', soopEmoticon: undefined },
      { ...row, id: 'ordinary-nightbot', soopEmoticon: undefined, soop: { ...row.soop!, role: 'nightbot', badge: 'nightbot' } },
      { ...row, id: 'ogq-nightbot', soop: { ...row.soop!, role: 'nightbot', badge: 'nightbot' } },
      { ...row, id: 'ordinary-streamer', soopEmoticon: undefined, soop: { ...row.soop!, role: 'streamer', badge: 'streamer' } },
      { ...row, id: 'ogq-streamer', soop: { ...row.soop!, role: 'streamer', badge: 'streamer' } },
    ];
    await render(apiFor(rows));
    const authors = content().querySelectorAll('.author');
    expect(authors).toHaveLength(rows.length);
    expect(authors[0]).toHaveClass('random-color1');
    expect(authors[1]).not.toHaveClass('random-color1');
    expect(authors[2]).toHaveClass('random-color1');
    expect(authors[3]).not.toHaveClass('random-color1');
    expect(authors[4]).not.toHaveClass('random-color1');
    expect(content().querySelectorAll('.platform-replay-chat-text')).toHaveLength(rows.length);
  });

  it('keeps the first and last emotes readable in a full120-row page within the existing256-ID cap', async () => {
    const identifier = (n: number) => n.toString(16).padStart(64, '0');
    const rows = Array.from({ length: 120 }, (_, index) => ({ ...row, id: `row-${index}`, soopEmoticon: {
      kind: 'ogq' as const, animated: true, staticImageAssetId: identifier(index * 4 + 1), smallStaticImageAssetId: identifier(index * 4 + 2),
      animatedImageAssetId: identifier(index * 4 + 3), smallAnimatedImageAssetId: identifier(index * 4 + 4),
    } }));
    const api = { page: vi.fn().mockResolvedValue(page(rows)), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: urls[0]! }) };
    await render(api); expect(api.asset).toHaveBeenCalledTimes(240);
    const emotes = content().querySelectorAll('.emoticon'); expect(emotes).toHaveLength(120);
    expect(emotes[0]).toHaveAttribute('src', urls[0]); expect(emotes[119]).toHaveAttribute('src', urls[0]);
    expect(api.asset).not.toHaveBeenCalledWith('ogq-recording', identifier(2));
    await settings(); await toggle('OGQ 이모티콘 작게 보기');
    expect(content().querySelectorAll('.emoticon-box.small')).toHaveLength(120);
    await toggle('OGQ 이모티콘 작게 보기');
    const returned = content().querySelectorAll('.emoticon');
    expect(returned[0]).toHaveAttribute('src', urls[0]); expect(returned[119]).toHaveAttribute('src', urls[0]);
    expect(api.asset.mock.calls.length).toBeLessThanOrEqual(720);
  });
});
