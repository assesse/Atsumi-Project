export type DanbooruRating = "g" | "s" | "q" | "e";
export type DanbooruFileType = "jpg" | "png" | "gif" | "webp" | "avif" | "webm" | "mp4" | "zip";
export type DanbooruRelationship = "any" | "has_parent" | "no_parent" | "has_children" | "no_children";
export type DanbooruSort =
  | "newest"
  | "oldest"
  | "score"
  | "favorites"
  | "resolution"
  | "file_size"
  | "tag_count"
  | "portrait"
  | "landscape";

export type DanbooruSearchFilters = {
  ratings: DanbooruRating[];
  fileTypes: DanbooruFileType[];
  dateFrom: string;
  dateTo: string;
  minimumScore: string;
  minimumFavorites: string;
  relationship: DanbooruRelationship;
  sort: DanbooruSort;
};

export const DANBOORU_RATINGS: ReadonlyArray<{ value: DanbooruRating; label: string; description: string }> = [
  { value: "g", label: "General", description: "일반 공개에 적합한 이미지" },
  { value: "s", label: "Sensitive", description: "노출이 적더라도 주의가 필요한 이미지" },
  { value: "q", label: "Questionable", description: "성인·노골적 요소가 강한 이미지" },
  { value: "e", label: "Explicit", description: "명시적인 성인 이미지" },
];

export const DANBOORU_FILE_TYPES: ReadonlyArray<{ value: DanbooruFileType; label: string }> = [
  { value: "jpg", label: "JPG" },
  { value: "png", label: "PNG" },
  { value: "gif", label: "GIF" },
  { value: "webp", label: "WebP" },
  { value: "avif", label: "AVIF" },
  { value: "webm", label: "WebM" },
  { value: "mp4", label: "MP4" },
  { value: "zip", label: "Ugoira ZIP" },
];

export const DANBOORU_SORTS: ReadonlyArray<{ value: DanbooruSort; label: string; metatag?: string }> = [
  { value: "newest", label: "최신 등록순" },
  { value: "oldest", label: "오래된 등록순", metatag: "order:id_asc" },
  { value: "score", label: "점수 높은순", metatag: "order:score" },
  { value: "favorites", label: "즐겨찾기 많은순", metatag: "order:favcount" },
  { value: "resolution", label: "해상도 높은순", metatag: "order:mpixels" },
  { value: "file_size", label: "파일 큰순", metatag: "order:filesize" },
  { value: "tag_count", label: "태그 많은순", metatag: "order:tagcount" },
  { value: "portrait", label: "세로 이미지 우선", metatag: "order:portrait" },
  { value: "landscape", label: "가로 이미지 우선", metatag: "order:landscape" },
];

const DANBOORU_UNLIMITED_METATAGS = new Set([
  "status", "rating", "limit", "is", "id", "date", "age", "filesize", "filetype",
  "parent", "child", "md5", "width", "height", "duration", "mpixels", "ratio", "score",
  "upvote", "downvotes", "favcount", "embedded", "tagcount", "pixiv_id", "pixiv",
]);

export const defaultDanbooruSearchFilters = (): DanbooruSearchFilters => ({
  ratings: DANBOORU_RATINGS.map(({ value }) => value),
  fileTypes: DANBOORU_FILE_TYPES.map(({ value }) => value),
  dateFrom: "",
  dateTo: "",
  minimumScore: "",
  minimumFavorites: "",
  relationship: "any",
  sort: "newest",
});

const preferenceKey = "atsumi.danbooru-search-preferences.v1";
export const DANBOORU_SEARCH_PREFERENCES_CHANGED = "atsumi:danbooru-search-preferences-changed";

const isDate = (value: unknown): value is string => typeof value === "string" && (/^\d{4}-\d{2}-\d{2}$/).test(value);
const numericText = (value: unknown): string => typeof value === "string" && (/^-?\d+$/).test(value) ? value : "";

