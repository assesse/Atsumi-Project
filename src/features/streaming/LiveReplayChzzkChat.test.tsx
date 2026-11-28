import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LiveReplayChzzkItem } from '../../api/cloudLiveReplay';
import { LiveReplayChzzkChat } from './RecordingReplayChat';

const id = 'a'.repeat(64), image = 'data:image/png;base64,aGVsbG8=';
const row = (sequence: number, patch: Partial<LiveReplayChzzkItem> = {}): LiveReplayChzzkItem => ({
  sequence, sender: `기록 사용자 ${sequence}`, text: `기록 메시지 ${sequence}`, serverTime: null, receivedAt: 1800000000000 + sequence,
  offsetSeconds: 0, broadcastOffsetSeconds: null, mediaTimeSeconds: null, syncQuality: 'unknown', ...patch,
});
let host: HTMLDivElement, root: Root;
const surface = () => host.querySelector('.recording-replay-chat')!.shadowRoot!;
const content = () => surface().querySelector('.original-chat-root')!;
beforeEach(() => { vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe('native Live history in original CHZZK chat', () => {
  it('preserves unknown rows without a fake timestamp or seek; known positions can still seek', async () => {
    const onSeek = vi.fn(), asset = vi.fn().mockResolvedValue(null);
    const items = [row(1), row(2, { mediaTimeSeconds: 12.5, syncQuality: 'observed_media' }), row(3, { mediaTimeSeconds: 0, syncQuality: 'unknown' })];
    await act(async () => root.render(<LiveReplayChzzkChat token="live" items={items} asset={asset} onSeek={onSeek} />));
    expect(content().querySelector('#aside-chatting')).not.toBeNull();
    expect(content().querySelector('[role="log"]')).not.toBeNull();
    expect(content().textContent).toContain('채팅');
    for (const sequence of [1, 3]) {
      const element = content().querySelector(`[data-sequence="${sequence}"]`)!;
      expect(element.textContent).toContain(`기록 메시지 ${sequence}`);
      expect(element.querySelector('.recording-replay-chat-time')).toBeNull();
      expect(element.querySelector('.recording-replay-row-seek')).toBeNull();
      expect(element.getAttribute('title')).toContain('영상 위치 확인되지 않음');
      expect(element.getAttribute('title')).not.toContain('동기화 근사');
    }
    const seek = content().querySelector<HTMLButtonElement>('[data-sequence="2"] .recording-replay-row-seek')!;
    expect(seek).toHaveAttribute('aria-label', '00:00:12 영상 위치로 이동');
    await act(async () => seek.click()); expect(onSeek).toHaveBeenCalledExactlyOnceWith(12.5);
    expect(asset).not.toHaveBeenCalled();
  });
  it('retains recorded color, notice, badge and emoji using only the native scoped asset', async () => {
    const asset = vi.fn().mockResolvedValue(image);
    const items = [row(4, { text: '감사해요 {:flower:}', assetIds: { 'https://untrusted.invalid/badge': id, 'https://untrusted.invalid/emoji': id },
      rich: { nicknameColor: '#12abef', textColor: '#aabbcc', badges: [{ kind: 'subscription', title: '구독 3개월', imageUrl: 'https://untrusted.invalid/badge' }],
        emojis: [{ id: 'flower', imageUrl: 'https://untrusted.invalid/emoji' }], notice: { kind: 'donation', amount: 100 } } })];
    await act(async () => root.render(<LiveReplayChzzkChat token="owned-live" items={items} asset={asset} />));
    expect(asset).toHaveBeenCalledExactlyOnceWith('owned-live', id);
    expect(content().textContent).toContain('100 치즈');
    expect(content().querySelector<HTMLElement>('[data-sequence="4"]')!.style.getPropertyValue('--replay-text-color')).toBe('#aabbcc');
    expect([...content().querySelectorAll<HTMLElement>('[style]')].some(element => element.style.color === 'rgb(18, 171, 239)')).toBe(true);
    const images = [...content().querySelectorAll('img')]; expect(images.length).toBeGreaterThan(0);
    for (const element of images) { expect(element.src).toBe(image); expect(element.getAttribute('referrerpolicy') ?? element.referrerPolicy).toBe('no-referrer'); }
    expect(content().querySelector('.recording-replay-row-seek')).toBeNull();
  });
  it.each([{ active: false, privacyMode: false }, { active: true, privacyMode: true }])('hides recorded content and suppresses asset reads when %j', async flags => {
    const asset = vi.fn().mockResolvedValue(image); const items = [row(1, { assetIds: { old: id } })];
    await act(async () => root.render(<LiveReplayChzzkChat token="live" items={items} asset={asset} {...flags} />));
    expect(content().textContent).not.toContain('기록 메시지'); expect(content().querySelector('img')).toBeNull(); expect(asset).not.toHaveBeenCalled();
  });
  it('drops late assets from the previous native token instead of reusing them for a new receipt', async () => {
    let resolve!: (value: string) => void;
    const asset = vi.fn().mockReturnValueOnce(new Promise(value => { resolve = value; })).mockResolvedValue(null);
    const items = [row(1, { assetIds: { badge: id }, rich: { badges: [{ kind: 'profile', imageUrl: 'badge' }], emojis: [] } })];
    await act(async () => root.render(<LiveReplayChzzkChat token="old-live" items={items} asset={asset} />));
    await act(async () => root.render(<LiveReplayChzzkChat token="new-live" items={items} asset={asset} />));
    await act(async () => resolve(image));
    expect(asset.mock.calls).toEqual([['old-live', id], ['new-live', id]]);
    expect(content().querySelector('img')).toBeNull();
  });
});
