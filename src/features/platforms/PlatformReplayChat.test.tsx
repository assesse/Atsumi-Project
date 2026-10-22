import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type ReplayChatApi, type ReplayChatPage, type PlatformReplayChatProps } from './PlatformReplayChat';

const row = { id: 'fixture', timeMs: 1000, kind: 'chat', author: '작성자', text: '<img src=x onerror=bad>', amount: null, badges: ['구독 3개월'], color: null };
const sample = (changes: Partial<ReplayChatPage> = {}): ReplayChatPage => ({ items: [row], state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: 1, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false, ...changes });
const preparing = (scannedBytes = 20, totalBytes = 100): ReplayChatPage => sample({ state: 'indexing', items: [], total: 0, timelineQuality: 'missing-anchor', preparation: { scannedBytes, totalBytes } });
let host: HTMLDivElement, root: Root;
async function render(api: ReplayChatApi, changes: Partial<PlatformReplayChatProps> = {}) { await act(async () => root.render(<PlatformReplayChat libraryId="fixture" currentTime={2} active privacyMode={false} api={api} {...changes} />)); }
async function click(node: Element) { await act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true }))); }
describe('platform replay chat', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });
  it('does not read or expose any saved chat in privacy mode or while inactive', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api, { privacyMode: true }); expect(api.page).not.toHaveBeenCalled(); expect(host.textContent).toContain('프라이버시 모드'); expect(host.textContent).not.toContain(row.author);
    await render(api, { active: false }); expect(api.page).not.toHaveBeenCalled();
  });
  it('renders stored text as text, shows approximate timing and opens a local author popup', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api);
    expect(host.querySelector('img')).toBeNull(); expect(host.querySelector('.platform-replay-chat-text')?.textContent).toBe(row.text); expect(host.textContent).toContain('수신 시각 기준');
    await click(host.querySelector('.platform-replay-chat-author')!); expect(host.querySelector('[role="dialog"]')?.textContent).toContain('구독 3개월'); expect(host.querySelector('a')).toBeNull();
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))); expect(host.querySelector('[role="dialog"]')).toBeNull();
  });
  it('keeps close beside chat settings and does not create account actions', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample()) }, close = vi.fn(); await render(api, { onClose: close });
    const buttons = host.querySelectorAll('.platform-replay-chat-header button'); expect(buttons).toHaveLength(2); expect(buttons[1]?.getAttribute('aria-label')).toBe('재생 닫기'); await click(buttons[1]!); expect(close).toHaveBeenCalledOnce();
    expect(host.textContent).not.toMatch(/선물|신고|차단/);
  });
  it.each(['youtube', 'soop'] as const)('uses the %s presentation without inventing historical profiles or remote controls', async provider => {
    const api = { page: vi.fn().mockResolvedValue(sample({ items: [{ ...row, color: '#55AAFF' }], partialFile: true })) };
    await render(api, { provider, onClose: vi.fn() });
    expect(host.querySelector('aside')).toHaveAttribute('data-provider', provider);
    expect(host.querySelectorAll('.platform-replay-chat-header button svg')).toHaveLength(2);
    expect(host.querySelectorAll('img,iframe,video,script,a')).toHaveLength(0);
    expect(host.querySelector('.platform-replay-chat-author')).toHaveStyle({ color: '#55AAFF' });
    expect(host.querySelector('.platform-replay-chat-text')).toHaveTextContent(row.text);
    expect(host.textContent).toContain('일부 기록만 표시'); expect(host.textContent).not.toMatch(/로그인|전송|시청자/);
    expect(api.page).toHaveBeenCalledExactlyOnceWith('fixture', 2000, null, 120);
    await render(api, { provider, privacyMode: true, onClose: vi.fn() });
    expect(host.querySelector('[aria-label="채팅 표시 설정"]')).toBeDisabled();
    expect(host.querySelector('[aria-label="재생 닫기"]')).not.toBeDisabled(); expect(host.textContent).not.toContain(row.author);
  });
  it.each([
    { partialFile: true, malformedLines: 0 },
    { partialFile: false, malformedLines: 1 },
    { partialFile: true, malformedLines: 1 },
    { partialFile: false, malformedLines: 0 },
  ])('distinguishes unverified preservation from actual unreadable rows: %j', async status => {
    const api = { page: vi.fn().mockResolvedValue(sample(status)) };
    await render(api);
    const note = [...host.querySelectorAll('.platform-replay-chat-note')].find(node => node.textContent === '일부 기록만 표시될 수 있습니다.');
    if (!status.partialFile && !status.malformedLines) expect(note).toBeUndefined();
    else {
      expect(note).toBeDefined();
      expect(note!.getAttribute('title')).toBe(status.malformedLines > 0
        ? '읽을 수 없는 기록은 건너뛰었습니다. 원본 기록은 변경하지 않습니다.'
        : '채팅 보존 상태가 완전히 확인되지 않았습니다. 원본 기록은 변경하지 않습니다.');
      expect(note!.getAttribute('title')).not.toMatch(/정상|미제공|제공하지 않|문제없음/);
      if (!status.malformedLines) expect(note!.getAttribute('title')).not.toContain('건너뛰');
    }
    expect(host.querySelector('.platform-replay-chat-text')).toHaveTextContent(row.text);
    expect(api.page).toHaveBeenCalledExactlyOnceWith('fixture', 2000, null, 120);
  });
  it('does not refresh unmapped chronology every playback second and pages manually', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample({ state: 'unmapped', timelineQuality: 'missing-anchor', items: [{ ...row, timeMs: null }], cursor: '120' })) };
    await render(api); const calls = api.page.mock.calls.length; await render(api, { currentTime: 22 }); expect(api.page).toHaveBeenCalledTimes(calls);
    expect(host.textContent).toContain('시간 정보 없음'); expect(host.querySelector('time')).toBeNull();
    await click([...host.querySelectorAll('button')].find(node => node.textContent === '다음')!); expect(api.page).toHaveBeenLastCalledWith('fixture', 0, '120', 120);
    expect(host.querySelector('.platform-replay-chat-messages')?.scrollTop).toBe(0);
  });
  it('seeks backwards after the end without retaining future messages', async () => {
    const api = { page: vi.fn().mockImplementation(async (_id, ms) => sample({ items: ms < 1000 ? [] : [row] })) };
    await render(api, { currentTime: 600 }); expect(host.textContent).toContain(row.text);
    await render(api, { currentTime: 0 }); expect(api.page).toHaveBeenLastCalledWith('fixture', 0, null, 120); expect(host.textContent).not.toContain(row.text);
  });
  it.each(['missing', 'empty'] as const)('does not repeatedly read a completed %s archive as video time advances', async state => {
    const api = { page: vi.fn().mockResolvedValue(sample({ state, items: [], total: 0, timelineQuality: 'missing-anchor' })) };
    await render(api); const calls = api.page.mock.calls.length;
    await render(api, { currentTime: 20 }); await render(api, { currentTime: 30 });
    expect(api.page).toHaveBeenCalledTimes(calls);
  });
  it.each(['missing', 'empty'] as const)('does not infer disabled or complete chat from an unverified zero-row %s result', async state => {
    const api = { page: vi.fn().mockResolvedValue(sample({ state, items: [], total: 0, timelineQuality: 'missing-anchor', partialFile: true })) };
    await render(api); const calls = api.page.mock.calls.length;
    expect(host.querySelector('[title="채팅 보존 상태가 완전히 확인되지 않았습니다. 원본 기록은 변경하지 않습니다."]')).toHaveTextContent('일부 기록만 표시될 수 있습니다.');
    expect(host.textContent).not.toMatch(/정상|미제공|제공하지 않|문제없음/);
    expect(host.querySelector('.platform-replay-chat-empty')).toHaveTextContent(state === 'missing' ? '저장된 채팅이 없습니다.' : '표시할 채팅이 없습니다.');
    await render(api, { currentTime: 30 }); expect(api.page).toHaveBeenCalledTimes(calls);
  });
  it('keeps historical scroll until the user returns to the current position', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample()) }; await render(api);
    const scroll = host.querySelector<HTMLDivElement>('.platform-replay-chat-messages')!;
    Object.defineProperties(scroll, { scrollHeight: { configurable: true, value: 1000 }, clientHeight: { configurable: true, value: 200 } });
    await act(async () => { scroll.scrollTop = 100; scroll.dispatchEvent(new Event('scroll', { bubbles: true })); });
    const calls = api.page.mock.calls.length; await render(api, { currentTime: 15 }); expect(api.page).toHaveBeenCalledTimes(calls); expect(scroll.scrollTop).toBe(100);
    await click(host.querySelector('.platform-replay-chat-follow')!); expect(api.page).toHaveBeenLastCalledWith('fixture', 15000, null, 120); expect(scroll.scrollTop).toBe(1000);
  });
  it('allows at most one in-flight native call and discards obsolete archive responses', async () => {
    let resolve!: (page: ReplayChatPage) => void;
    const api = { page: vi.fn().mockImplementationOnce(() => new Promise<ReplayChatPage>(done => { resolve = done; })).mockResolvedValue(sample({ items: [{ ...row, text: 'new archive' }] })) };
    await render(api); await render(api, { libraryId: 'new', currentTime: 9 }); await render(api, { libraryId: 'new', currentTime: 10 }); expect(api.page).toHaveBeenCalledTimes(1);
    await act(async () => resolve(sample())); expect(api.page).toHaveBeenCalledTimes(2); expect(api.page).toHaveBeenLastCalledWith('new', 10000, null, 120); expect(host.textContent).toContain('new archive'); expect(host.textContent).not.toContain(row.text);
  });
  it('does not display a pending response after privacy is enabled', async () => {
    let resolve!: (page: ReplayChatPage) => void; const api = { page: vi.fn().mockImplementation(() => new Promise<ReplayChatPage>(done => { resolve = done; })) };
    await render(api); await render(api, { privacyMode: true }); await act(async () => resolve(sample())); expect(host.textContent).not.toContain(row.author); expect(host.querySelector('.platform-replay-chat-row')).toBeNull();
  });
  it('offers retry on read failure while retaining the existing archive display', async () => {
    const api = { page: vi.fn().mockResolvedValueOnce(sample()).mockRejectedValueOnce(new Error('do not expose local file path')).mockResolvedValue(sample()) };
    await render(api); await render(api, { currentTime: 4 }); expect(host.textContent).toContain('채팅을 불러오지 못했습니다.'); expect(host.textContent).not.toContain('local file path'); expect(host.textContent).toContain(row.text);
    await click([...host.querySelectorAll('button')].find(node => node.textContent === '다시 시도')!); expect(host.querySelector('[role="alert"]')).toBeNull();
  });
  it('shows only explicit amount and membership data, and timestamps toggle without moving buttons', async () => {
    const api = { page: vi.fn().mockResolvedValue(sample({ items: [{ ...row, kind: 'donation', amount: '₩1,000' }] })) }; await render(api);
    expect(host.querySelector('.platform-replay-chat-amount')?.textContent).toBe('₩1,000'); expect(host.querySelector('time')).toBeNull();
    await click(host.querySelector('[aria-label="채팅 표시 설정"]')!); await click(host.querySelector('input[type="checkbox"]')!); expect(host.querySelector('time')?.textContent).toBe('0:01');
  });
  it('prepares in timer-driven slices and uses the latest paused/seek position without per-second reads', async () => {
    vi.useFakeTimers();
    const api = { page: vi.fn().mockResolvedValueOnce(preparing()).mockResolvedValueOnce(preparing(80)).mockResolvedValue(sample()) };
    await render(api); expect(api.page).toHaveBeenCalledTimes(1);
    expect(host.querySelector('[role="status"]')?.textContent).toContain('채팅 준비 중20%');
    expect(host.textContent).not.toContain('표시할 채팅이 없습니다.');
    await render(api, { currentTime: 25 }); await render(api, { currentTime: 45 });
    expect(api.page).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(99); }); expect(api.page).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(api.page).toHaveBeenLastCalledWith('fixture', 45000, null, 120);
    expect(host.querySelector('[role="status"]')?.textContent).toContain('80%');
    await act(async () => { vi.advanceTimersByTime(100); });
    expect(host.querySelector('.platform-replay-chat-preparing')).toBeNull(); expect(host.textContent).toContain(row.text);
    const settled = api.page.mock.calls.length;
    await act(async () => { vi.advanceTimersByTime(1000); }); expect(api.page).toHaveBeenCalledTimes(settled);
  });
  it('does not overlap preparation requests and stops its timer on failure until explicit retry', async () => {
    vi.useFakeTimers(); let reject!: (reason: Error) => void;
    const api = { page: vi.fn().mockResolvedValueOnce(preparing()).mockImplementationOnce(() => new Promise<ReplayChatPage>((_resolve, fail) => { reject = fail; })).mockResolvedValue(sample()) };
    await render(api);
    await act(async () => { vi.advanceTimersByTime(100); }); expect(api.page).toHaveBeenCalledTimes(2);
    await act(async () => { vi.advanceTimersByTime(5000); }); expect(api.page).toHaveBeenCalledTimes(2);
    await act(async () => reject(new Error('private native detail')));
    expect(host.querySelector('[role="alert"]')).not.toBeNull(); expect(host.querySelector('.platform-replay-chat-preparing')).toBeNull();
    await act(async () => { vi.advanceTimersByTime(5000); }); expect(api.page).toHaveBeenCalledTimes(2);
    await click([...host.querySelectorAll('button')].find(node => node.textContent === '다시 시도')!);
    expect(host.querySelector('[role="alert"]')).toBeNull(); expect(host.textContent).toContain(row.text);
  });
  it.each(['privacy', 'inactive', 'unmount'] as const)('cancels preparation scheduling when %s and discards a late reply', async mode => {
    vi.useFakeTimers(); let resolve!: (page: ReplayChatPage) => void;
    const api = { page: vi.fn().mockResolvedValueOnce(preparing()).mockImplementationOnce(() => new Promise<ReplayChatPage>(done => { resolve = done; })) };
    await render(api); await act(async () => { vi.advanceTimersByTime(100); }); expect(api.page).toHaveBeenCalledTimes(2);
    if (mode === 'unmount') await act(async () => root.render(null));
    else await render(api, mode === 'privacy' ? { privacyMode: true } : { active: false });
    await act(async () => resolve(sample())); await act(async () => { vi.advanceTimersByTime(5000); });
    expect(api.page).toHaveBeenCalledTimes(2); expect(host.textContent).not.toContain(row.text);
  });
  it('cancels the scheduled preparation timer before privacy and ignores the old library during a switch', async () => {
    vi.useFakeTimers(); let resolve!: (page: ReplayChatPage) => void;
    const api = { page: vi.fn().mockResolvedValueOnce(preparing()).mockImplementationOnce(() => new Promise<ReplayChatPage>(done => { resolve = done; })).mockResolvedValue(sample({ items: [{ ...row, text: 'new record' }] })) };
    await render(api); await render(api, { privacyMode: true });
    await act(async () => { vi.advanceTimersByTime(1000); }); expect(api.page).toHaveBeenCalledTimes(1);
    await render(api); expect(api.page).toHaveBeenCalledTimes(2);
    await render(api, { libraryId: 'new', currentTime: 10 }); expect(api.page).toHaveBeenCalledTimes(2);
    await act(async () => resolve(preparing(80)));
    expect(api.page).toHaveBeenLastCalledWith('new', 10000, null, 120);
    expect(host.textContent).toContain('new record'); expect(host.textContent).not.toContain('80%');
  });
  it('does not expose partial rows or claim 100 percent before the native index is ready', async () => {
    vi.useFakeTimers(); const api = { page: vi.fn().mockResolvedValue({ ...preparing(100), items: [row] }) };
    await render(api); expect(host.textContent).toContain('99%'); expect(host.textContent).not.toContain(row.text);
    expect(host.querySelector('.platform-replay-chat-header')?.querySelectorAll('button')).toHaveLength(1);
  });
  it('requires explicit retry after the initial read fails rather than retrying every video second', async () => {
    const api = { page: vi.fn().mockRejectedValueOnce(new Error('native unavailable')).mockResolvedValue(sample()) };
    await render(api); expect(api.page).toHaveBeenCalledTimes(1); expect(host.querySelector('[role="alert"]')).not.toBeNull();
    await render(api, { currentTime: 20 }); await render(api, { currentTime: 30 }); expect(api.page).toHaveBeenCalledTimes(1);
    await click([...host.querySelectorAll('button')].find(node => node.textContent === '다시 시도')!);
    expect(api.page).toHaveBeenCalledTimes(2); expect(api.page).toHaveBeenLastCalledWith('fixture', 30000, null, 120);
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });
});
