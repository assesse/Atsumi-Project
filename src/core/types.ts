export type Brand<T, Name extends string> = T & { readonly __brand: Name };

export type GalleryId = Brand<number, "GalleryId">;
export type ViewId = "explore" | "auto-find" | "downloads";
export type Language = "korean" | "japanese" | "chinese" | "english";
export type SearchSort =
  | "recent"
  | "popular_today"
  | "popular_week"
  | "popular_month"
  | "popular_year"
  | "random";

export type DownloadState =
  | "queued"
  | "resolving_metadata"
  | "downloading"
  | "hashing"
  | "verifying"
  | "retry_wait"
  | "review_required"
  | "interrupted"
  | "failed"
  | "completed"
  | "quarantined"
  | "cancelled";

export type Gallery = {
  id: GalleryId;
  title: string;
  subtitle: string;
  artist: string;
  group?: string;
  pages: number;
  score: number;
  publishedAt: string;
  coverIndex: number;
  language: Language;
  tags: string[];
  series: string[];
  characters: string[];
  thumbnailKey?: string;
  thumbnailWidth?: number;
  thumbnailHeight?: number;
  /** Present only after the Detail metadata request has completed. */
  pageDimensions?: ReadonlyArray<{ readonly sourcePage: number; readonly width?: number; readonly height?: number }>;
  relatedIds?: GalleryId[];
  favorite?: boolean;
  download?: {
    entryId: string;
    revision?: number;
    state: DownloadState;
    progress?: number;
    attempt?: number;
    errorCode?: string;
    errorMessage?: string;
    reviewKind?: "gallery_duplicate" | "internal_pages";
    reviewId?: string;
    createdAt?: string;
    updatedAt?: string;
  };
};

export type SearchUi = {
  draft: string;
  committed: string;
  languages: Language[];
  suggestionsOpen: boolean;
  activeSuggestion: number | null;
};

export type SelectionState = {
  ids: ReadonlySet<GalleryId>;
  anchorId: GalleryId | null;
};

export type DetailState = {
  tabs: GalleryId[];
  activeId: GalleryId | null;
  minimized: boolean;
};

export type DownloadFilter = "all" | "active" | "review" | "failed" | "complete";

export const retryableDownloadStates: ReadonlySet<DownloadState> = new Set([
  "interrupted",
  "failed",
  "cancelled",
]);

export type UiState = {
  view: ViewId;
  railCollapsed: boolean;
  search: Record<ViewId, SearchUi>;
  exploreSort: SearchSort;
  downloadsFilter: DownloadFilter;
  grouping: Record<"auto-find" | "downloads", "all" | "day" | "artist">;
  selection: SelectionState;
  detail: DetailState;
  overlays: {
    activityOpen: boolean;
    settingsOpen: boolean;
    reviewGalleryId: GalleryId | null;
    exitConfirmOpen: boolean;
  };
};

export const galleryId = (value: number): GalleryId => value as GalleryId;
