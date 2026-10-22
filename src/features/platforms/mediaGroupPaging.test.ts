import { beforeEach, describe, expect, it, vi } from 'vitest';
import { emptyMediaGroups, groupPanelPosition, mediaGroupStorageKey, readMediaGroups, readMediaMembers, readOpenMediaGroup, saveOpenMediaGroup } from './mediaGroupPaging';
import type { MediaGroupsPage, MediaGroupMembersPage, MediaLibraryItem, MediaRecordingGroup } from './platformMedia';

const item = (id: string): MediaLibraryItem => ({ libraryId: id, revision: 1, reference: { provider: 'youtube', kind: 'live', externalId: 'abcdefghijk' }, title: id, channelName: '채널', bytes: 10, duration: 1, sha256: 'a'.repeat(64), completedAt: 1 });
const group = (count = 135, revision = 'members-1'): MediaRecordingGroup => ({ type: 'recordingGroup', key: 'broadcast-a', groupRevision: revision, representative: item('0'), fileCount: count, totalBytes: count * 10, latestCompletedAt: 1 });
const summary = (revision = 'catalog-1', cursor: string | null = null, key = 'broadcast-a'): MediaGroupsPage => ({ status: 'ready', catalogRevision: revision, cursor, entries: [{ ...group(), key }] });
const yes = () => true;
beforeEach(() => { localStorage.removeItem(mediaGroupStorageKey('youtube')); localStorage.removeItem(mediaGroupStorageKey('soop')); });

