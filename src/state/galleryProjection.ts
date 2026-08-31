import type {
  DownloadEntry,
  DownloadLibraryPage,
  GalleryDetail,
  GalleryPage,
  GalleryPageDimension,
  GallerySummary,
} from "../api/contracts";
import type { Gallery, GalleryId, Language } from "../core/types";

type RuntimeGallerySummary = Partial<Record<keyof GallerySummary, unknown>>;

const runtimeSummary = (summary: GallerySummary): RuntimeGallerySummary =>
  summary as unknown as RuntimeGallerySummary;

const finiteNumber = (value: unknown): number | undefined =>
  typeof value === "number" && Number.isFinite(value) ? value : undefined;

const positiveInteger = (value: unknown): number | undefined =>
  typeof value === "number" && Number.isInteger(value) && value > 0 ? value : undefined;

const stringArray = (value: unknown): string[] | undefined =>
  Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : undefined;

const language = (value: unknown): Language | undefined => {
  switch (value) {
    case "korean":
    case "japanese":
    case "chinese":
    case "english":
      return value;
    default:
      return undefined;
  }
};

const pageDimensions = (values: readonly GalleryPageDimension[]): Gallery["pageDimensions"] => {
  const known = new Set<number>();
  const result: GalleryPageDimension[] = [];
  for (const value of values) {
    const sourcePage = positiveInteger(value.sourcePage);
    if (sourcePage === undefined || known.has(sourcePage)) continue;
    known.add(sourcePage);
    const width = positiveInteger(value.width);
    const height = positiveInteger(value.height);
    result.push({ sourcePage, ...(width !== undefined ? { width } : {}), ...(height !== undefined ? { height } : {}) });
  }
  return result;
};

const publishedAtFromRank = (rank: number): string => {
  const value = Math.trunc(rank).toString().padStart(8, "0");
  if (!/^\d{8}$/.test(value)) return "0000-00-00";
  return `${value.slice(0, 4)}-${value.slice(4, 6)}-${value.slice(6, 8)}`;
};

const coverIndexFor = (id: GalleryId, thumbnailKey?: string): number => {
  let hash = Math.abs(Number(id));
  for (const character of thumbnailKey ?? "") {
    hash = ((hash * 31) + character.charCodeAt(0)) >>> 0;
  }
  return hash % 6;
};

export function projectGallerySummary(summary: GallerySummary, current?: Gallery): Gallery {
  const runtime = runtimeSummary(summary);
  const incomingPublishedRank = finiteNumber(runtime.publishedRank);
  const incomingThumbnailKey = typeof runtime.thumbnailKey === "string" && runtime.thumbnailKey
    ? runtime.thumbnailKey
    : undefined;
  const thumbnailKey = incomingThumbnailKey ?? current?.thumbnailKey;
  const thumbnailWidth = positiveInteger(runtime.thumbnailWidth) ?? current?.thumbnailWidth;
  const thumbnailHeight = positiveInteger(runtime.thumbnailHeight) ?? current?.thumbnailHeight;

  return {
    id: summary.id,
    title: summary.title,
    subtitle: current?.subtitle ?? "",
    artist: summary.artist,
    ...(summary.group ? { group: summary.group } : {}),
    pages: summary.pages,
    score: finiteNumber(runtime.popularity) ?? current?.score ?? 0,
    publishedAt: incomingPublishedRank === undefined
      ? current?.publishedAt ?? "0000-00-00"
      : publishedAtFromRank(incomingPublishedRank),
    coverIndex: current?.coverIndex ?? coverIndexFor(summary.id, thumbnailKey),
    language: language(runtime.language) ?? current?.language ?? "korean",
    tags: stringArray(runtime.tags) ?? (current ? [...current.tags] : []),
    series: stringArray(runtime.series) ?? (Array.isArray(current?.series) ? [...current.series] : []),
    characters: stringArray(runtime.characters) ?? (Array.isArray(current?.characters) ? [...current.characters] : []),
    ...(thumbnailKey ? { thumbnailKey } : {}),
    ...(thumbnailWidth !== undefined ? { thumbnailWidth } : {}),
    ...(thumbnailHeight !== undefined ? { thumbnailHeight } : {}),
    ...(current?.relatedIds ? { relatedIds: current.relatedIds } : {}),
    ...(current?.pageDimensions !== undefined ? { pageDimensions: current.pageDimensions } : {}),
    ...(current?.favorite !== undefined ? { favorite: current.favorite } : {}),
    ...(current?.download ? { download: current.download } : {}),
  };
}

export function mergeGalleryPage(
  galleries: ReadonlyMap<GalleryId, Gallery>,
  page: GalleryPage,
): { galleries: ReadonlyMap<GalleryId, Gallery>; ids: GalleryId[] } {
  const next = new Map(galleries);
  for (const summary of page.items) {
    next.set(summary.id, projectGallerySummary(summary, next.get(summary.id)));
  }
  return { galleries: next, ids: page.items.map((summary) => summary.id) };
}

