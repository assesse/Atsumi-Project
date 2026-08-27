import type { Gallery, GalleryId, UiState } from "../core/types";
import { normalizeTokenValue } from "../search/searchTokens";

const matchesQuery = (gallery: Gallery, query: string): boolean => {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;

  const separator = needle.indexOf(":");
  if (separator > 0) {
    const namespace = needle.slice(0, separator);
    const value = needle.slice(separator + 1);
    const metadataValue = normalizeTokenValue(value);
    if (namespace === "artist") return normalizeTokenValue(gallery.artist).includes(metadataValue);
    if (namespace === "group") return gallery.group ? normalizeTokenValue(gallery.group).includes(metadataValue) : false;
    if (namespace === "series") {
      return (gallery.series ?? []).some((item) => normalizeTokenValue(item).includes(metadataValue));
    }
    if (namespace === "character") {
      return (gallery.characters ?? []).some((item) => normalizeTokenValue(item).includes(metadataValue));
    }
    if (namespace === "language") return gallery.language.includes(value);
    if (namespace === "tag") {
      return gallery.tags.some((tag) => normalizeTokenValue(tag.replace(/^tag:/, "")).includes(metadataValue));
    }
    return gallery.tags.some((tag) => tag.toLocaleLowerCase().includes(needle));
  }

  return [String(gallery.id), gallery.title, gallery.subtitle, gallery.artist, gallery.group ?? "", ...(gallery.series ?? []), ...(gallery.characters ?? []), ...gallery.tags]
    .join(" ")
    .toLocaleLowerCase()
    .includes(needle);
};

const matchesDownloadFilter = (gallery: Gallery, state: UiState): boolean => {
  if (!gallery.download) return false;
  switch (state.downloadsFilter) {
    case "all":
      return true;
    case "active":
      return ["queued", "resolving_metadata", "downloading", "hashing", "verifying", "retry_wait"].includes(
        gallery.download.state,
      );
    case "review":
      return gallery.download.state === "review_required";
    case "failed":
      return ["failed", "interrupted"].includes(gallery.download.state);
    case "complete":
      return gallery.download.state === "completed";
  }
};

export function visibleGalleries(state: UiState, galleries: Iterable<Gallery>): Gallery[] {
  const search = state.search[state.view];
  let items = [...galleries].filter((gallery) => search.languages.includes(gallery.language));

  if (state.view === "auto-find") {
    items = items.filter((gallery) => gallery.favorite && gallery.download?.state !== "quarantined");
  }
  if (state.view === "downloads") {
    items = items.filter((gallery) => gallery.download?.state !== "quarantined" && matchesDownloadFilter(gallery, state));
  }
  items = items.filter((gallery) => matchesQuery(gallery, search.committed));

  if (state.view === "explore") {
    if (state.exploreSort === "recent") {
      items.sort((left, right) => right.publishedAt.localeCompare(left.publishedAt) || right.id - left.id);
    } else if (state.exploreSort.startsWith("popular")) {
      items.sort((left, right) => right.score - left.score);
    } else if (state.exploreSort === "random") {
      items.sort((left, right) => ((left.id * 2654435761) >>> 0) - ((right.id * 2654435761) >>> 0));
    }
  }
  return items;
}

export function withGalleryPatch(
  items: ReadonlyMap<GalleryId, Gallery>,
  id: GalleryId,
  patch: Partial<Gallery>,
): ReadonlyMap<GalleryId, Gallery> {
  const current = items.get(id);
  if (!current) return items;
  const next = new Map(items);
  next.set(id, { ...current, ...patch });
  return next;
}