describe('revision-bound catalog paging', () => {
  it('keeps a135-file broadcast as one complete summary', async () => {
    const read = vi.fn().mockResolvedValue(summary());
    const result = await readMediaGroups(read, 'youtube', emptyMediaGroups(), 1, yes);
    expect(result.status).toBe('ready');
    if (result.status !== 'ready') throw new Error('Expected ready');
    expect(result.snapshot.entries).toHaveLength(1); expect(result.snapshot.entries[0]).toMatchObject({ fileCount: 135, totalBytes: 1350 });
  });
  it('stages a changed two-page prefix atomically and removes stale tail entries', async () => {
    const previous = { revision: 'old', entries: [{ ...group(), key: 'deleted-tail' }], cursor: 'old-tail', pages: 2 };
    let complete!: (page: MediaGroupsPage) => void;
    const read = vi.fn().mockResolvedValueOnce(summary('new', 'page-2')).mockReturnValueOnce(new Promise(resolve => { complete = resolve; }));
    const promise = readMediaGroups(read, 'youtube', previous, 2, yes);
    await Promise.resolve();
    expect(previous.entries[0]?.key).toBe('deleted-tail');
    complete(summary('new', 'page-3', 'new-tail'));
    const result = await promise;
    expect(result).toMatchObject({ status: 'ready', snapshot: { revision: 'new', pages: 2, cursor: 'page-3' } });
    if (result.status === 'ready') expect(result.snapshot.entries.map(value => value.key)).toEqual(['broadcast-a', 'new-tail']);
  });
  it('skips older summary pages on an unchanged revision, but reloads them for more pages', async () => {
    const previous = { revision: 'catalog-1', entries: [group()], cursor: 'page-2', pages: 2 };
    const read = vi.fn().mockResolvedValue(summary('catalog-1', 'next'));
    expect(await readMediaGroups(read, 'youtube', previous, 2, yes)).toEqual({ status: 'ready', snapshot: previous });
    expect(read).toHaveBeenCalledTimes(1);
  });
  it('returns changed once on a second revision without recursively retrying', async () => {
    const read = vi.fn().mockResolvedValueOnce(summary('one', 'p2')).mockResolvedValueOnce(summary('two', null, 'b'));
    expect(await readMediaGroups(read, 'youtube', emptyMediaGroups(), 2, yes)).toEqual({ status: 'changed' });
    expect(read).toHaveBeenCalledTimes(2);
  });
  it('preserves typed changed and rejects duplicated keys and cursor cycles', async () => {
    expect(await readMediaGroups(vi.fn().mockResolvedValue({ status: 'changed', catalogRevision: 'two' }), 'youtube', emptyMediaGroups(), 2, yes)).toEqual({ status: 'changed' });
    await expect(readMediaGroups(vi.fn().mockResolvedValue(summary('one', 'p2')), 'youtube', emptyMediaGroups(), 2, yes)).rejects.toThrow('목록이 변경');
    await expect(readMediaGroups(vi.fn().mockResolvedValueOnce(summary('one', 'p2')).mockResolvedValueOnce(summary('one', 'p2', 'b')), 'youtube', emptyMediaGroups(), 3, yes)).rejects.toThrow('목록 위치');
  });
  it('rejects late lifetime responses without publishing or requesting another page', async () => {
    let current = true;
    const read = vi.fn().mockImplementation(async () => { current = false; return summary('one', 'p2'); });
    expect(await readMediaGroups(read, 'youtube', emptyMediaGroups(), 2, () => current)).toEqual({ status: 'cancelled' });
    expect(read).toHaveBeenCalledTimes(1);
  });
});
describe('coherent member pages', () => {
  it('exposes all135 original files without duplicates over two bounded pages', async () => {
    const all = Array.from({ length: 135 }, (_, index) => item(String(index)));
    const read = vi.fn().mockResolvedValueOnce({ status: 'ready', group: group(), items: all.slice(0, 100), cursor: 'p2' }).mockResolvedValueOnce({ status: 'ready', group: group(), items: all.slice(100), cursor: null });
    const result = await readMediaMembers(read, 'youtube', 'broadcast-a', 2, yes);
    expect(result).toMatchObject({ status: 'ready', snapshot: { group: { fileCount: 135 }, cursor: null, pages: 2 } });
    if (result.status === 'ready') expect(result.snapshot.items).toEqual(all);
    expect(read.mock.calls).toEqual([['youtube', 'broadcast-a', undefined], ['youtube', 'broadcast-a', 'p2']]);
  });
  it('uses the first current member summary rather than an older catalog count', async () => {
    const read = vi.fn().mockResolvedValue({ status: 'ready', group: group(1, 'new'), items: [item('new')], cursor: null });
    const result = await readMediaMembers(read, 'youtube', 'broadcast-a', 2, yes);
    expect(result).toMatchObject({ status: 'ready', snapshot: { group: { fileCount: 1, groupRevision: 'new' }, pages: 1 } });
  });
  it('rejects mixed member revisions before making the staged result visible', async () => {
    const read = vi.fn().mockResolvedValueOnce({ status: 'ready', group: group(2), items: [item('a')], cursor: 'p2' }).mockResolvedValueOnce({ status: 'ready', group: group(1, 'new'), items: [item('b')], cursor: null });
    expect(await readMediaMembers(read, 'youtube', 'broadcast-a', 2, yes)).toEqual({ status: 'changed' });
  });
  it('reads per-file warnings again even when membership is unchanged', async () => {
    const saved = item('a');
    const read = vi.fn().mockResolvedValueOnce({ status: 'ready', group: group(1), items: [saved], cursor: null }).mockResolvedValueOnce({ status: 'ready', group: group(1), items: [{ ...saved, chatArchive: { state: 'partial', reasons: ['archive_limit'], originalsRetained: true, assetsIncomplete: false } }], cursor: null });
    await readMediaMembers(read, 'youtube', 'broadcast-a', 1, yes);
    expect(await readMediaMembers(read, 'youtube', 'broadcast-a', 1, yes)).toMatchObject({ status: 'ready', snapshot: { items: [{ chatArchive: { state: 'partial' } }] } });
    expect(read).toHaveBeenCalledTimes(2);
  });
  it.each([{ status: 'missing' }, { status: 'changed', groupRevision: 'new' }] as MediaGroupMembersPage[])('retains the typed missing/changed outcome: $status', async response => {
    expect(await readMediaMembers(vi.fn().mockResolvedValue(response), 'youtube', 'broadcast-a', 1, yes)).toEqual({ status: response.status });
  });
  it('rejects wrong keys, duplicates and false completed counts', async () => {
    for (const page of [
      { status: 'ready', group: { ...group(1), key: 'other' }, items: [item('a')], cursor: null },
      { status: 'ready', group: group(2), items: [item('a'), item('a')], cursor: null },
      { status: 'ready', group: group(2), items: [item('a')], cursor: null },
    ]) await expect(readMediaMembers(vi.fn().mockResolvedValue(page), 'youtube', 'broadcast-a', 1, yes)).rejects.toThrow();
  });
});
describe('presentation preferences and placement', () => {
  it('persists exactly one provider-scoped key or explicit closed state', () => {
    saveOpenMediaGroup('youtube', 'unloaded-a'); saveOpenMediaGroup('soop', 'other');
    expect(readOpenMediaGroup('youtube')).toBe('unloaded-a');
    saveOpenMediaGroup('youtube', 'b'); expect(readOpenMediaGroup('youtube')).toBe('b');
    saveOpenMediaGroup('youtube', null); expect(readOpenMediaGroup('youtube')).toBeNull();
    expect(JSON.parse(localStorage.getItem(mediaGroupStorageKey('youtube'))!)).toEqual({ version: 1, openKey: null });
    expect(readOpenMediaGroup('soop')).toBe('other');
  });
  it('ignores malformed or incompatible preferences', () => {
    for (const value of ['{', '[]', '{"version":2,"openKey":"a"}', JSON.stringify({ version: 1, openKey: 'a'.repeat(513) })]) {
      localStorage.setItem(mediaGroupStorageKey('youtube'), value); expect(readOpenMediaGroup('youtube')).toBeNull();
    }
  });
  it.each([[320, 640], [640, 640], [960, 800]])('keeps fixed overlay bounds in a %i×%i viewport', (width, height) => {
    for (const top of [10, height - 200]) {
      const result = groupPanelPosition({ left: width - 260, top, bottom: top + 160, width: 245 }, width, height);
      expect(result.left).toBeGreaterThanOrEqual(12); expect(result.top).toBeGreaterThanOrEqual(12);
      expect(result.left + result.width).toBeLessThanOrEqual(width - 12);
      expect(result.top + result.maxHeight).toBeLessThanOrEqual(height - 12);
      expect(result.maxHeight).toBeGreaterThan(100);
    }
  });
  it('aligns the expanded group with its card and overlaps its lower edge instead of pushing the grid', () => {
    const anchor = { left: 80, top: 100, bottom: 260, width: 312 };
    expect(groupPanelPosition(anchor, 1200, 1000)).toMatchObject({ left: 80, top: 256, width: 312 });
    expect(groupPanelPosition({ ...anchor, width: 600 }, 1200, 1000).width).toBe(420);
    expect(groupPanelPosition({ ...anchor, width: 200 }, 1200, 1000).width).toBe(250);
  });
});
