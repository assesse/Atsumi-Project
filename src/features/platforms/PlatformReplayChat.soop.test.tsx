import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type PlatformReplayChatProps, type ReplayChatApi, type ReplayChatItem, type ReplayChatPage } from './PlatformReplayChat';

const row: ReplayChatItem = { id: 'saved-message', timeMs: 1000, kind: 'chat', author: '저장된 닉네임', text: '<img src=x onerror=bad> 저장된 본문', amount: null, badges: ['구독 3개월'], color: '#55AAFF' };
const sample = (changes: Partial<ReplayChatPage> = {}): ReplayChatPage => ({ items: [row], state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: 1, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false, ...changes });
const imageId = 'a'.repeat(64);
const imageUrl = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==';
let host: HTMLDivElement, root: Root;
async function render(api: ReplayChatApi, changes: Partial<PlatformReplayChatProps> = {}) { await act(async () => root.render(<PlatformReplayChat provider="soop" libraryId="saved-soop" currentTime={2} active privacyMode={false} api={api} {...changes} />)); }
function content(): HTMLElement { return host.querySelector('aside')?.shadowRoot?.querySelector<HTMLElement>('.soop-original-chat-root') ?? host; }
async function click(node: Element) { await act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }))); }
async function key(node: EventTarget, value: string) { await act(async () => node.dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true, composed: true, cancelable: true }))); }
function action(label: string): HTMLElement { const node = content().querySelector<HTMLElement>(`button[aria-label="${label}"],[role="button"][aria-label="${label}"]`); expect(node, `action ${label}`).not.toBeNull(); return node!; }
function checkbox(label: string): HTMLInputElement {
  const target = [...content().querySelectorAll('label')].find(value => value.textContent === label);
  const node = target?.querySelector<HTMLInputElement>('input[type="checkbox"]') ?? (target?.htmlFor ? content().querySelector<HTMLInputElement>(`input[id="${target.htmlFor}"]`) : null);
  expect(node, `checkbox ${label}`).not.toBeNull(); return node!;
}
function settings(): HTMLElement | null { return content().querySelector('[role="dialog"][aria-label="채팅 설정"]'); }
function settingsValue() { return content().querySelector('[aria-label="채팅 글자 크기"]')?.textContent?.trim(); }

