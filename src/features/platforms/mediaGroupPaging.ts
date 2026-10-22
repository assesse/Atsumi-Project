import type { MediaLibraryEntry, MediaLibraryItem, MediaProvider, MediaRecordingGroup, PlatformMediaApi } from './platformMedia';

export interface MediaGroupSnapshot { revision: string; entries: MediaLibraryEntry[]; cursor: string | null; pages: number }
export interface MediaMemberSnapshot { group: MediaRecordingGroup; items: MediaLibraryItem[]; cursor: string | null; pages: number }
export const emptyMediaGroups = (): MediaGroupSnapshot => ({ revision: '', entries: [], cursor: null, pages: 1 });
type ReadResult<T> = { status: 'ready'; snapshot: T } | { status: 'changed' | 'cancelled' };

/** Stage the entire visible prefix before publishing it. Never mix catalog revisions. */
export async function readMediaGroups(read: NonNullable<PlatformMediaApi['groups']>, provider: MediaProvider,
  previous: MediaGroupSnapshot, requestedPages: number, current: () => boolean, force = false): Promise<ReadResult<MediaGroupSnapshot>> {
  const entries: MediaLibraryEntry[] = [], keys = new Set<string>(), cursors = new Set<string>();
  let revision = '', cursor: string | undefined, pages = 0;
  while (pages < Math.max(1, requestedPages)) {
    if (!current()) return { status: 'cancelled' };
    const page = await read(provider, cursor);
    if (!current()) return { status: 'cancelled' };
    if (page.status === 'changed' || pages > 0 && page.catalogRevision !== revision) return { status: 'changed' };
    if (!pages && !force && previous.revision && page.catalogRevision === previous.revision && requestedPages <= previous.pages)
      return { status: 'ready', snapshot: previous };
    revision = page.catalogRevision;
    for (const entry of page.entries) {
      if (keys.has(entry.key)) throw new Error('보관함 목록이 변경됐습니다. 다시 확인해 주세요.');
      keys.add(entry.key); entries.push(entry);
    }
    pages++;
    if (!page.cursor) return { status: 'ready', snapshot: { revision, entries, cursor: null, pages } };
    if (cursors.has(page.cursor)) throw new Error('보관함 목록 위치를 확인하지 못했습니다.');
    cursors.add(page.cursor); cursor = page.cursor;
  }
  return { status: 'ready', snapshot: { revision, entries, cursor: cursor ?? null, pages } };
}

/** Read statuses even if membership is unchanged: chat sidecars have their own lifetime. */
export async function readMediaMembers(read: NonNullable<PlatformMediaApi['groupMembers']>, provider: MediaProvider,
  key: string, requestedPages: number, current: () => boolean): Promise<ReadResult<MediaMemberSnapshot> | { status: 'missing' }> {
  const items: MediaLibraryItem[] = [], ids = new Set<string>(), cursors = new Set<string>();
  let group: MediaRecordingGroup | null = null, cursor: string | undefined, pages = 0;
  while (pages < Math.max(1, requestedPages)) {
    if (!current()) return { status: 'cancelled' };
    const page = await read(provider, key, cursor);
    if (!current()) return { status: 'cancelled' };
    if (page.status === 'missing') return { status: 'missing' };
    if (page.status === 'changed') return { status: 'changed' };
    if (page.group.key !== key) throw new Error('녹화 묶음을 확인하지 못했습니다.');
    if (group && (group.groupRevision !== page.group.groupRevision || group.fileCount !== page.group.fileCount || group.totalBytes !== page.group.totalBytes)) return { status: 'changed' };
    group ??= page.group;
    for (const item of page.items) {
      if (ids.has(item.libraryId)) throw new Error('녹화 파일 목록이 변경됐습니다. 다시 확인해 주세요.');
      ids.add(item.libraryId); items.push(item);
    }
    pages++;
    if (items.length > group.fileCount || !page.cursor && items.length !== group.fileCount)
      throw new Error('녹화 파일 수를 확인하지 못했습니다. 다시 확인해 주세요.');
    if (!page.cursor) return { status: 'ready', snapshot: { group, items, cursor: null, pages } };
    if (cursors.has(page.cursor)) throw new Error('녹화 파일 목록 위치를 확인하지 못했습니다.');
    cursors.add(page.cursor); cursor = page.cursor;
  }
  return { status: 'ready', snapshot: { group: group!, items, cursor: cursor ?? null, pages } };
}

export const mediaGroupStorageKey = (provider: MediaProvider) => `atsumi.platform-media.${provider}.open-group.v1`;
export function readOpenMediaGroup(provider: MediaProvider): string | null {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(mediaGroupStorageKey(provider)) ?? 'null');
    if (value && typeof value === 'object' && !Array.isArray(value) && 'version' in value && value.version === 1 && 'openKey' in value)
      return typeof value.openKey === 'string' && value.openKey.length > 0 && value.openKey.length <= 512 ? value.openKey : null;
  } catch { /* Optional presentation preference cannot block the library. */ }
  return null;
}
export function saveOpenMediaGroup(provider: MediaProvider, openKey: string | null): void {
  try { localStorage.setItem(mediaGroupStorageKey(provider), JSON.stringify({ version: 1, openKey })); } catch { /* Keep the in-memory preference usable. */ }
}

/** Self-contained so the actual placement can also be exercised with real layout rectangles. */
export function groupPanelPosition(anchor: { left: number; top: number; bottom: number; width: number }, viewportWidth: number, viewportHeight: number) {
  const width = Math.max(0, Math.min(Math.max(250, anchor.width), 420, viewportWidth - 24));
  const left = Math.max(12, Math.min(anchor.left + (anchor.width - width) / 2, viewportWidth - width - 12));
  const below = Math.max(0, viewportHeight - anchor.bottom - 18), above = Math.max(0, anchor.top - 18);
  const placeBelow = below >= Math.min(280, viewportHeight - 24) || below >= above;
  const maxHeight = Math.max(0, Math.min(440, viewportHeight - 24, placeBelow ? below : above));
  const top = Math.max(12, Math.min(placeBelow ? anchor.bottom - 4 : anchor.top - maxHeight + 4, viewportHeight - maxHeight - 12));
  return { left, top, width, maxHeight };
}
