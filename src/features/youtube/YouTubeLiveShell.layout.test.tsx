import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it, vi } from 'vitest';
import { YouTubeViewingControls } from './YouTubeViewingControls';
import type { YouTubeMetadata } from './model';
import { targetKey } from './model';
import { auditLayouts, layoutBrowser } from '../../test/offlineLayoutBrowser';
import appCss from '../../styles.css?raw';
import shellCss from './YouTubeConnection.css?raw';
import type { YouTubeAutoRecordApi, YouTubeAutoSnapshot } from '../../api/youtubeAutoRecord';

// Test aliases only; production never depends on another provider's selectors.
function geometryAliases(element: HTMLElement) {
  const clone = element.cloneNode(true) as HTMLElement;
  for (const [selector, alias] of [
    ['.youtube-channel-add', 'connection-channel-add'], ['.youtube-channel-clear', 'connection-channel-clear'],
    ['.youtube-channel-field>.youtube-channel-star', 'connection-channel-favorite'], ['.youtube-channel-actions', 'live-channel-actions'],
    ['.youtube-channel-list', 'live-channel-list'], ['.youtube-channel-card', 'live-channel-row'], ['.youtube-channel-copy', 'live-channel-identity'],
    ['.youtube-viewing-mode', 'mado-layout-options'], ['.youtube-session-actions', 'connection-session-actions'],
  ]) for (const item of clone.querySelectorAll(selector!)) item.classList.add(alias!);
  for (const item of clone.querySelectorAll('.youtube-connection-actions')) item.classList.add('connection-actions', 'is-unified');
  return clone.innerHTML;
}
describe.skipIf(!layoutBrowser)('YouTube CHZZK-clone offline geometry', () => {
  it.each([[960, 640], [1440, 900]])('keeps overlay controls centered and four-row favorites at %ix%i', async (width, height) => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
    const container = document.createElement('div'); document.body.append(container); const root = createRoot(container);
    const frames: { name: string; html: string }[] = [];
    try {
      for (const panelWidth of [420, 960]) for (const count of [0, 2, 4, 8, 10]) {
        const favorites: YouTubeMetadata[] = Array.from({ length: count }, (_, index) => ({ kind: 'video', id: `abcdefghij${index}`, url: `https://www.youtube.com/watch?v=abcdefghij${index}`, title: `채널 ${index}`, author: `채널 ${index}` }));
        await act(async () => root.render(<YouTubeViewingControls key={`${panelWidth}-${count}`} active open privacyMode={false} busy={false} items={[]} favorites={favorites} favoriteIds={favorites.map(targetKey)} playingIds={[]} mode="all" lead="" onOpenChange={vi.fn()} onApply={vi.fn()} onStop={vi.fn()} onExplore={vi.fn()} onCloseItem={vi.fn()} onFavorite={vi.fn()} onDownload={vi.fn()} />));
        for (let index = 0; index < 3; index++) await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="방송 추가"]')!.click());
        expect(container.querySelectorAll('.youtube-channel-field')).toHaveLength(4); expect(container.querySelectorAll('.youtube-channel-card')).toHaveLength(count);
        frames.push({ name: `drawer-${panelWidth}-${count}`, html: `<main style="position:relative;width:${Math.min(width, panelWidth)}px;height:${height}px">${geometryAliases(container)}</main>` });
      }
      const statusRows: YouTubeAutoSnapshot['channels'] = Array.from({ length: 10 }, (_, index) => ({
        channelId: `UC${String(index).padStart(22, '0')}`, name: `상태 채널 ${index}`, enabled: true,
        status: (['idle', 'recording', 'stopping', 'failed'] as const)[index % 4]!,
        liveStatus: (['live', 'live', 'offline', 'unknown'] as const)[index % 4]!,
        recordingCount: index % 4 === 1 ? 2 : 0, jobId: null, checkedAt: null, error: null,
      }));
      const favorites: YouTubeMetadata[] = statusRows.map(row => ({ kind: 'channel', id: `channel/${row.channelId}`, url: `https://www.youtube.com/channel/${row.channelId}`, title: row.name, author: '' }));
      const autoApi: YouTubeAutoRecordApi = { snapshot: vi.fn(async () => ({ revision: 1, ready: true, error: null, channels: statusRows })), resolve: vi.fn(), set: vi.fn() };
      for (const panelWidth of [420, 960]) for (const privacy of [false, true]) {
        await act(async () => root.render(<YouTubeViewingControls key={`states-${panelWidth}-${privacy}`} active open privacyMode={privacy} busy={false} items={[]} favorites={favorites} favoriteIds={favorites.map(targetKey)} playingIds={[]} autoApi={autoApi} mode="all" lead="" onOpenChange={vi.fn()} onApply={vi.fn()} onStop={vi.fn()} onExplore={vi.fn()} onCloseItem={vi.fn()} onFavorite={vi.fn()} onDownload={vi.fn()} />));
        expect(container.querySelectorAll('.youtube-channel-card')).toHaveLength(10);
        if (!privacy) { expect(container.querySelector('.is-recording')).not.toBeNull(); expect(container.querySelector('.is-problem')).not.toBeNull(); }
        frames.push({ name: `status-${panelWidth}-${privacy}`, html: `<main style="position:relative;width:${Math.min(width, panelWidth)}px;height:${height}px">${geometryAliases(container)}</main>` });
      }
      const results = await auditLayouts(frames, `${appCss}\n${shellCss}`, width, height, 'youtube-live-shell');
      expect(results).toHaveLength(frames.length); expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
    } finally { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); }
  }, 60000);
});