export function mergeGalleryDetail(
  galleries: ReadonlyMap<GalleryId, Gallery>,
  detail: GalleryDetail,
): ReadonlyMap<GalleryId, Gallery> {
  const next = new Map(galleries);
  const projected = projectGallerySummary(detail, next.get(detail.id));
  next.set(detail.id, {
    ...projected,
    relatedIds: detail.related.map((item) => item.id),
    pageDimensions: pageDimensions(detail.pageDimensions),
  });
  for (const related of detail.related) {
    next.set(related.id, projectGallerySummary(related, next.get(related.id)));
  }
  return next;
}

const placeholderGallery = (id: GalleryId): Gallery => ({
  id,
  title: `Gallery #${id}`,
  subtitle: "",
  artist: "정보 불러오는 중",
  pages: 0,
  score: 0,
  publishedAt: "0000-00-00",
  coverIndex: Math.abs(Number(id)) % 6,
  language: "korean",
  languageKnown: false,
  tags: [],
  series: [],
  characters: [],
});

export function mergeDownloadEntries(
  galleries: ReadonlyMap<GalleryId, Gallery>,
  entries: DownloadEntry[],
): ReadonlyMap<GalleryId, Gallery> {
  const next = new Map(galleries);
  for (const entry of entries) {
    const current = next.get(entry.galleryId) ?? placeholderGallery(entry.galleryId);
    if (
      current.download?.entryId === entry.entryId
      && entry.revision <= (current.download.revision ?? -1)
    ) {
      continue;
    }
    next.set(entry.galleryId, {
      ...current,
      download: {
        entryId: entry.entryId,
        revision: entry.revision,
        state: entry.state,
        ...(entry.progress !== undefined ? { progress: entry.progress } : {}),
        ...(entry.attempt !== undefined ? { attempt: entry.attempt } : {}),
        ...(entry.errorCode !== undefined ? { errorCode: entry.errorCode } : {}),
        ...(entry.errorMessage !== undefined ? { errorMessage: entry.errorMessage } : {}),
        ...(entry.reviewKind !== undefined ? { reviewKind: entry.reviewKind } : {}),
        ...(entry.reviewId !== undefined ? { reviewId: entry.reviewId } : {}),
        ...(entry.createdAt !== undefined ? { createdAt: entry.createdAt } : current.download?.createdAt !== undefined ? { createdAt: current.download.createdAt } : {}),
        ...(entry.updatedAt !== undefined ? { updatedAt: entry.updatedAt } : current.download?.updatedAt !== undefined ? { updatedAt: current.download.updatedAt } : {}),
      },
    });
  }
  return next;
}

export function mergeDownloadLibraryPage(
  galleries: ReadonlyMap<GalleryId, Gallery>,
  page: DownloadLibraryPage,
): { galleries: ReadonlyMap<GalleryId, Gallery>; ids: GalleryId[] } {
  const next = new Map(galleries);
  for (const item of page.items) {
    const current = next.get(item.gallery.id);
    const summary = {
      id: item.gallery.id,
      title: item.gallery.title ?? current?.title ?? `Gallery #${item.gallery.id}`,
      artist: item.gallery.artist ?? current?.artist ?? "정보 불러오는 중",
      ...(item.gallery.group ?? current?.group ? { group: item.gallery.group ?? current?.group } : {}),
      pages: item.gallery.pages ?? current?.pages ?? 0,
      language: item.gallery.language ?? current?.language ?? "korean",
      tags: current ? [...current.tags] : [],
      series: current ? [...current.series] : [],
      characters: current ? [...current.characters] : [],
      ...(item.gallery.publishedRank !== undefined ? { publishedRank: item.gallery.publishedRank } : {}),
      ...(current?.score !== undefined ? { popularity: current.score } : {}),
      ...(current?.thumbnailKey ? { thumbnailKey: current.thumbnailKey } : {}),
      ...(current?.thumbnailWidth !== undefined ? { thumbnailWidth: current.thumbnailWidth } : {}),
      ...(current?.thumbnailHeight !== undefined ? { thumbnailHeight: current.thumbnailHeight } : {}),
    } as GallerySummary;
    const projected = projectGallerySummary(summary, current);
    next.set(item.gallery.id, {
      ...projected,
      languageKnown: item.gallery.language !== undefined
        || (current !== undefined && current.languageKnown !== false),
      download: {
        entryId: item.download.entryId,
        revision: item.download.revision,
        state: item.download.state,
        ...(item.download.progress !== undefined ? { progress: item.download.progress } : {}),
        ...(item.download.attempt !== undefined ? { attempt: item.download.attempt } : {}),
        ...(item.download.errorCode !== undefined ? { errorCode: item.download.errorCode } : {}),
        ...(item.download.errorMessage !== undefined ? { errorMessage: item.download.errorMessage } : {}),
        ...(item.download.reviewKind !== undefined ? { reviewKind: item.download.reviewKind } : {}),
        ...(item.download.reviewId !== undefined ? { reviewId: item.download.reviewId } : {}),
        ...(item.download.createdAt !== undefined ? { createdAt: item.download.createdAt } : current?.download?.createdAt !== undefined ? { createdAt: current.download.createdAt } : {}),
        ...(item.download.updatedAt !== undefined ? { updatedAt: item.download.updatedAt } : current?.download?.updatedAt !== undefined ? { updatedAt: current.download.updatedAt } : {}),
      },
    });
  }
  return { galleries: next, ids: page.items.map((item) => item.gallery.id) };
}
