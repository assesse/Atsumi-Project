import { afterEach, expect, it } from 'vitest';
import { SOOP_FAVORITES_KEY, readSoopFavorites, writeSoopFavorites } from '../soop/soopFavorites';
import { YOUTUBE_FAVORITES_KEY, readYouTubeFavorites, saveYouTubeFavorites, type YouTubeFavorite } from '../youtube/model';
import { mediaGroupStorageKey, readOpenMediaGroup, saveOpenMediaGroup } from './mediaGroupPaging';
import liveSource from './PlatformLivePlayer.tsx?raw';
import liveCss from './PlatformLivePlayer.css?raw';
import shellSource from './PlatformWorkspace.tsx?raw';

afterEach(() => localStorage.clear());

it('keeps channel and video favorites independent even when identifiers coincide', () => {
  const sameId = 'abcdefghijk';
  const video: YouTubeFavorite = { kind: 'video', id: sameId, url: `https://www.youtube.com/watch?v=${sameId}`, title: 'YouTube video', author: 'YouTube creator', savedAt: 1 };
  expect(writeSoopFavorites([{ channelId: sameId, name: 'SOOP channel' }])).toBe(true);
  const soopBefore = localStorage.getItem(SOOP_FAVORITES_KEY);
  saveYouTubeFavorites(localStorage, [video]);
  expect(localStorage.getItem(SOOP_FAVORITES_KEY)).toBe(soopBefore);
  const youtubeBefore = localStorage.getItem(YOUTUBE_FAVORITES_KEY);
  expect(writeSoopFavorites([])).toBe(true);
  expect(localStorage.getItem(YOUTUBE_FAVORITES_KEY)).toBe(youtubeBefore);
  expect(readSoopFavorites()).toEqual([]);
  expect(readYouTubeFavorites(localStorage)).toMatchObject([video]);
});

it('keeps each recording group expansion and all-closed choice local to its provider', () => {
  expect(mediaGroupStorageKey('soop')).not.toBe(mediaGroupStorageKey('youtube'));
  saveOpenMediaGroup('soop', 'same-group');
  saveOpenMediaGroup('youtube', 'same-group');
  saveOpenMediaGroup('soop', null);
  expect(readOpenMediaGroup('soop')).toBeNull();
  expect(readOpenMediaGroup('youtube')).toBe('same-group');
});

it('loads shared playback indicators without importing either provider stylesheet', () => {
  expect(liveSource).not.toContain('youtube-spinner');
  expect(liveSource).not.toContain('soop-spinner');
  expect(liveSource).toContain('platform-live-spinner');
  expect(liveCss).toContain('.platform-live-spinner');
  expect(liveCss).toContain('@keyframes platform-live-spin');
});

it('keeps the copied shell presentation separate from CHZZK service controllers', () => {
  expect(shellSource).toContain('LiveEdgePanel');
  expect(shellSource).not.toMatch(/import.*(?:MadoWorkspace|StreamingWorkspace|officialBrowser|multiview|liveChannels|autoRecording)/);
  expect(shellSource).toContain('hidden={!active}');
  expect(shellSource).toContain('inert={!active}');
});
