import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { YouTubePlayer } from './YouTubePlayer';
import { parseYouTubeTarget, type YouTubeMetadata } from './model';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
const item: YouTubeMetadata = { ...parseYouTubeTarget('M7lc1UVf-VE')!, title: 'Public sample', author: 'Google for Developers' };
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  vi.useFakeTimers(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});
async function render() { await act(async () => root.render(<YouTubePlayer item={item} privacyMode={false} onExternal={vi.fn()} />)); }
async function message(frame: HTMLIFrameElement, data: unknown, origin = 'https://www.youtube.com') {
  await act(async () => window.dispatchEvent(new MessageEvent('message', { origin, source: frame.contentWindow, data })));
}

describe('YouTube original iframe connection state', () => {
  it('keeps the official player and loading state until its verified readiness message', async () => {
    await render(); const frame = container.querySelector('iframe')!;
    expect(new URL(frame.src).origin).toBe('https://www.youtube.com');
    expect(new URL(frame.src).pathname).toBe('/embed/M7lc1UVf-VE');
    expect(container.querySelector('[role="status"]')).not.toBeNull();
    await message(frame, { event: 'initialDelivery' }, 'https://untrusted.example');
    expect(container.querySelector('[role="status"]')).not.toBeNull();
    await message(frame, { event: 'initialDelivery' });
    expect(container.querySelector('[role="status"]')).toBeNull();
    expect(container.querySelector('iframe')).toBe(frame);
  });
  it('does not let late readiness hide a verified playback failure', async () => {
    await render(); const frame = container.querySelector('iframe')!;
    await message(frame, { event: 'onError', info: 153 });
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('앱의 재생 요청');
    await message(frame, { event: 'initialDelivery' });
    await message(frame, { event: 'onReady' });
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('앱의 재생 요청');
  });
  it('ends the handshake after its bounded timeout and exposes retry inside the player', async () => {
    await render(); const frame = container.querySelector('iframe')!, sent = vi.spyOn(frame.contentWindow!, 'postMessage');
    await act(async () => vi.advanceTimersByTime(20_000));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('재생 응답이 없습니다');
    const count = sent.mock.calls.length;
    await act(async () => vi.advanceTimersByTime(10_000));
    expect(sent.mock.calls).toHaveLength(count);
    await message(frame, { event: 'onReady' });
    expect(container.querySelector('[role="alert"]')).not.toBeNull();
  });
  it('retries only this iframe and ignores messages from the retired frame', async () => {
    await render(); const previous = container.querySelector('iframe')!, previousWindow = previous.contentWindow;
    await message(previous, { event: 'onError', info: 100 });
    const retry = [...container.querySelectorAll('button')].find(button => button.textContent === '다시 시도')!;
    await act(async () => retry.click());
    const frame = container.querySelector('iframe')!;
    expect(frame).not.toBe(previous); expect(container.querySelector('[role="status"]')).not.toBeNull();
    await act(async () => window.dispatchEvent(new MessageEvent('message', { origin: 'https://www.youtube.com', source: previousWindow, data: { event: 'onReady' } })));
    expect(container.querySelector('[role="status"]')).not.toBeNull();
    await message(frame, { event: 'onReady' });
    expect(container.querySelector('[role="status"]')).toBeNull();
  });
});
