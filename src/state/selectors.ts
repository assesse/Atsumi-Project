import type { Gallery, GalleryId, UiState } from "../core/types";
import { normalizeTokenValue } from "../search/searchTokens";

const galleryHaystack = (gallery: Gallery): string => normalizeTokenValue([
  String(gallery.id),
  gallery.title,
  gallery.subtitle,
  gallery.artist,
  gallery.group ?? "",
  ...(gallery.series ?? []),
  ...(gallery.characters ?? []),
  ...gallery.tags,
].join(" "));

const matchesSearchToken = (gallery: Gallery, rawToken: string): boolean => {
  const negative = rawToken.startsWith("-");
  const token = negative ? rawToken.slice(1) : rawToken;
  if (!token) return true;

  const separator = token.indexOf(":");
  let matched: boolean;
  if (separator > 0) {
    const namespace = token.slice(0, separator).toLocaleLowerCase();
    const metadataValue = normalizeTokenValue(token.slice(separator + 1));
    if (!metadataValue) return true;
    if (namespace === "artist") matched = normalizeTokenValue(gallery.artist).includes(metadataValue);
    else if (namespace === "group") matched = gallery.group ? normalizeTokenValue(gallery.group).includes(metadataValue) : false;
    else if (namespace === "series") {
      matched = (gallery.series ?? []).some((item) => normalizeTokenValue(item).includes(metadataValue));
    } else if (namespace === "character") {
      matched = (gallery.characters ?? []).some((item) => normalizeTokenValue(item).includes(metadataValue));
    } else if (namespace === "language") matched = gallery.language.includes(metadataValue);
    else if (namespace === "tag") {
      matched = gallery.tags.some((tag) => {
        const normalized = tag.toLocaleLowerCase();
        return !normalized.startsWith("female:")
          && !normalized.startsWith("male:")
          && normalizeTokenValue(normalized.replace(/^tag:/, "")).includes(metadataValue);
      });
    } else if (namespace === "female" || namespace === "male") {
      matched = gallery.tags.some((tag) => {
        const normalized = tag.toLocaleLowerCase();
        return normalized.startsWith(`${namespace}:`)
          && normalizeTokenValue(normalized.slice(normalized.indexOf(":") + 1)).includes(metadataValue);
      });
    } else matched = galleryHaystack(gallery).includes(normalizeTokenValue(token));
  } else {
    matched = galleryHaystack(gallery).includes(normalizeTokenValue(token));
  }
  return negative ? !matched : matched;
};

const matchesQuery = (gallery: Gallery, query: string): boolean => {
  const tokens = query.trim().split(/\s+/).filter(Boolean);
  return tokens.every((token) => matchesSearchToken(gallery, token));
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
  const directExploreId = state.view === "explore" && /^\d{7}$/.test(search.committed.trim());
  let items = [...galleries].filter((gallery) => directExploreId
    || search.languages.includes(gallery.language)
    || (state.view === "downloads" && gallery.languageKnown === false));

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
  } else if (state.view === "downloads" && state.grouping.downloads === "all") {
    items.sort((left, right) => {
      const leftCreatedAt = left.download?.createdAt ?? left.download?.updatedAt ?? left.publishedAt;
      const rightCreatedAt = right.download?.createdAt ?? right.download?.updatedAt ?? right.publishedAt;
      return rightCreatedAt.localeCompare(leftCreatedAt) || right.id - left.id;
    });
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
