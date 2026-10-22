export type PinterestMedia = { url: string; width?: number; height?: number };
export type PinterestPin = {
  id: string;
  title: string;
  description: string;
  creatorName?: string;
  creatorPath?: string;
  summaryOnly?: boolean;
  sourceUrl: string;
  media: PinterestMedia[];
  unsupportedVideo: boolean;
};
export type PinterestDownload = { pin: PinterestPin; files: string[]; bytes: number; savedAt: number; missingFiles?: boolean };
export type PinterestDownloadOutcome = { pinId: string; status: "completed" | "failed" | "cancelled"; record?: PinterestDownload; error?: string };
export type PinterestRemoved = { token: string; record: PinterestDownload };
export type PinterestDiscoveryKind = "home" | "keyword" | "profile" | "saved";
export type PinterestDiscovery = { title: string; path: string; pins: PinterestPin[]; boards: { title: string; path: string }[]; limited: boolean; cursor?: string | null; authStatus?: "signedIn" | "signedOut" | "unknown"; generation?: number };
export type PinterestApi = {
  discover(input: string, kind: PinterestDiscoveryKind, requestId: string, cursor?: string): Promise<PinterestDiscovery>;
  get(input: string, requestId: string): Promise<PinterestPin>;
  preview(pinId: string, page: number, requestId: string, thumbnail: boolean, removedToken?: string): Promise<string>;
  download(pinId: string, requestId: string): Promise<PinterestDownload>;
  downloadBatch(pinIds: string[], requestId: string, onProgress: (outcome: PinterestDownloadOutcome) => void): Promise<PinterestDownloadOutcome[]>;
  downloads(): Promise<PinterestDownload[]>;
  removed(requestId: string): Promise<PinterestRemoved[]>;
  cancel(requestId: string): Promise<void>;
  openExternal(pinId?: string): Promise<void>;
  libraryAction(action: "archive" | "restore" | "folder", id: string, requestId: string): Promise<string>;
};
export type PinterestFavorite = { pin: PinterestPin; page: number | null };
export type PinterestView = "explore" | "favorites" | "downloads";

/** IDs are strings: Pinterest IDs exceed JavaScript's safe integer range. */
export function normalizePinterestInput(value: string): string | null {
  const input = value.trim();
  if (/^[1-9]\d{4,24}$/.test(input)) return input;
  try {
    const url = new URL(input);
    if (url.protocol !== "https:" || url.username || url.password || url.port) return null;
    if (!/^(?:(?:www|[a-z]{2})\.)?pinterest\.com$/.test(url.hostname)) return null;
    const match = /^\/pin\/(?:[^/]*--)?([1-9]\d{4,24})\/?$/.exec(url.pathname);
    return match?.[1] ?? null;
  } catch { return null; }
}

export const pinterestFavoriteKey = (pinId: string, page: number | null) => `${pinId}:${page ?? "album"}`;

/** Independent desktop selection rules; do not couple Pinterest to Hitomi state. */
export function selectPinterestCard(ids: readonly string[], selected: ReadonlySet<string>, id: string, anchor: string | null,
  modifiers: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): Set<string> {
  if (modifiers.shiftKey && anchor && ids.includes(anchor)) {
    const first = ids.indexOf(anchor), last = ids.indexOf(id);
    if (last < 0) return new Set(selected);
    const range = ids.slice(Math.min(first, last), Math.max(first, last) + 1);
    return new Set(modifiers.ctrlKey || modifiers.metaKey ? [...selected, ...range] : range);
  }
  if (modifiers.ctrlKey || modifiers.metaKey) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  }
  return selected.size === 1 && selected.has(id) ? new Set() : new Set([id]);
}

