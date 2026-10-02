import type { FavoriteKey } from "../api/contracts";
import type { Gallery } from "../core/types";
import { normalizeTokenValue } from "../search/searchTokens";
import { groupGalleries, type GalleryAccordionGrouping, type GalleryGroup } from "./galleryGrouping";

const normalizedSets = new WeakMap<ReadonlySet<string>, ReadonlySet<string>>();
const normalizedFavorites = (values: ReadonlySet<string>): ReadonlySet<string> => {
  const cached = normalizedSets.get(values);
  if (cached) return cached;
  const normalized = new Set([...values].map(normalizeTokenValue));
  normalizedSets.set(values, normalized);
  return normalized;
};

/** Complete participant metadata is authoritative; never split legacy artist names. */
export const participatingArtists = (gallery: Pick<Gallery, "artist" | "artists">): readonly string[] =>
  gallery.artists?.some((name) => name.trim()) ? gallery.artists : [gallery.artist];

export function autoFindFavoriteMatches(
  gallery: Gallery,
  favorites: ReadonlySet<string>,
  recordedMatch?: string | readonly string[],
): FavoriteKey[] {
  const normalized = normalizedFavorites(favorites);
  const matches = new Map<string, FavoriteKey>();
  const add = (namespace: "artist" | "group", raw: string) => {
    const value = normalizeTokenValue(raw);
    const token = `${namespace}:${value}`;
    if (value && normalized.has(token)) matches.set(token, { namespace, value: value.replaceAll("_", " ") });
  };
  participatingArtists(gallery).forEach((artist) => add("artist", artist));
  if (gallery.group) add("group", gallery.group);
  for (const recorded of typeof recordedMatch === "string" ? [recordedMatch] : recordedMatch ?? []) {
    const split = recorded.indexOf(":");
    const namespace = recorded.slice(0, split);
    // Legacy caches lack all artists. Group discovery is also authoritative
    // when an album's summary only exposes the first of several source groups.
    if (namespace === "group" || (namespace === "artist" && !gallery.artists?.some((name) => name.trim()))) {
      add(namespace, recorded.slice(split + 1));
    }
  }
  return [...matches.values()];
}

export function groupAutoFindGalleries(
  galleries: readonly Gallery[],
  grouping: GalleryAccordionGrouping,
  dateForGallery: (gallery: Gallery) => string | undefined,
  favorites: ReadonlySet<string>,
  recordedMatches: ReadonlyMap<Gallery["id"], string | readonly string[]>,
): GalleryGroup[] {
  if (grouping === "day") return groupGalleries(galleries, grouping, dateForGallery);
  const groups = new Map<string, GalleryGroup>();
  for (const gallery of uniqueAutoFindGalleries(galleries)) {
    for (const match of autoFindFavoriteMatches(gallery, favorites, recordedMatches.get(gallery.id))) {
      const key = `artist\u001f${match.namespace}:${match.value}`;
      const group = groups.get(key) ?? {
        key, label: `${match.namespace === "group" ? "그룹" : "작가"} · ${match.value}`, items: [],
      };
      group.items.push(gallery);
      groups.set(key, group);
    }
  }
  return [...groups.values()].sort((left, right) => left.label.localeCompare(right.label, "ko"));
}

/** Group membership may repeat a card, but pagination and actions count IDs once. */
export function uniqueAutoFindGalleries(galleries: readonly Gallery[]): Gallery[] {
  const seen = new Set<Gallery["id"]>();
  return galleries.filter((gallery) => {
    if (seen.has(gallery.id)) return false;
    seen.add(gallery.id);
    return true;
  });
}
