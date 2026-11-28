import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type PlatformReplayChatProps, type ReplayChatApi, type ReplayChatItem, type ReplayChatPage } from './PlatformReplayChat';
import type { SoopDisplayState, SoopDisplayValues } from './platformMedia';

const defaults: SoopDisplayValues = { theme: 'light', fontLevel: 2, randomNicknameColor: false,
  fanMessageColor: false, showEmoticons: true, animateEmoticons: false, smallOgq: false };
const saved = (values: Partial<SoopDisplayValues> = {}): SoopDisplayState => ({ version: 1,
  values: { ...defaults, ...values }, basis: { theme: 'dom', fontLevel: 'dom', randomNicknameColor: 'dom',
    fanMessageColor: 'saved-option', showEmoticons: 'saved-option', animateEmoticons: 'saved-option', smallOgq: 'saved-option' },
  sequence: 1, documentEpoch: 1, observedAtMs: 1000, timeMs: 0, timingQuality: 'receive-time-approximate' });
const id = (n: number) => n.toString(16).padStart(64, '0');
const row = (key: string): ReplayChatItem => ({ id: key, timeMs: 1000, kind: 'chat', author: 'saved viewer', text: 'saved text', amount: null, badges: [], color: null,
  soop: { flag1: 32, flag2: 0, role: 'fan', badge: 'fan', subscriptionTier: null, subscriptionMonths: null, accumulatedSubscriptionMonths: null, nicknameColorKey: 'E12E2E', messageColor: '#113355' } });
const page = (display: SoopDisplayState | undefined, items: ReplayChatItem[] = [row('row')]): ReplayChatPage => ({ items, state: 'ready', timelineQuality: 'receive-time-approximate',
  nextAtMs: null, cursor: null, previousCursor: null, total: items.length, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false, soopDisplay: display });
let host: HTMLDivElement, root: Root;
async function render(api: ReplayChatApi, changes: Partial<PlatformReplayChatProps> = {}) {
  await act(async () => root.render(<PlatformReplayChat provider="soop" libraryId="saved" currentTime={2} active privacyMode={false} api={api} {...changes} />));
}
function content() { return host.querySelector('aside')!.shadowRoot!; }
async function click(selector: string) { await act(async () => (content().querySelector(selector) as HTMLElement).click()); }
const theme = () => content().querySelector('.soop-original-chat-root')?.getAttribute('dark');
const font = () => content().querySelector('#chatting_area')?.getAttribute('font-level');

