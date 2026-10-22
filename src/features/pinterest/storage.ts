import { normalizePinterestInput, type PinterestFavorite, type PinterestPin } from "./types";

export const PINTEREST_FAVORITES_KEY = "atsumi.pinterest.favorites.v1";

function cleanPin(value: unknown): PinterestPin | null {
  if (!value || typeof value !== "object") return null;
  const pin = value as Partial<PinterestPin>;
  if (typeof pin.id !== "string" || normalizePinterestInput(pin.id) !== pin.id || typeof pin.title !== "string"
    || typeof pin.description !== "string" || !Array.isArray(pin.media) || pin.media.length > 50
    || typeof pin.sourceUrl !== "string" || normalizePinterestInput(pin.sourceUrl) !== pin.id) return null;
  const media = [];
  for (const item of pin.media) {
    if (!item || typeof item.url !== "string" || item.url.length > 2048) return null;
    try {
      const url = new URL(item.url);
      if (url.protocol !== "https:" || url.hostname !== "i.pinimg.com" || url.username || url.password || url.port) return null;
    } catch { return null; }
    media.push({ url: item.url, ...(Number.isInteger(item.width) && item.width! > 0 ? { width: Math.min(item.width!, 100000) } : {}), ...(Number.isInteger(item.height) && item.height! > 0 ? { height: Math.min(item.height!, 100000) } : {}) });
  }
  // Reconstruct known fields: never persist image bytes, server payloads or injected fields.
  return { id: pin.id, title: pin.title.slice(0, 500), description: pin.description.slice(0, 8000), sourceUrl: `https://www.pinterest.com/pin/${pin.id}/`, media,
    unsupportedVideo: pin.unsupportedVideo === true, ...(pin.summaryOnly ? { summaryOnly: true } : {}),
    ...(typeof pin.creatorName === "string" ? { creatorName: pin.creatorName.slice(0, 200) } : {}),
    ...(typeof pin.creatorPath === "string" && /^\/[a-zA-Z0-9_-]{1,100}\/$/.test(pin.creatorPath) ? { creatorPath: pin.creatorPath } : {}) };
}

function cleanFavorite(value: unknown): PinterestFavorite | null {
  if (!value || typeof value !== "object") return null;
  const item = value as PinterestFavorite, pin = cleanPin(item.pin);
  return pin && (item.page === null || (Number.isInteger(item.page) && item.page >= 0 && item.page < pin.media.length)) ? { pin, page: item.page } : null;
}

export function loadPinterestFavorites(): PinterestFavorite[] {
  try {
    const data: unknown = JSON.parse(localStorage.getItem(PINTEREST_FAVORITES_KEY) ?? "[]");
    if (!Array.isArray(data)) return [];
    return data.slice(0, 2000).map(cleanFavorite).filter((item): item is PinterestFavorite => item !== null);
  } catch { return []; }
}

export function savePinterestFavorites(favorites: PinterestFavorite[]): boolean {
  if (favorites.length > 2000) return false;
  const clean = favorites.map(cleanFavorite);
  if (clean.some(item => !item)) return false;
  try { localStorage.setItem(PINTEREST_FAVORITES_KEY, JSON.stringify(clean)); return true; }
  catch { return false; }
}

