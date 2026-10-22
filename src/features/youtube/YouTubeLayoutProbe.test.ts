// @ts-expect-error Node-only diagnostics fixture, never imported by production.
import { runInNewContext } from 'node:vm';
import probe from '../../../src-tauri/examples/youtube_layout_probe.js?raw';
import { expect, it } from 'vitest';

function inspect(options: { origin?: string; pathname?: string; noSheet?: boolean; ancestry?: number; dark?: boolean; color?: string; background?: string; tokens?: Record<string, string>; messages?: number } = {}) {
  const style = { position: 'fixed', display: 'block', visibility: 'visible', opacity: '1', transform: 'matrix(1, 0, 0, 1, 181, 0)', filter: 'none', backdropFilter: 'none', perspective: 'none', contain: 'layout paint', willChange: 'transform, opacity', contentVisibility: 'visible', color: options.color ?? 'rgb(255, 255, 255)', backgroundColor: options.background ?? 'rgb(255, 255, 255)', zIndex: '2147483000', getPropertyValue: (name: string) => name.startsWith('--atsumi-') ? '34px' : options.tokens?.[name] ?? '' };
  const element = { tagName: 'DIV', id: 'private-dynamic-identifier', getBoundingClientRect: () => ({ x: 215, y: 176, width: 951, height: 539 }), hasAttribute: (name: string) => name === 'dark' && options.dark === true, parentElement: null as unknown, querySelector: () => video };
  const video = { ...element, currentTime: 42.25, readyState: 4, getVideoPlaybackQuality: () => ({ totalVideoFrames: 240 }) };
  const player = { ...element, id: 'movie_player', parentElement: null as unknown };
  let parent: unknown = null;
  for (let i = 0; i < (options.ancestry ?? 4); i++) parent = { ...element, parentElement: parent };
  player.parentElement = parent;
  const sheet = options.noSheet ? null : { cssRules: { length: 12 } };
  const document = { documentElement: element, body: element, cookie: 'never-read-secret', getElementById: (id: string) => id === 'movie_player' ? player : { sheet }, querySelector: () => element, querySelectorAll: (selector: string) => selector.includes('#message') ? Array.from({ length: options.messages ?? 1 }, () => element) : ({ length: 9999 }) };
  return runInNewContext(probe, { window: { __atsumiYouTubeOriginal: {}, visualViewport: { width: 1266, height: 782, scale: 1 }, matchMedia: () => ({ matches: true }) }, document, location: { origin: options.origin ?? 'https://www.youtube.com', pathname: options.pathname ?? '/watch' }, innerWidth: 1266, innerHeight: 782, devicePixelRatio: 1, getComputedStyle: () => style });
}
it('projects only bounded geometry and fixed enums, never dynamic document identifiers or content', () => {
  const result = inspect({ ancestry: 40 });
  expect(result).toMatchObject({ supported: true, bridge: true, stylePresent: true, sheetPresent: true, sheetRules: 12, embeddedChatCount: 8, ancestorTruncated: true });
  expect(result.ancestors).toHaveLength(12);
  expect(result.player).toMatchObject({ rect: { x: 215, y: 176, width: 951, height: 539 }, transform: [1, 0, 0, 1, 181, 0], containFlags: 3, willChangeFlags: 17 });
  expect(result.ancestors[0].id).toBe('other');
  expect(JSON.stringify(result)).not.toMatch(/private-dynamic|never-read-secret/);
});
it('distinguishes an existing style element from an unapplied or unavailable stylesheet', () => {
  expect(inspect({ noSheet: true })).toMatchObject({ stylePresent: true, sheetPresent: false, sheetRules: null });
});
it('does not inspect foreign origins or account paths and never performs network or page mutations', () => {
  expect(inspect({ origin: 'https://accounts.google.com' })).toEqual({ supported: false });
  expect(inspect({ pathname: '/signin' })).toEqual({ supported: false });
  expect(inspect({ pathname: '/live_chat' }).documentKind).toBe('chat');
  for (const forbidden of ['fetch(', 'XMLHttpRequest', 'document.cookie', 'localStorage', 'setProperty(', '.appendChild(', '.play(', '.pause(', 'textContent', 'innerHTML']) expect(probe).not.toContain(forbidden);
});
it('measures a dark legacy text and light token background mismatch without projecting content', () => {
  const result = inspect({ pathname: '/live_chat', dark: true, messages: 20, tokens: {
    '--t3e41d7b17b187f69': '#fff', '--yt-sys-color-baseline--base-background': '#0f0f0f',
    '--yt-sys-color-baseline--text-primary': '#f1f1f1', '--yt-live-chat-primary-text-color': '#fff',
  } });
  expect(result.rootDark).toBe(true);
  expect(result.chatTheme).toMatchObject({ systemDark: true, tokens: { newBackground: [255, 255, 255, 1000], baselineBackground: [15, 15, 15, 1000], baselineText: [241, 241, 241, 1000], primaryText: [255, 255, 255, 1000] }, renderer: { foregroundRgba: [255, 255, 255, 1000], backgroundRgba: [255, 255, 255, 1000] } });
  expect(result.chatTheme.messages).toHaveLength(3);
  expect(JSON.stringify(result)).not.toMatch(/private-dynamic|never-read-secret|#fff|rgb\(/);
  expect(inspect().chatTheme).toBeNull();
});
it('bounds colors and returns null for unknown or out-of-range CSS instead of copying strings', () => {
  expect(inspect({ color: 'rgba(1, 2, 3, 0.5)', background: 'transparent' }).root).toMatchObject({ foregroundRgba: [1, 2, 3, 500], backgroundRgba: [0, 0, 0, 0] });
  expect(inspect({ color: '#1234', background: '#01020380' }).root).toMatchObject({ foregroundRgba: [17, 34, 51, 267], backgroundRgba: [1, 2, 3, 502] });
  for (const value of ['rgb(999, 2, 3)', 'rgba(1, 2, 3, 4)', 'url(private-secret)', 'color(display-p3 1 0 0)', 'var(--private-value)']) {
    const result = inspect({ color: value, background: value });
    expect(result.root).toMatchObject({ foregroundRgba: null, backgroundRgba: null });
    expect(JSON.stringify(result)).not.toContain(value);
  }
});