export const sanitizeDanbooruSearchFilters = (value: unknown): DanbooruSearchFilters => {
  const defaults = defaultDanbooruSearchFilters();
  if (!value || typeof value !== "object") return defaults;
  const candidate = value as Partial<DanbooruSearchFilters>;
  const ratings = Array.isArray(candidate.ratings)
    ? DANBOORU_RATINGS.map(({ value }) => value).filter((rating) => candidate.ratings?.includes(rating))
    : defaults.ratings;
  const fileTypes = Array.isArray(candidate.fileTypes)
    ? DANBOORU_FILE_TYPES.map(({ value }) => value).filter((fileType) => candidate.fileTypes?.includes(fileType))
    : defaults.fileTypes;
  const relationship = (["any", "has_parent", "no_parent", "has_children", "no_children"] as const)
    .includes(candidate.relationship as DanbooruRelationship)
    ? candidate.relationship as DanbooruRelationship
    : defaults.relationship;
  const sort = DANBOORU_SORTS.some(({ value }) => value === candidate.sort)
    ? candidate.sort as DanbooruSort
    : defaults.sort;
  return {
    ratings,
    fileTypes,
    dateFrom: isDate(candidate.dateFrom) ? candidate.dateFrom : "",
    dateTo: isDate(candidate.dateTo) ? candidate.dateTo : "",
    minimumScore: numericText(candidate.minimumScore),
    minimumFavorites: numericText(candidate.minimumFavorites),
    relationship,
    sort,
  };
};

export const loadDanbooruSearchPreferences = (): DanbooruSearchFilters => {
  try {
    return sanitizeDanbooruSearchFilters(JSON.parse(window.localStorage.getItem(preferenceKey) ?? "null"));
  } catch {
    return defaultDanbooruSearchFilters();
  }
};

export const saveDanbooruSearchPreferences = (filters: DanbooruSearchFilters): void => {
  const sanitized = sanitizeDanbooruSearchFilters(filters);
  try {
    window.localStorage.setItem(preferenceKey, JSON.stringify(sanitized));
    window.dispatchEvent(new CustomEvent(DANBOORU_SEARCH_PREFERENCES_CHANGED, { detail: sanitized }));
  } catch {
    // Search remains usable when local preferences cannot be persisted.
  }
};

const selectionMetatag = (values: string[], allValues: readonly string[], name: string): string | null => {
  if (!values.length || values.length === allValues.length) return null;
  return `${name}:${values.join(",")}`;
};

export const buildDanbooruSearchQuery = (tags: string, filters: DanbooruSearchFilters): string => {
  const raw = tags.trim();
  if (/^\d+$/.test(raw)) return raw;
  const terms = raw.split(/\s+/).filter(Boolean);
  const rating = selectionMetatag(filters.ratings, DANBOORU_RATINGS.map(({ value }) => value), "rating");
  const fileType = selectionMetatag(filters.fileTypes, DANBOORU_FILE_TYPES.map(({ value }) => value), "filetype");
  if (rating) terms.push(rating);
  if (fileType) terms.push(fileType);
  if (filters.dateFrom && filters.dateTo) terms.push(`date:${filters.dateFrom}..${filters.dateTo}`);
  else if (filters.dateFrom) terms.push(`date:>=${filters.dateFrom}`);
  else if (filters.dateTo) terms.push(`date:<=${filters.dateTo}`);
  if (filters.minimumScore) terms.push(`score:>=${filters.minimumScore}`);
  if (filters.minimumFavorites) terms.push(`favcount:>=${filters.minimumFavorites}`);
  if (filters.relationship === "has_parent") terms.push("parent:any");
  else if (filters.relationship === "no_parent") terms.push("parent:none");
  else if (filters.relationship === "has_children") terms.push("child:any");
  else if (filters.relationship === "no_children") terms.push("child:none");
  const sort = DANBOORU_SORTS.find(({ value }) => value === filters.sort)?.metatag;
  if (sort) terms.push(sort);
  return terms.join(" ");
};

export const danbooruLimitedTermCount = (query: string): number => query.trim().split(/\s+/).filter(Boolean).filter((term) => {
  const normalized = term.replace(/^-/, "");
  const separator = normalized.indexOf(":");
  if (separator < 1) return true;
  return !DANBOORU_UNLIMITED_METATAGS.has(normalized.slice(0, separator).toLowerCase());
}).length;

export const activeDanbooruFilterCount = (filters: DanbooruSearchFilters): number => {
  const defaults = defaultDanbooruSearchFilters();
  return Number(filters.ratings.length !== defaults.ratings.length)
    + Number(filters.fileTypes.length !== defaults.fileTypes.length)
    + Number(Boolean(filters.dateFrom || filters.dateTo))
    + Number(Boolean(filters.minimumScore))
    + Number(Boolean(filters.minimumFavorites))
    + Number(filters.relationship !== "any");
};
