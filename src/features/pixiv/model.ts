export type PixivMode = "home" | "illustration" | "manga" | "latest" | "daily" | "weekly" | "artist";
export type PixivHomeMode = "home" | "illustration" | "manga";
export const isPixivHome = (mode: PixivMode): mode is PixivHomeMode => mode === "home" || mode === "illustration" || mode === "manga";
export type PixivView = "explore" | "favorites" | "downloads";
export type PixivTagTranslation = { tag: string; ko?: string; en?: string; romaji?: string };
export type PixivWork = {
  id: string; title: string; artistId: string; artist: string;
  pageCount: number; width: number; height: number; thumbnailUrl?: string;
  bookmarkCount: number; tags: string[]; tagTranslations?: PixivTagTranslation[]; createdAt: string;
  type: "illustration" | "manga" | "ugoira";
};
export type PixivPage = { index: number; width: number; height: number; previewUrl: string; originalUrl: string };
export type PixivDetail = { work: PixivWork; pages: PixivPage[] };
export type PixivFavoriteTagTranslation = { tag: string; translation: string; language: "ko" | "en" };
export type PixivHomeSuggestions = { recommendedTags: string[]; popularTags: string[]; popularNovelTags?: string[]; favoriteTags?: string[]; favoriteTagTranslations?: PixivFavoriteTagTranslation[]; tagTranslations?: PixivTagTranslation[] };
export type PixivDiscoverySection = { id: string; title: string; kind: "followed" | "recommended" | "tag" | "newest" | "related"; tag?: string; items: PixivWork[] };
export type PixivCreator = { id: string; name: string; imageUrl?: string; works: PixivWork[] };
export type PixivDiscovery = { sections: PixivDiscoverySection[]; creators: PixivCreator[]; suggestions?: PixivHomeSuggestions; cursor?: string; page: number; issues: string[] };
export type PixivDiscoveryRequest = { surface: PixivHomeMode | "related"; workId?: string; cursor?: string; page?: number; safeOnly?: boolean };
export type PixivSearch = { items: PixivWork[]; page: number; hasMore: boolean; homeSuggestions?: PixivHomeSuggestions; homeCursor?: string };
export type PixivDownload = { work: PixivWork; savedAt: string; pageCount: number; bytes: number; folder: string; issue?: string; removedToken?: string };
export type PixivBookmark = { work: PixivWork; page?: number; savedAt: number };
export type PixivProgress = { id: string; completedPages: number; totalPages: number; bytes: number; cancelled: boolean; queued?: boolean };
export type PixivDownloadOutcome = { id: string; saved?: PixivDownload; error?: string };
// Desktop originals include encoded byte size; string values support fixture APIs.
export type PixivMedia = string | { url: string; bytes: number; preview?: boolean };
// No account identifiers, cookie values or authorization material cross IPC.
export type PixivAccountSnapshot = { authenticated: boolean | null; open: boolean; busy: boolean; revision: number; error?: string | null };
export type PixivApi = {
  account?(): Promise<PixivAccountSnapshot>;
  login?(): Promise<PixivAccountSnapshot>;
  logout?(): Promise<PixivAccountSnapshot>;
  search(request: { query: string; page: number; mode: PixivMode; homeCursor?: string; safeOnly?: boolean }): Promise<PixivSearch>;
  discover?(request: PixivDiscoveryRequest): Promise<PixivDiscovery>;
  detail(input: string): Promise<PixivDetail>;
  localDetail(id: string): Promise<PixivDetail>;
  localMedia(id: string, page: number, detail?: boolean): Promise<PixivMedia>;
  pageMedia(id: string, page: number): Promise<PixivMedia>;
  media(url: string, detail?: boolean): Promise<string>;
  download(id: string): Promise<PixivDownload>;
  downloadBatch(ids: string[]): Promise<PixivDownloadOutcome[]>;
  subscribeDownloads?(listener: (outcome: PixivDownloadOutcome) => void): Promise<() => void>;
  downloads(): Promise<PixivDownload[]>;
  progress(): Promise<PixivProgress[]>;
  cancel(id: string): Promise<void>;
  open(id: string): Promise<void>;
  libraryAction(id: string, action: "open" | "remove" | "restore", token?: string): Promise<{ token?: string; record?: PixivDownload }>;
};
export const PIXIV_BOOKMARK_KEY = "atsumi.pixiv.bookmarks.v1";
export const PIXIV_HISTORY_KEY = "atsumi.pixiv.search-history.v1";
export function loadHistory(): string[] { try { const value: unknown = JSON.parse(localStorage.getItem(PIXIV_HISTORY_KEY) ?? "[]"); return Array.isArray(value) ? [...new Set(value.filter((item): item is string => typeof item === "string" && item.trim().length > 0 && item.length <= 120))].slice(0, 10) : []; } catch { return []; } }
export const artworkUrl = (id: string) => `https://www.pixiv.net/artworks/${id}`;
export const localPageUrl = (id: string, page = 0) => `atsumi-pixiv:${id}:${page}`;
export function artworkId(input: string): string | null {
  const value = input.trim();
  if (/^[1-9]\d{0,11}$/.test(value)) return value;
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || !["pixiv.net", "www.pixiv.net"].includes(url.hostname) || url.username || url.password || (url.port && url.port !== "443")) return null;
    return url.pathname.match(/^\/(?:[a-z]{2}\/)?artworks\/([1-9]\d{0,11})\/?$/)?.[1] ?? null;
  } catch { return null; }
}
export function bookmarkKey(work: Pick<PixivWork, "id">, page?: number): string { return `${work.id}:${page ?? "album"}`; }
export function loadBookmarks(): PixivBookmark[] {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(PIXIV_BOOKMARK_KEY) ?? "[]");
    if (!Array.isArray(raw)) return [];
    return raw.filter((item): item is PixivBookmark => Boolean(item && typeof item === "object" && item.work && artworkId(String(item.work.id)) === item.work.id && typeof item.work.title === "string" && typeof item.work.artist === "string" && (item.page === undefined || Number.isSafeInteger(item.page) && item.page >= 0))).slice(0, 2000).map(item => ({
      ...item, savedAt: Number.isFinite(item.savedAt) ? item.savedAt : 0,
      work: { ...item.work, artistId: typeof item.work.artistId === "string" ? item.work.artistId : "", bookmarkCount: Number.isFinite(item.work.bookmarkCount) ? item.work.bookmarkCount : 0, pageCount: Number.isSafeInteger(item.work.pageCount) && item.work.pageCount > 0 ? item.work.pageCount : 1, tags: Array.isArray(item.work.tags) ? item.work.tags.filter(tag => typeof tag === "string") : [], thumbnailUrl: typeof item.work.thumbnailUrl === "string" && /^https:\/\/i\.pximg\.net\//.test(item.work.thumbnailUrl) ? item.work.thumbnailUrl : undefined },
    }));
  } catch { return []; }
}
export function updateSelection(ids: string[], current: string[], id: string, anchor: string | null, modifiers: { ctrl: boolean; shift: boolean }): string[] {
  if (modifiers.shift && anchor && ids.includes(anchor)) {
    const a = ids.indexOf(anchor), b = ids.indexOf(id);
    const range = ids.slice(Math.min(a, b), Math.max(a, b) + 1);
    return modifiers.ctrl ? [...new Set([...current, ...range])] : range;
  }
  if (modifiers.ctrl) return current.includes(id) ? current.filter(value => value !== id) : [...current, id];
  return current.length === 1 && current[0] === id ? [] : [id];
}
