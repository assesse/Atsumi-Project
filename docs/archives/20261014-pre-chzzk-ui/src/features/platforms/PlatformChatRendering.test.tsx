import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformLiveChat } from './PlatformLiveChat';
import { PlatformReplayChat, type ReplayChatPage } from './PlatformReplayChat';
import type { MediaChatPart, PlatformLiveApi } from './platformMedia';
const id = 'a'.repeat(64), dataUrl = 'data:image/png;base64,aGVsbG8=';
const parts: MediaChatPart[] = [{ type: 'text', text: 'before ' }, { type: 'emoji', assetId: id, alt: ':flower:' }, { type: 'text', text: ' after' }];
const row = { author: 'Fixture', text: 'before :flower: after', parts, kind: 'chat', amount: null, badges: [], color: null };
let host: HTMLDivElement, root: Root;
describe('live and archived rich chat integration', () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });
  it('live displays the local scoped image and stops asset reads under privacy', async () => {
    const chat = vi.fn().mockResolvedValue({ items: [{ ...row, sequence: 1 }], cursor: 1, status: '채팅 연결됨', ended: false, gap: false });
    const chatAsset = vi.fn().mockResolvedValue({ state: 'ready', dataUrl });
    const api: PlatformLiveApi = { chat, chatAsset, open: vi.fn(), snapshot: vi.fn(), record: vi.fn(), close: vi.fn() };
    await act(async () => root.render(<PlatformLiveChat token="session" api={api} active privacyMode={false} title="Fixture" />));
    expect(host.querySelector('img')?.src).toBe(dataUrl); expect(chatAsset).toHaveBeenCalledExactlyOnceWith('session', id);
    await act(async () => root.render(<PlatformLiveChat token="session" api={api} active privacyMode title="Fixture" />));
    expect(host.querySelector('img')).toBeNull(); expect(host.textContent).not.toContain('Fixture'); expect(chatAsset).toHaveBeenCalledOnce();
  });
  it('replay connects only to the current owned library asset API', async () => {
    const page: ReplayChatPage = { items: [{ ...row, id: 'event', timeMs: 10 }], state: 'ready', timelineQuality: 'receive-time-approximate', nextAtMs: null, cursor: null, previousCursor: null, total: 1, malformedLines: 0, unsupportedItems: 0, reordered: 0, duplicates: 0, partialFile: false };
    const api = { page: vi.fn().mockResolvedValue(page), asset: vi.fn().mockResolvedValue({ state: 'ready', dataUrl }) };
    await act(async () => root.render(<PlatformReplayChat libraryId="recording" currentTime={1} active privacyMode={false} api={api} />));
    expect(api.asset).toHaveBeenCalledExactlyOnceWith('recording', id); expect(host.querySelector('img')?.src).toBe(dataUrl);
    expect(host.querySelector('.platform-replay-chat-text')?.textContent).toBe('before  after');
  });
  it('legacy live rows render unchanged without starting asset reads', async () => {
    const api: PlatformLiveApi = { open: vi.fn(), snapshot: vi.fn(), record: vi.fn(), close: vi.fn(), chatAsset: vi.fn(), chat: vi.fn().mockResolvedValue({ items: [{ ...row, parts: undefined, sequence: 1 }], cursor: 1, status: '', ended: false, gap: false }) };
    await act(async () => root.render(<PlatformLiveChat token="legacy" api={api} active privacyMode={false} title="Fixture" />));
    expect(host.textContent).toContain(row.text); expect(api.chatAsset).not.toHaveBeenCalled(); expect(host.querySelector('img')).toBeNull();
  });
});
