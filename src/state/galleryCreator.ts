import type { Gallery } from "../core/types";
import { normalizeTokenValue } from "../search/searchTokens";

const missingArtistLabels = new Set([
  "unknown_artist", "unknown", "알_수_없는_작가", "작가_정보_없음", "정보_불러오는_중",
]);

export function isKnownArtist(value: string): boolean {
  const normalized = normalizeTokenValue(value);
  return Boolean(normalized) && !missingArtistLabels.has(normalized);
}

/** A display/grouping fallback, never a rewrite of source artist metadata. */
export function galleryCreator(gallery: Pick<Gallery, "artist" | "artists" | "group">) {
  const artist = isKnownArtist(gallery.artist) ? gallery.artist.trim()
    : gallery.artists?.find(isKnownArtist)?.trim();
  if (artist) return { kind: "artist" as const, value: artist, label: artist };
  const group = gallery.group?.trim();
  if (group) return { kind: "group" as const, value: group, label: `그룹 · ${group.replaceAll("_", " ")}` };
  return { kind: "unknown" as const, value: "", label: "작가·그룹 정보 없음" };
}
