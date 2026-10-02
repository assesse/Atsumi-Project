import { invoke, isTauri } from "@tauri-apps/api/core";
import type { Gallery, GalleryId, Language } from "../../core/types";

export type Target = { source: "hitomi"; galleryId: number; /** Zero means the album itself. */ page: number };
export type GallerySnapshot = Pick<Gallery, "title" | "artist" | "pages" | "language"> & {
  artists: string[]; thumbnailWidth: number | null; thumbnailHeight: number | null;
};
export type SavedKey = Target & { collectionIds: string[] };
export type Collection = { id: string; name: string; count: number };
export type Summary = { revision: number; keys: SavedKey[]; collections: Collection[] };
export type Bookmark = Target & {
  snapshot: GallerySnapshot; createdAt: string;
  reference: { status: "remote" | "local" | "excluded" | "changed" | "unavailable"; entryId: string | null };
};
export type LibraryQuery = { kind: "albums" | "pages"; collectionId: string | null; search: string; offset: number; limit: number };
export type Request = { action: "summary" } | ({ action: "list" } & LibraryQuery)
  | { action: "get"; target: Target }
  | { action: "bookmark_set"; target: Target; enabled: boolean; snapshot: GallerySnapshot | null }
  | { action: "collection_save"; id: string | null; name: string }
  | { action: "collection_delete"; id: string }
  | { action: "membership_set"; target: Target; collectionId: string; enabled: boolean };
export type Response = { summary: Summary; items: Bookmark[]; total: number; collectionId: string | null };
export type LibraryApi = (request: Request) => Promise<Response>;
export const targetKey = (target: Target): string => `${target.source}:${target.galleryId}:${target.page}`;
export const targetFor = (gallery: Gallery, page = 0): Target => ({ source: "hitomi", galleryId: gallery.id, page });
export const snapshotFor = (gallery: Gallery): GallerySnapshot => ({
  title: gallery.title, artist: gallery.artist, artists: gallery.artists ?? [gallery.artist], pages: gallery.pages,
  language: gallery.language, thumbnailWidth: gallery.thumbnailWidth ?? null, thumbnailHeight: gallery.thumbnailHeight ?? null,
});
export function galleryFor(item: Bookmark): Gallery {
  return { id: item.galleryId as GalleryId, ...item.snapshot, thumbnailWidth: item.snapshot.thumbnailWidth ?? undefined,
    thumbnailHeight: item.snapshot.thumbnailHeight ?? undefined, language: item.snapshot.language as Language,
    subtitle: "", score: 0, publishedAt: "", coverIndex: 0, tags: [], tagsKnown: false, series: [], characters: [],
    ...(item.reference.status === "local" && item.reference.entryId ? { download: { entryId: item.reference.entryId, state: "completed" as const } } : {}),
  };
}

/** Browser preview only. Native personal records never live in webview storage. */
export function createBrowserLibraryApi(storage: Pick<Storage, "getItem" | "setItem">): LibraryApi {
  type Data = { revision: number; items: Array<Bookmark & { collectionIds: string[] }>; collections: Array<{ id: string; name: string }> };
  const storageKey = "atsumi.personal-library.preview.v1";
  return async (request) => {
    const raw = storage.getItem(storageKey);
    const data: Data = raw ? JSON.parse(raw) : { revision: 0, items: [], collections: [] };
    if (!Array.isArray(data.items) || !Array.isArray(data.collections) || !Number.isSafeInteger(data.revision)) throw new Error("즐겨찾기 저장 데이터를 확인할 수 없습니다. 기존 데이터는 유지됩니다.");
    let items: Bookmark[] = [], total = 0, collectionId: string | null = null;
    if (request.action === "list") {
      const query = request.search.trim().toLocaleLowerCase();
      const filtered = data.items.filter((item) => (request.kind === "albums" ? item.page === 0 : item.page > 0)
        && (!request.collectionId || (request.collectionId === "unfiled" ? item.collectionIds.length === 0 : item.collectionIds.includes(request.collectionId)))
        && `${item.snapshot.title} ${item.snapshot.artist} ${item.snapshot.artists.join(" ")} ${item.galleryId}`.toLocaleLowerCase().includes(query))
        .sort((a, b) => b.createdAt.localeCompare(a.createdAt) || b.galleryId - a.galleryId || b.page - a.page);
      total = filtered.length; items = filtered.slice(request.offset, request.offset + request.limit);
    } else if (request.action === "get") {
      const item = data.items.find((item) => targetKey(item) === targetKey(request.target));
      if (!item) throw new Error("즐겨찾기가 해제된 항목입니다."); items = [item]; total = 1;
    } else if (request.action === "bookmark_set") {
      const key = targetKey(request.target);
      if (request.enabled && !data.items.some((item) => targetKey(item) === key)) {
        if (!request.snapshot || request.target.page < 0 || request.target.page > request.snapshot.pages) throw new Error("페이지 정보를 확인해 주세요.");
        data.items.push({ ...request.target, snapshot: request.snapshot, createdAt: new Date().toISOString(), collectionIds: [], reference: { status: "remote", entryId: null } });
      } else if (!request.enabled) data.items = data.items.filter((item) => targetKey(item) !== key);
    } else if (request.action === "collection_save") {
      const name = request.name.trim().replace(/\s+/g, " ");
      if (!name || Array.from(name).length > 60) throw new Error("컬렉션 이름은 1~60자로 입력해 주세요.");
      if (data.collections.some((c) => c.id !== request.id && c.name.toLocaleLowerCase() === name.toLocaleLowerCase())) throw new Error("같은 이름의 컬렉션이 있습니다.");
      if (request.id) {
        const collection = data.collections.find((c) => c.id === request.id);
        if (!collection) throw new Error("컬렉션을 찾을 수 없습니다."); collection.name = name; collectionId = collection.id;
      } else { collectionId = crypto.randomUUID(); data.collections.push({ id: collectionId, name }); }
    } else if (request.action === "collection_delete") {
      data.collections = data.collections.filter((c) => c.id !== request.id);
      data.items.forEach((item) => { item.collectionIds = item.collectionIds.filter((id) => id !== request.id); });
    } else if (request.action === "membership_set") {
      const item = data.items.find((item) => targetKey(item) === targetKey(request.target));
      if (!item || !data.collections.some((c) => c.id === request.collectionId)) throw new Error("저장된 항목과 컬렉션을 확인해 주세요.");
      item.collectionIds = request.enabled ? [...new Set([...item.collectionIds, request.collectionId])] : item.collectionIds.filter((id) => id !== request.collectionId);
    }
    if (!["summary", "list", "get"].includes(request.action)) {
      data.revision++; storage.setItem(storageKey, JSON.stringify(data));
    }
    return { items, total, collectionId, summary: { revision: data.revision,
      keys: data.items.map(({ source, galleryId, page, collectionIds }) => ({ source, galleryId, page, collectionIds })),
      collections: data.collections.map((collection) => ({ ...collection, count: data.items.filter((item) => item.collectionIds.includes(collection.id)).length })),
    } };
  };
}

export const libraryApi: LibraryApi = (request) => isTauri()
  ? invoke<Response>("personal_library", { request })
  : createBrowserLibraryApi(localStorage)(request);