describe('SOOP original LIVE saved-chat interactions', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });

  it('isolates the official surface and operates original font settings without re-reading the archive', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api);
    expect(host.querySelector('aside')?.shadowRoot).not.toBeNull(); expect(host.querySelector('.chatbox')).toBeNull();
    expect(content().querySelector('#chatting_area #chatbox .chat_title')).not.toBeNull();
    const toggle = action('채팅 표시 설정'); expect(toggle).toHaveAttribute('role', 'button'); expect(toggle).toHaveAttribute('tabindex', '0'); expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(content().textContent).not.toContain('수신 시각 기준'); await key(toggle, 'Enter');
    expect(settings()).not.toBeNull(); expect(toggle).toHaveAttribute('aria-expanded', 'true'); expect(settings()).toHaveTextContent('수신 시각 기준'); expect(settingsValue()).toBe('3');
    expect(settings()?.querySelector('input[type="range"]')).toBeNull();
    await click(action('글자 크기 줄이기')); expect(settingsValue()).toBe('2'); await click(action('글자 크기 줄이기')); expect(settingsValue()).toBe('1'); expect(action('글자 크기 줄이기')).toBeDisabled();
    for (let level = 2; level <= 8; level += 1) { await click(action('글자 크기 키우기')); expect(settingsValue()).toBe(String(level)); }
    expect(action('글자 크기 키우기')).toBeDisabled(); expect(action('글자 크기 줄이기')).not.toBeDisabled();
    await key(action('채팅 설정 닫기'), ' '); expect(settings()).toBeNull(); await click(toggle); expect(settingsValue()).toBe('8');
    expect(api.page).toHaveBeenCalledExactlyOnceWith('saved-soop', 2000, null, 120);
  });

  it('uses original subpanels for saved timestamps and local emotes without changing the media clock', async () => {
    const richRow: ReplayChatItem = { ...row, text: '앞 :flower: 뒤', parts: [{ type: 'text', text: '앞 ' }, { type: 'emoji', assetId: imageId, alt: ':flower:' }, { type: 'text', text: ' 뒤' }] };
    const api = { page: vi.fn().mockResolvedValue(sample({ items: [richRow] })), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl: imageUrl }) };
    await render(api); expect(content().querySelector('time')).toBeNull(); expect(content().querySelector('img')).toHaveAttribute('src', imageUrl); expect(api.asset).toHaveBeenCalledExactlyOnceWith('saved-soop', imageId);
    await click(action('채팅 표시 설정')); await click(action('개인 설정')); expect(checkbox('시각 표시')).not.toBeChecked(); await click(checkbox('시각 표시')); expect(content().querySelector('time')).toHaveTextContent('0:01');
    await click(action('설정으로 돌아가기')); await click(action('이모티콘 표시 방법')); expect(checkbox('이모티콘 보기')).toBeChecked(); await click(checkbox('이모티콘 보기'));
    expect(content().querySelector('img')).toBeNull(); expect(content().querySelector('.platform-replay-chat-text')).toHaveTextContent(richRow.text);
    await click(checkbox('이모티콘 보기')); expect(content().querySelector('img')).toHaveAttribute('src', imageUrl);
    await click(action('설정으로 돌아가기')); await click(action('개인 설정')); await click(checkbox('시각 표시')); expect(content().querySelector('time')).toBeNull();
    expect(api.page).toHaveBeenCalledExactlyOnceWith('saved-soop', 2000, null, 120); expect(api.asset).toHaveBeenCalledTimes(1);
  });

  it('uses explicit role metadata and keeps legacy badge strings in the local profile without inventing rank icons', async () => {
    const flagged: ReplayChatItem = { ...row, id: 'flagged', soop: { flag1: 0, flag2: 0, role: 'fan', badge: 'fan', subscriptionTier: null, subscriptionMonths: null, accumulatedSubscriptionMonths: null } };
    const noExtras: ReplayChatItem = { ...row, id: 'plain', author: '다른 닉네임', text: '일반 본문', badges: [], color: 'url(https://invalid.test/color)' };
    const api = { page: vi.fn().mockResolvedValue(sample({ items: [row, flagged, noExtras], total: 3 })) }; await render(api);
    const rows = content().querySelectorAll('.platform-replay-chat-row'); expect(rows[0]?.querySelector('[class^="grade-badge-"]')).toBeNull(); expect(rows[0]?.textContent).not.toContain('구독 3개월');
    expect(rows[1]).toHaveAttribute('user-type', 'fan'); expect(rows[1]?.querySelector('.grade-badge-fan')).toHaveAttribute('aria-label', '팬클럽'); expect(rows[2]?.querySelector('[class^="grade-badge-"]')).toBeNull();
    expect((rows[0]?.querySelector('.author') as HTMLElement).style.color).toBe(''); expect((rows[2]?.querySelector('.author') as HTMLElement).style.color).toBe(''); expect(rows[0]?.querySelector('.platform-replay-chat-text')).toHaveTextContent(row.text);
    expect(content().querySelectorAll('img,iframe,video,script,a[href]')).toHaveLength(0); expect(content().querySelector('textarea,input[type="text"],[contenteditable="true"]')).toBeNull();
    await click(rows[0]!.querySelector('.platform-replay-chat-author')!); const popup = content().querySelector('[role="dialog"][aria-label="채팅 사용자 정보"]');
    expect(popup).toHaveTextContent('저장 당시 정보'); expect(popup).toHaveTextContent('구독 3개월'); expect(popup?.textContent).not.toMatch(/선물|신고|차단|로그인|전송/);
    await key(action('사용자 정보 닫기'), 'Escape'); expect(content().querySelector('[aria-label="채팅 사용자 정보"]')).toBeNull();
  });

  it('keeps partial-history warnings visible outside closed settings', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample({ partialFile: true })) }; await render(api); expect(settings()).toBeNull(); expect(content().textContent).not.toContain('수신 시각 기준');
    const warning = content().querySelector('[title="채팅 보존 상태가 완전히 확인되지 않았습니다. 원본 기록은 변경하지 않습니다."]'); expect(warning).toHaveTextContent('일부 기록만 표시될 수 있습니다.');
    await click(action('채팅 표시 설정')); expect(settings()).toHaveTextContent('수신 시각 기준'); expect(warning?.closest('[role="dialog"]')).toBeNull(); await click(action('채팅 설정 닫기')); expect(warning?.isConnected).toBe(true);
  });

  it('keeps healthy chat visible when optional badge metadata is unavailable without retries or invented badges', async () => {
    vi.useFakeTimers();
    const api = { page: vi.fn().mockResolvedValueOnce(sample({ displayMetadataUnavailable: true })).mockResolvedValue(sample()) };
    await render(api);
    const warning = content().querySelector('[title="배지용 보조 기록을 읽지 못했습니다. 저장된 채팅은 계속 재생됩니다."]');
    expect(warning).toHaveTextContent('배지 정보를 불러오지 못했습니다.');
    expect(warning?.closest('[role="dialog"]')).toBeNull(); expect(settings()).toBeNull();
    expect(content().querySelector('.platform-replay-chat-text')).toHaveTextContent(row.text);
    expect(content().querySelector('[class^="grade-badge-"],.thumb,[role="alert"]')).toBeNull();
    expect(content().textContent).not.toContain('다시 시도');
    await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
    expect(api.page).toHaveBeenCalledExactlyOnceWith('saved-soop', 2000, null, 120);
    await render(api, { privacyMode: true });
    expect(content().textContent).not.toContain('배지 정보를 불러오지 못했습니다.');
    expect(content().querySelector('.platform-replay-chat-row')).toBeNull(); expect(api.page).toHaveBeenCalledOnce();
    await render(api, { libraryId: 'available-metadata' });
    expect(content().textContent).not.toContain('배지 정보를 불러오지 못했습니다.');
    expect(content().querySelector('.platform-replay-chat-text')).toHaveTextContent(row.text); expect(api.page).toHaveBeenCalledTimes(2);
  });

  it('shows an explicitly qualified local subscriber fallback only when the archive supplies a subscription tier', async () => {
    const subscribed: ReplayChatItem = { ...row, id: 'subscriber', soop: { flag1: 32, flag2: 0, role: 'subscribe', badge: 'fan', subscriptionTier: 1, subscriptionMonths: 17, accumulatedSubscriptionMonths: 19 } };
    const api = { page: vi.fn().mockResolvedValue(sample({ items: [row, subscribed], total: 2 })) }; await render(api);
    const rows = content().querySelectorAll('.platform-replay-chat-row');
    expect(rows[0]?.querySelector('.thumb')).toBeNull();
    expect(rows[1]).toHaveAttribute('user-type', 'subscribe');
    expect(rows[1]?.querySelector('.grade-badge-fan')).not.toBeNull();
    expect(rows[1]?.querySelector('.grade-badge-subscribe')).toBeNull();
    expect(rows[1]?.querySelector('.thumb')).toHaveAttribute('title', '구독 기본 아이콘 · 당시 채널별 아이콘은 저장되지 않았습니다.');
    expect(rows[1]?.querySelector('.thumb img')).toHaveAttribute('src', '/assets/soop/signature-default.svg');
    expect(rows[1]?.querySelector('.thumb img')).toHaveAttribute('alt', '구독 기본 퍼스나콘');
    expect(rows[1]?.querySelector('.tier_tip')).toHaveTextContent('구독 1단계 · 누적 19개월');
    expect(api.page).toHaveBeenCalledExactlyOnceWith('saved-soop', 2000, null, 120);
  });

  it('retains LIVE footer composition without enabling sending, emotes or payment in a recording', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api);
    const footer = content().querySelector('.actionbox')!;
    expect(footer).not.toBeNull(); expect(footer).toHaveAttribute('aria-label', '녹화 채팅 · 전송할 수 없음');
    const input = footer.querySelector('[role="textbox"]');
    expect(input).toHaveTextContent('녹화된 채팅입니다.'); expect(input).toHaveAttribute('aria-readonly', 'true'); expect(input).toHaveAttribute('aria-disabled', 'true'); expect(input).not.toHaveAttribute('contenteditable', 'true');
    expect(footer.querySelector('textarea,input,[contenteditable="true"],a[href]')).toBeNull();
    const actions = footer.querySelectorAll<HTMLButtonElement | HTMLAnchorElement>('button,[role="button"]'); expect(actions.length).toBeGreaterThanOrEqual(4);
    for (const node of actions) {
      if (node instanceof HTMLButtonElement) expect(node).toBeDisabled();
      else { expect(node).toHaveAttribute('aria-disabled', 'true'); expect(node).not.toHaveAttribute('href'); expect(node).not.toHaveAttribute('tabindex', '0'); }
      await click(node); await key(node, 'Enter');
    }
    expect(settings()).toBeNull(); expect(api.page).toHaveBeenCalledExactlyOnceWith('saved-soop', 2000, null, 120);
    await render(api, { privacyMode: true }); expect(content().querySelector('.actionbox')).toBeNull();
  });

  it('uses stored message color only with its explicit coloring flag and never guesses a nickname color', async () => {
    const metadata: NonNullable<ReplayChatItem['soop']> = { flag1: 32, flag2: 0, role: 'fan', badge: 'fan', subscriptionTier: null, subscriptionMonths: null, accumulatedSubscriptionMonths: null, messageColor: '#113355' };
    const api = { page: vi.fn().mockResolvedValue(sample({ items: [
      { ...row, id: 'color', color: null, soop: metadata },
      { ...row, id: 'no-flag', color: null, soop: { ...metadata, flag1: 0 } },
      { ...row, id: 'invalid', color: null, soop: { ...metadata, messageColor: 'url(https://invalid.test/color)' } },
    ], total: 3 })) }; await render(api);
    const rows = content().querySelectorAll('.platform-replay-chat-row');
    expect(rows[0]?.querySelector('.platform-replay-chat-text')).toHaveStyle({ color: '#113355' });
    for (const node of rows) expect((node.querySelector('.author') as HTMLElement).style.color).toBe('');
    expect((rows[1]?.querySelector('.platform-replay-chat-text') as HTMLElement).style.color).toBe('');
    expect((rows[2]?.querySelector('.platform-replay-chat-text') as HTMLElement).style.color).toBe('');
  });

  it('preserves local close and separates settings Escape/outside handling from replay close', async () => {
    const close = vi.fn(), api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api, { onClose: close }); await click(action('채팅 표시 설정'));
    await act(async () => action('개인 설정').dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, composed: true }))); expect(settings()).not.toBeNull();
    await click(action('개인 설정')); await key(checkbox('시각 표시'), 'Escape'); expect(settings()).toBeNull(); expect(close).not.toHaveBeenCalled();
    await click(action('채팅 표시 설정')); await act(async () => document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, composed: true }))); expect(settings()).toBeNull(); expect(close).not.toHaveBeenCalled();
    await key(action('재생 닫기'), 'Enter'); expect(close).toHaveBeenCalledOnce(); await render(api); expect(content().querySelector('[aria-label="재생 닫기"]')).toBeNull(); expect(content().querySelectorAll('.chat_title [role="button"]')).toHaveLength(1);
  });

  it('blocks gear pointer/keyboard actions during privacy and hides saved content without further reads', async () => {
    const close = vi.fn(), api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api, { privacyMode: true, onClose: close }); expect(api.page).not.toHaveBeenCalled();
    expect(action('채팅 표시 설정')).toHaveAttribute('aria-disabled', 'true'); expect(action('채팅 표시 설정')).toHaveAttribute('tabindex', '-1'); await click(action('채팅 표시 설정')); await key(action('채팅 표시 설정'), 'Enter'); expect(settings()).toBeNull();
    await render(api, { onClose: close }); await click(action('채팅 표시 설정')); expect(settings()).not.toBeNull(); await render(api, { privacyMode: true, onClose: close }); expect(settings()).toBeNull();
    expect(content().textContent).not.toContain(row.author); expect(content().querySelectorAll('.platform-replay-chat-row,img')).toHaveLength(0); expect(action('채팅 표시 설정')).toHaveAttribute('aria-disabled', 'true'); expect(action('재생 닫기')).not.toHaveAttribute('aria-disabled', 'true'); expect(api.page).toHaveBeenCalledOnce();
  });

  it('follows only parent video time and preserves manual history inside the shadow surface', async () => {
    const api = { page: vi.fn().mockImplementation(async (_id, ms) => sample({ items: ms < 1000 ? [] : [row] })) }; await render(api);
    const scroll = content().querySelector<HTMLDivElement>('.platform-replay-chat-messages')!; Object.defineProperties(scroll, { scrollHeight: { configurable: true, value: 1000 }, clientHeight: { configurable: true, value: 200 } });
    await act(async () => { scroll.scrollTop = 100; scroll.dispatchEvent(new Event('scroll', { bubbles: true, composed: true })); }); await render(api, { currentTime: 15 }); expect(api.page).toHaveBeenCalledTimes(1); expect(scroll.scrollTop).toBe(100);
    await click(action('현재 시각으로')); expect(api.page).toHaveBeenLastCalledWith('saved-soop', 15000, null, 120); expect(scroll.scrollTop).toBe(1000);
    await render(api, { currentTime: 0 }); expect(api.page).toHaveBeenLastCalledWith('saved-soop', 0, null, 120); expect(content().querySelector('.platform-replay-chat-row')).toBeNull();
  });

  it('has one isolated surface and does not read when inactive', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api, { active: false }); expect(api.page).not.toHaveBeenCalled(); expect(host.querySelector('aside')).toHaveAttribute('hidden');
    await render(api); expect(host.querySelectorAll('aside')).toHaveLength(1); expect(host.querySelector('aside')).not.toHaveAttribute('hidden'); expect(content().querySelectorAll('#chatbox')).toHaveLength(1); expect(host.querySelector('style')).toBeNull(); expect(api.page).toHaveBeenCalledOnce();
  });

  it('leaves YouTube markup, settings, visible timing and author presentation unchanged', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api, { provider: 'youtube' }); expect(host.querySelector('aside')?.shadowRoot).toBeNull(); expect(content().textContent).toContain('수신 시각 기준');
    expect(content().querySelector('[class^="grade-badge-"]')).toBeNull(); expect(content().querySelector('[aria-label="채팅 글자 크기"]')).toBeNull(); await click(action('채팅 표시 설정')); expect(settings()).toBeNull(); expect(content().querySelector('[aria-label="글자 크기 키우기"]')).toBeNull();
    expect([...content().querySelectorAll('label')].some(node => node.textContent?.includes('이모티콘'))).toBe(false); await click(checkbox('시각 표시')); expect(content().querySelector('time')).toHaveTextContent('0:01'); expect(api.page).toHaveBeenCalledExactlyOnceWith('saved-soop', 2000, null, 120);
  });
});
