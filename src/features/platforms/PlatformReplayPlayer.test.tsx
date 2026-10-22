import { act, createRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayPlayer, replayTime, type PlatformReplayPlayerProps } from './PlatformReplayPlayer';
import playerCss from './PlatformReplayPlayer.css?raw';

let host: HTMLDivElement, root: Root, props: PlatformReplayPlayerProps;
const button = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
const click = async (node: Element) => act(async () => { node.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
const render = async (change: Partial<PlatformReplayPlayerProps> = {}) => act(async () => { props = { ...props, ...change }; root.render(<div className="platform-media-player show-controls"><PlatformReplayPlayer {...props} /></div>); });
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  props = { provider: 'youtube', url: 'http://platform-media.localhost/token/media.mp4', videoRef: createRef<HTMLVideoElement>(), mediaEvents: {},
    title: '저장 영상', active: true, privacyMode: false, paused: false, ended: false, position: 10, duration: 60, volume: .5, muted: false, busy: false, error: '',
    onShowControls: vi.fn(), onLeaveControls: vi.fn(), onToggle: vi.fn(), onReplay: vi.fn(), onSeek: vi.fn(), onVolume: vi.fn(), onMute: vi.fn(), onFullscreen: vi.fn(), onRetry: vi.fn(), onClose: vi.fn() };
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });
describe('local provider replay presentation', () => {
  it('declares enough border-box footer height for both rows without clipping the lower controls', () => {
    // jsdom has no layout: this checks the actual CSS declarations, not pixels.
    const rule = (selector: string) => playerCss.split(`${selector}{`)[1]?.split('}')[0] ?? '';
    const pixels = (body: string, property: string) => Number(body.match(new RegExp(`(?:^|;)${property}:(\\d+)px(?:;|$)`))?.[1]);
    const footer = rule('.platform-media-player .platform-replay-player footer.platform-replay-controls');
    const padding = footer.match(/(?:^|;)padding:(\d+)px \d+px (\d+)(?:px)?;/);
    expect(footer).toContain('box-sizing:border-box'); expect(padding).not.toBeNull();
    const occupied = Number(padding?.[1]) + Number(padding?.[2])
      + pixels(rule('.platform-replay-controls .platform-replay-progress'), 'height')
      + pixels(rule('.platform-media-player .platform-replay-player footer>.platform-replay-control-row'), 'height');
    expect(Number.isFinite(occupied)).toBe(true);
    expect(pixels(footer, 'height')).toBeGreaterThanOrEqual(occupied);
    expect(pixels(rule('.platform-replay-settings'), 'bottom')).toBeGreaterThanOrEqual(pixels(footer, 'height'));
  });
  it.each(['youtube', 'soop'] as const)('uses provider-scoped vector controls and only the owned source for %s', async provider => {
    await render({ provider }); expect(host.querySelector('.platform-replay-player')).toHaveClass(`is-${provider}`);
    expect(host.querySelectorAll('video')).toHaveLength(1); expect(host.querySelector('video')).toHaveAttribute('src', props.url);
    expect(host.querySelectorAll('iframe,img,script,link')).toHaveLength(0); expect(host.textContent).not.toMatch(/[Ⅱ▶⛶]/);
    for (const label of ['일시정지', '음소거', '재생 설정', '전체화면']) expect(button(label).querySelector('svg')).not.toBeNull();
    expect(host.querySelector('.platform-replay-control-row')?.lastElementChild).toBe(button('전체화면'));
    await click(button('일시정지')); expect(props.onToggle).toHaveBeenCalledOnce();
    await render({ paused: true }); expect(button('재생').querySelector('svg')).not.toBeNull();
  });
  it('preserves the same media node, URL, time and paused state while choosing local speed', async () => {
    await render(); const video = host.querySelector('video')!; video.currentTime = 17;
    await click(button('재생 설정')); expect(host.querySelector('[role="menu"]')).toHaveAttribute('aria-label', '재생 속도');
    await click([...host.querySelectorAll('[role="menuitemradio"]')].find(node => node.textContent === '1.5배')!);
    expect(video.playbackRate).toBe(1.5); expect(video.currentTime).toBe(17); expect(host.querySelector('video')).toBe(video);
    expect(props.onToggle).not.toHaveBeenCalled(); expect(host.querySelector('[role="menu"]')).toBeNull(); expect(document.activeElement).toBe(button('재생 설정'));
  });
  it('reads the actual playback rate event instead of inventing a selected setting', async () => {
    await render(); const video = host.querySelector('video')!;
    await act(async () => { video.playbackRate = 2; video.dispatchEvent(new Event('ratechange')); });
    await click(button('재생 설정')); expect([...host.querySelectorAll('[role="menuitemradio"]')].find(node => node.getAttribute('aria-checked') === 'true')).toHaveTextContent('2배');
  });
  it('settings Escape is consumed without closing replay and outside pointer closes only settings', async () => {
    await render(); await click(button('재생 설정'));
    const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    await act(async () => document.dispatchEvent(escape)); expect(escape.defaultPrevented).toBe(true); expect(props.onClose).not.toHaveBeenCalled();
    expect(host.querySelector('[role="menu"]')).toBeNull(); await click(button('재생 설정'));
    await act(async () => document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }))); expect(host.querySelector('[role="menu"]')).toBeNull();
  });
  it('does not capture composing Escape and releases document listeners on unmount', async () => {
    await render(); await click(button('재생 설정'));
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', isComposing: true, bubbles: true }))); expect(host.querySelector('[role="menu"]')).not.toBeNull();
    await act(async () => root.render(null)); const event = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }); document.dispatchEvent(event); expect(event.defaultPrevented).toBe(false);
  });
  it('hides settings and disables media actions in privacy while keeping close available', async () => {
    await render(); await click(button('재생 설정')); const video = host.querySelector('video'); await render({ privacyMode: true, title: '프라이버시 모드' });
    expect(host.querySelector('[role="menu"]')).toBeNull(); expect(host.querySelector('video')).toBe(video); expect(video).not.toHaveAttribute('autoplay');
    await click(button('일시정지')); await click(button('재생 설정')); expect(props.onToggle).not.toHaveBeenCalled(); expect(host.querySelector('[role="menu"]')).toBeNull();
    expect(button('재생 닫기')).not.toBeDisabled(); await click(button('재생 닫기')); expect(props.onClose).toHaveBeenCalledOnce();
  });
  it('keeps hidden playback mounted without media commands and closes its transient settings', async () => {
    await render(); const video = host.querySelector('video'); await click(button('재생 설정')); await render({ active: false });
    expect(host.querySelector('video')).toBe(video); expect(host.querySelector('[role="menu"]')).toBeNull(); expect(button('일시정지')).toBeDisabled(); expect(props.onToggle).not.toHaveBeenCalled();
  });
  it('keeps actual EOF replay, warning and retry error surfaces', async () => {
    await render({ ended: true, title: <>저장 영상 <span>채팅 일부 누락</span></> });
    expect(host.querySelector('header')).toHaveTextContent('채팅 일부 누락');
    await click(host.querySelector('.platform-media-replay')!); expect(props.onReplay).toHaveBeenCalledOnce();
    await click(button('재생')); expect(props.onReplay).toHaveBeenCalledTimes(2);
    await render({ error: '저장 영상을 재생하지 못했습니다.' }); expect(host.querySelector('[role="alert"]')).toHaveTextContent('저장 영상을 재생하지 못했습니다.');
    await click(host.querySelector('[role="alert"] button')!); expect(props.onRetry).toHaveBeenCalledOnce();
  });
  it('delegates fullscreen once and observes true ownership for its exit icon', async () => {
    await render(); await click(button('전체화면')); expect(props.onFullscreen).toHaveBeenCalledOnce();
    const descriptor = Object.getOwnPropertyDescriptor(document, 'fullscreenElement'); let owner: Element | null = host.firstElementChild;
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => owner });
    try {
      await act(async () => document.dispatchEvent(new Event('fullscreenchange'))); expect(button('전체화면 종료')).not.toBeNull();
      owner = document.body; await act(async () => document.dispatchEvent(new Event('fullscreenchange'))); expect(button('전체화면')).not.toBeNull();
    } finally { if (descriptor) Object.defineProperty(document, 'fullscreenElement', descriptor); else Reflect.deleteProperty(document, 'fullscreenElement'); }
  });
  it('bounds progress display and supports hours without a new media clock', async () => {
    await render({ position: Infinity, duration: NaN }); const progress = host.querySelector<HTMLInputElement>('[aria-label="재생 위치"]')!;
    expect(progress.value).toBe('0'); expect(progress).toBeDisabled(); expect(progress.getAttribute('aria-valuetext')).toBe('0:00 / 0:00');
    expect(replayTime(3661)).toBe('1:01:01'); expect(replayTime(-1)).toBe('0:00');
  });
});