describe('SOOP saved viewer presentation', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

  it('restores global settings through forward/backward seek without applying later row-creation options', async () => {
    const item = { ...row('earlier'), soopDisplay: { fanMessageColor: true, showEmoticons: true, smallOgq: false } };
    const api = { page: vi.fn(async (_id: string, position: number) => page(position >= 8000 ? saved({ theme: 'dark', fontLevel: 7, randomNicknameColor: true }) : saved(), [item])) };
    await render(api); expect(theme()).toBe('false'); expect(font()).toBe('2');
    expect(content().querySelector('#chatbox')).not.toHaveClass('random-color-on');
    expect(content().querySelector('.platform-replay-chat-text')).toHaveStyle({ color: '#113355' });
    await render(api, { currentTime: 9, seekVersion: 1 }); expect(theme()).toBe('true'); expect(font()).toBe('7');
    expect(content().querySelector('#chatbox')).toHaveClass('random-color-on');
    await render(api, { currentTime: 2, paused: true, seekVersion: 2 }); expect(theme()).toBe('false'); expect(font()).toBe('2');
    expect(api.page.mock.calls.map(call => call[1])).toEqual([2000, 9000, 2000]);
  });

  it('restores historical color variables with forward/backward seeking and source resets while keeping official role classes', async () => {
    const withPalette = (color: string) => ({ ...saved(), palette: { version: 1 as const, basis: 'computed-css-variable' as const,
      nickname: Array(20).fill(color), roles: { normal: color, streamer: color, manager: color, vip: color, subscribe: color, fan: color, support: color, nightbot: color } } });
    const api = { page: vi.fn(async (scope: string, position: number) => page(scope === 'legacy' ? undefined : withPalette(position < 5000 ? '#123456' : '#abcdef'))) };
    const color = () => (content().querySelector('#chatArea') as HTMLElement).style.getPropertyValue('--nickname-ramdom-color1');
    await render(api); expect(color()).toBe('#123456');
    expect(content().querySelector('.author')).toHaveClass('random-color1');
    expect(content().querySelector('[data-chat-row-id]')).toHaveAttribute('user-type', 'fan');
    await click('[aria-label="채팅 표시 설정"]'); await click('[aria-label="글자 크기 키우기"]');
    await render(api, { currentTime: 8 }); expect(color()).toBe('#abcdef'); expect(font()).toBe('3');
    await render(api, { currentTime: 2, paused: true, seekVersion: 1 }); expect(color()).toBe('#123456');
    await render(api, { libraryId: 'legacy' }); expect(color()).toBe('');
  });

  it('keeps explicit font/emoji overrides during time changes and resets them for another recording', async () => {
    const emote = { ...row('emoji'), soopEmoticon: { kind: 'ogq' as const, animated: false }, soopDisplay: { fanMessageColor: false, showEmoticons: true, smallOgq: false } };
    const api = { page: vi.fn(async (_id: string, position: number) => page(position >= 8000 ? saved({ theme: 'dark', fontLevel: 7 }) : saved(), [emote])) };
    await render(api); await click('[aria-label="채팅 표시 설정"]'); await click('[aria-label="글자 크기 키우기"]');
    expect(font()).toBe('3'); await click('[aria-label="이모티콘 표시 방법"]'); await click('#replay_ogq');
    expect(content().querySelector('.emoticon-box')).toHaveClass('small');
    await render(api, { currentTime: 9 }); expect(theme()).toBe('true'); expect(font()).toBe('3');
    expect(content().querySelector('.emoticon-box')).toHaveClass('small');
    expect(content()).toHaveTextContent('사용자 표시 설정');
    await render(api, { libraryId: 'different', currentTime: 9 }); expect(font()).toBe('7');
    expect(content().querySelector('.emoticon-box')).not.toHaveClass('small');
  });

  it('restores only the ordinary surface on seek and clears it for unknown history while preserving fan, staff and rich branches', async () => {
    const normal: ReplayChatItem = { ...row('ordinary'), soop: { ...row('ordinary').soop!, flag1: 0, role: 'normal', badge: 'normal', messageColor: null } };
    const fan = { ...row('fan'), soopDisplay: { fanMessageColor: true, showEmoticons: true, smallOgq: false } };
    const staff: ReplayChatItem = { ...row('staff'), soop: { ...row('staff').soop!, messageType: 'staff' } };
    const gift: ReplayChatItem = { ...row('gift'), kind: 'donation', soopGift: { kind: 'balloon', count: 211, relay: false, isDefault: true } };
    const gap = { ...row('gap'), kind: 'gap' };
    const api = { page: vi.fn(async (scope: string, position: number) => page({ ...saved({ theme: 'dark' }), surface: scope === 'legacy' ? null : {
      version: 1, basis: 'computed-dom', foreground: position < 5000 ? '#223344' : '#556677', background: position < 5000 ? '#101112' : '#202122',
    } }, [normal, fan, staff, gift, gap])) };
    const area = () => content().querySelector<HTMLElement>('#chatArea')!;
    await render(api); expect(area().style.backgroundColor).toBe('rgb(16, 17, 18)');
    expect(area().style.getPropertyValue('--soop-captured-foreground')).toBe('#223344');
    expect(area()).toHaveClass('soop-captured-foreground');
    expect(content().querySelectorAll('.soop-replay-ordinary')).toHaveLength(2);
    expect(content().querySelector('[data-chat-row-id="fan"] .msg')).toHaveStyle({ color: '#113355' });
    for (const key of ['staff', 'gift', 'gap']) expect(content().querySelector(`[data-chat-row-id="${key}"]`)).not.toHaveClass('soop-replay-ordinary');
    expect(content().querySelector<HTMLElement>('[data-chat-row-id="ordinary"] .msg')!.style.color).toBe('');
    expect(content().querySelector<HTMLElement>('#chatting_area')!.style.backgroundColor).toBe('');
    await render(api, { currentTime: 8 }); expect(area().style.getPropertyValue('--soop-captured-foreground')).toBe('#556677');
    await render(api, { currentTime: 2, paused: true, seekVersion: 1 }); expect(area().style.getPropertyValue('--soop-captured-foreground')).toBe('#223344');
    await render(api, { libraryId: 'legacy' }); expect(area().style.backgroundColor).toBe('');
    expect(area().style.getPropertyValue('--soop-captured-foreground')).toBe(''); expect(area()).not.toHaveClass('soop-captured-foreground');
  });

  it('retains row-specific fan color, hidden emotes and OGQ size while loading only the selected local sizes', async () => {
    const emote = { kind: 'ogq' as const, animated: false, staticImageAssetId: id(1), smallStaticImageAssetId: id(2) };
    const items = [
      { ...row('large'), soopEmoticon: emote, soopDisplay: { fanMessageColor: true, showEmoticons: true, smallOgq: false } },
      { ...row('small'), soopEmoticon: emote, soopDisplay: { fanMessageColor: false, showEmoticons: true, smallOgq: true } },
      { ...row('hidden'), soopEmoticon: emote, soopDisplay: { fanMessageColor: false, showEmoticons: false, smallOgq: false } },
    ];
    const api = { page: vi.fn().mockResolvedValue(page(saved({ smallOgq: true, fanMessageColor: false }), items)),
      asset: vi.fn(async (_scope: string, asset: string) => ({ state: 'ready' as const, dataUrl: `data:image/png;base64,${asset === id(1) ? 'bGFyZ2U=' : 'c21hbGw='}` })) };
    await render(api); const rows = content().querySelectorAll('[data-chat-row-id]');
    expect(rows[0]?.querySelector('.emoticon-box')).not.toHaveClass('small');
    expect(rows[1]?.querySelector('.emoticon-box')).toHaveClass('small');
    expect(rows[2]?.querySelector('.emoticon-box')).toBeNull();
    expect(rows[0]?.querySelector('.platform-replay-chat-text')).toHaveStyle({ color: '#113355' });
    expect((rows[1]?.querySelector('.platform-replay-chat-text') as HTMLElement).style.color).toBe('');
    expect(api.asset.mock.calls.map(call => call[1]).sort()).toEqual([id(1), id(2)]);
  });

  it('selects the light/dark official fallback and current animation while preserving row size', async () => {
    const item = { ...row('image'), soopEmoticon: { kind: 'ogq' as const, animated: true }, soopDisplay: { fanMessageColor: null, showEmoticons: true, smallOgq: true } };
    const api = { page: vi.fn(async (_id: string, position: number) => page(saved({ theme: position < 5000 ? 'light' : 'dark', animateEmoticons: position >= 5000 }), [item])) };
    await render(api); expect(content().querySelector('.emoticon')).toHaveAttribute('src', '/assets/soop/ogq_default.svg');
    await render(api, { currentTime: 6 }); expect(content().querySelector('.emoticon')).toHaveAttribute('src', '/assets/soop/ogq_default_dark.svg');
    expect(content().querySelector('.emoticon-box')).toHaveClass('small');
  });

  it.each(['ready', 'empty', 'unmapped'] as const)('updates a mapped %s recording with settings and no chat during quiet intervals and paused seeking', async state => {
    const api = { page: vi.fn(async (_id: string, position: number) => ({ ...page(saved({ fontLevel: position >= 5000 ? 8 : 2 }), []), state })) };
    await render(api); expect(font()).toBe('2');
    await render(api, { currentTime: 6 }); expect(font()).toBe('8');
    await render(api, { currentTime: 2.25, paused: true, seekVersion: 1 }); expect(font()).toBe('2');
    expect(api.page.mock.calls.map(call => call[1])).toEqual([2000, 6000, 2250]);
    expect(content().querySelector('[data-chat-row-id]')).toBeNull();
  });

  it('does not label unknown or wrongly sourced values as recorded preferences, and preserves legacy defaults', async () => {
    const invalid = saved({ theme: 'light', fontLevel: 99, randomNicknameColor: null });
    invalid.basis.theme = 'saved-option';
    invalid.basis.fanMessageColor = invalid.basis.showEmoticons = invalid.basis.animateEmoticons = invalid.basis.smallOgq = null;
    const api = { page: vi.fn().mockResolvedValue(page(invalid)) }; await render(api);
    expect(theme()).toBe('true'); expect(font()).toBe('3'); expect(content().querySelector('#chatbox')).toHaveClass('random-color-on');
    await click('[aria-label="채팅 표시 설정"]'); expect(content()).toHaveTextContent('당시 표시 설정 없음');
    api.page.mockResolvedValue(page(undefined)); await render(api, { libraryId: 'legacy' });
    expect(theme()).toBe('true'); expect(font()).toBe('3');
    api.page.mockResolvedValue({ ...page(undefined), soopDisplayIncomplete: true }); await render(api, { libraryId: 'partial' });
    await click('[aria-label="채팅 표시 설정"]'); expect(content()).toHaveTextContent('당시 표시 설정 일부 누락');
    expect(content()).toHaveTextContent('saved text');
  });

  it('accepts initialized DOM evidence for optional settings without accepting storage as evidence of the visible theme', async () => {
    const display = saved({ animateEmoticons: false }); display.basis.animateEmoticons = 'dom';
    const api = { page: vi.fn().mockResolvedValue(page(display)) }; await render(api);
    await click('[aria-label="채팅 표시 설정"]'); await click('[aria-label="이모티콘 표시 방법"]');
    expect(content().querySelector('#replay_emoticon_ani')).not.toBeChecked();
    expect(content()).toHaveTextContent('저장 당시 표시 설정');
  });

  it('fences a previous recording setting response and overrides while the new recording is loading', async () => {
    const finish = new Map<string, (value: ReplayChatPage) => void>();
    const api = { page: vi.fn((scope: string) => new Promise<ReplayChatPage>(done => finish.set(scope, done))) };
    await render(api); await act(async () => finish.get('saved')!(page(saved({ fontLevel: 6 }))));
    await click('[aria-label="채팅 표시 설정"]'); await click('[aria-label="글자 크기 키우기"]'); expect(font()).toBe('7');
    await render(api, { libraryId: 'next' }); expect(theme()).toBe('true'); expect(font()).toBe('3');
    await render(api, { libraryId: 'last' });
    await act(async () => finish.get('next')!(page(saved({ fontLevel: 8 }))));
    expect(font()).toBe('3');
    await act(async () => finish.get('last')!(page(saved({ fontLevel: 1 }))));
    expect(font()).toBe('1'); expect(theme()).toBe('false');
  });
});
