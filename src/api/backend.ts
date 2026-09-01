import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { galleryId, type GalleryId } from "../core/types";
import {
  GALLERY_PREVIEW_PRESETS,
  normalizeGalleryPreviewWidth,
} from "../layout/galleryPreviewPresets";
import type {
  AppActiveWorkSnapshot,
  AppExitRequestedEvent,
  AppQuitRequest,
  AppQuitResult,
  ApiResult,
  AutoFindHistoryMode,
  AutoFindExclusionResult,
  AutoFindRun,
  AutoFindSnapshot,
  DownloadChangedEvent,
  DanbooruAutocompleteItem,
  DanbooruDownloadRecord,
  DanbooruDownloadsPage,
  DanbooruDownloadsRequest,
  DanbooruPost,
  DanbooruRelatedPosts,
  DanbooruRelatedRequest,
  DanbooruSearchPage,
  DanbooruSearchRequest,
  DetailOriginalPrepareRequest,
  DetailOriginalPrepared,
  DownloadEntry,
  DownloadLibraryGallery,
  DownloadLibraryPage,
  DownloadOverlapDecisionRequest,
  DownloadOverlapDecisionResult,
  DownloadOverlapReview,
  DownloadListRequest,
  DownloadPage,
  DuplicateCandidate,
  DuplicateDecisionRequest,
  DuplicateGalleryRef,
  DuplicateReview,
  DuplicateScanRun,
  DuplicateSnapshot,
  ExplorationDataResetRequest,
  ExplorationDataResetResult,
  ExplorationExclusion,
  ExplorationExclusionRestoreResult,
  FavoriteKey,
  FavoriteMutationResult,
  FavoriteRecord,
  GalleryDetail,
  GalleryPage,
  JobEvent,
  JobRef,
  InternalDuplicateReview,
  InternalDuplicateSnapshot,
  InternalArtifactScanProgress,
  InternalRemovalApplyRequest,
  InternalRemovalPlan,
  InternalRemovalPlanRequest,
  InternalRemovalResult,
  InternalRemovalUndoRequest,
  InternalScanRequest,
  InternalScanRun,
  MaintenanceAction,
  MaintenancePreview,
  MaintenanceResult,
  ReconcileReport,
  SearchHistoryEntry,
  SearchRequest,
  SearchSubmission,
  SettingsPatch,
  SettingsSnapshot,
  StorageUsageSnapshot,
  ThumbnailCompletionEvent,
  ThumbnailCacheClearResult,
  ThumbnailInvalidation,
  ThumbnailRequestDto,
  ThumbnailRequestToken,
  ThumbnailWorkerStats,
  TagCatalogStatus,
  TagNamespace,
  TagSuggestion,
  TagSuggestionRequest,
  WindowPlacement,
  WindowPlacementSnapshot,
} from "./contracts";
import { hasActiveWork } from "./contracts";
import { applyGlobalSearchRules } from "../search/globalSearchRules";
import {
  galleryDetailFixture,
  normalizeSearchRequest,
  runSearchFixture,
  searchFixturePage,
  searchFixtureQueryId,
  searchRequestValidationError,
  type SearchFixtureResult,
} from "./searchFixture";

export type BackendEventMap = {
  "auto-find:changed": AutoFindRun;
  "duplicate:changed": DuplicateScanRun;
  "internal-duplicate:changed": InternalScanRun;
  "internal-duplicate:artifact-progress": InternalArtifactScanProgress;
  "job:changed": JobEvent;
  "download:changed": DownloadChangedEvent;
  "thumbnail:ready": ThumbnailCompletionEvent;
  "settings:changed": SettingsSnapshot;
  "app:exit-requested": AppExitRequestedEvent;
};

export type Unsubscribe = () => void;

export interface BackendClient {
  readonly runtime: "tauri" | "browser-mock";
  settingsGet(): Promise<ApiResult<SettingsSnapshot>>;
  settingsUpdate(patch: SettingsPatch, expectedRevision: number): Promise<ApiResult<SettingsSnapshot>>;
  storageUsageGet(): Promise<ApiResult<StorageUsageSnapshot>>;
  danbooruSearch(request: DanbooruSearchRequest): Promise<ApiResult<DanbooruSearchPage>>;
  danbooruRandom(): Promise<ApiResult<DanbooruPost>>;
  danbooruRelated(request: DanbooruRelatedRequest): Promise<ApiResult<DanbooruRelatedPosts>>;
  danbooruAutocomplete(query: string, limit: number): Promise<ApiResult<DanbooruAutocompleteItem[]>>;
  danbooruDownload(postId: number): Promise<ApiResult<DanbooruDownloadRecord>>;
  danbooruDownloadsList(request: DanbooruDownloadsRequest): Promise<ApiResult<DanbooruDownloadsPage>>;
  tagCatalogStatus(): Promise<ApiResult<import("./contracts").TagCatalogStatus>>;
  tagCatalogRefresh(): Promise<ApiResult<import("./contracts").TagCatalogStatus>>;
  tagSuggestionsSearch(request: import("./contracts").TagSuggestionRequest): Promise<ApiResult<import("./contracts").TagSuggestion[]>>;
  folderNameTemplatePreview(template: string): Promise<ApiResult<string>>;
  windowPlacementGet(): Promise<ApiResult<WindowPlacementSnapshot>>;
  windowPlacementUpdate(
    placement: WindowPlacement,
    expectedRevision: number,
  ): Promise<ApiResult<WindowPlacementSnapshot>>;
  searchSubmit(request: SearchRequest): Promise<ApiResult<SearchSubmission>>;
  searchPageGet(queryId: string, page: number, requestId: string): Promise<ApiResult<GalleryPage>>;
  searchPageCancel(requestId: string): Promise<ApiResult<boolean>>;
  galleryDetailGet(galleryId: GalleryDetail["id"]): Promise<ApiResult<GalleryDetail>>;
  favoritesList(): Promise<ApiResult<FavoriteRecord[]>>;
  favoriteSet(key: FavoriteKey, enabled: boolean): Promise<ApiResult<FavoriteMutationResult>>;
  searchHistoryList(limit: number): Promise<ApiResult<SearchHistoryEntry[]>>;
  autoFindSnapshot(): Promise<ApiResult<AutoFindSnapshot>>;
  autoFindRefresh(): Promise<ApiResult<AutoFindRun>>;
  autoFindCancel(): Promise<ApiResult<AutoFindRun>>;
  autoFindExclude(galleryIds: GalleryId[], reason: string): Promise<ApiResult<AutoFindExclusionResult>>;
  explorationExclusionsList(): Promise<ApiResult<ExplorationExclusion[]>>;
  explorationExclusionsRestore(galleryIds: GalleryId[]): Promise<ApiResult<ExplorationExclusionRestoreResult>>;
  duplicateSnapshot(): Promise<ApiResult<DuplicateSnapshot>>;
  duplicateScanStart(): Promise<ApiResult<DuplicateScanRun>>;
  duplicateScanCancel(): Promise<ApiResult<DuplicateScanRun>>;
  duplicateReviewGet(candidateId: string): Promise<ApiResult<DuplicateReview>>;
  duplicateDecisionApply(request: DuplicateDecisionRequest): Promise<ApiResult<DuplicateReview>>;
  downloadOverlapReviewGet(reviewId: string): Promise<ApiResult<DownloadOverlapReview>>;
  downloadOverlapDecisionApply(request: DownloadOverlapDecisionRequest): Promise<ApiResult<DownloadOverlapDecisionResult>>;
  internalDuplicateSnapshot(): Promise<ApiResult<InternalDuplicateSnapshot>>;
  internalDuplicateActiveArtifact(): Promise<ApiResult<InternalArtifactScanProgress | null>>;
  internalDuplicateScanStart(request: InternalScanRequest): Promise<ApiResult<InternalScanRun>>;
  internalDuplicateScanCancel(): Promise<ApiResult<InternalScanRun>>;
  internalDuplicateReviewGet(entryId: string): Promise<ApiResult<InternalDuplicateReview>>;
  internalRemovalPlan(request: InternalRemovalPlanRequest): Promise<ApiResult<InternalRemovalPlan>>;
  internalRemovalApply(request: InternalRemovalApplyRequest): Promise<ApiResult<InternalRemovalResult>>;
  internalRemovalUndo(request: InternalRemovalUndoRequest): Promise<ApiResult<InternalRemovalResult>>;
  downloadQueueAdd(galleries: GalleryId[], requestId: string): Promise<ApiResult<DownloadEntry[]>>;
  downloadEntriesList(request: DownloadListRequest): Promise<ApiResult<DownloadPage>>;
  downloadLibraryPageList(request: DownloadListRequest): Promise<ApiResult<DownloadLibraryPage>>;
  downloadRetry(entryIds: string[]): Promise<ApiResult<JobRef[]>>;
  downloadCancel(entryIds: string[]): Promise<ApiResult<DownloadEntry[]>>;
  downloadQuarantine(entryIds: string[], reason: string): Promise<ApiResult<DownloadEntry[]>>;
  downloadQuarantineUndo(entryIds: string[]): Promise<ApiResult<DownloadEntry[]>>;
  appActiveWorkSnapshot(): Promise<ApiResult<AppActiveWorkSnapshot>>;
  artifactOpenFirst(entryId: string): Promise<ApiResult<null>>;
  artifactOpenFolder(entryId: string): Promise<ApiResult<null>>;
  appReconcile(): Promise<ApiResult<ReconcileReport>>;
  maintenancePreview(action: MaintenanceAction): Promise<ApiResult<MaintenancePreview>>;
  maintenanceExecute(previewId: string, action: MaintenanceAction): Promise<ApiResult<MaintenanceResult>>;
  thumbnailRequest(request: ThumbnailRequestDto): Promise<ApiResult<ThumbnailRequestToken>>;
  thumbnailCancel(requestId: string): Promise<ApiResult<boolean>>;
  thumbnailReprioritize(requestId: string, priority: ThumbnailRequestDto["priority"]): Promise<ApiResult<boolean>>;
  thumbnailInvalidate(key: ThumbnailRequestDto["key"]): Promise<ApiResult<ThumbnailInvalidation>>;
  thumbnailStats(): Promise<ApiResult<ThumbnailWorkerStats>>;
  thumbnailCacheClear(): Promise<ApiResult<ThumbnailCacheClearResult>>;
  detailOriginalPrepare(request: DetailOriginalPrepareRequest): Promise<ApiResult<DetailOriginalPrepared>>;
  detailOriginalDispose(requestId: string): Promise<ApiResult<boolean>>;
  explorationDataReset(request: ExplorationDataResetRequest): Promise<ApiResult<ExplorationDataResetResult>>;
  appMinimizeToTray(): Promise<ApiResult<null>>;
  appQuit(request: AppQuitRequest): Promise<ApiResult<AppQuitResult>>;
  on<K extends keyof BackendEventMap>(event: K, handler: (payload: BackendEventMap[K]) => void): Promise<Unsubscribe>;
}

const defaultSettings: SettingsSnapshot = {
  revision: 0,
  downloadRoot: "",
  folderNameTemplate: "[{artist}] {title} [{group}] {id}",
  autoFindHistoryMode: "include_all_history",
  downloadOverlapAutoMode: "off",
  explorePageSize: 50,
  danbooruPageSize: 60,
  maxColumns: 3,
  previewWidth: 220,
  danbooruPreviewWidth: 190,
  relatedPreviewWidth: 240,
  privacyMode: false,
  cacheLimitGb: 10,
  concurrentImageRequests: 5,
  requestStartIntervalMs: 25,
  autoFindGrouping: "all",
  downloadsGrouping: "all",
  exploreDisplayMode: "detail",
  autoFindDisplayMode: "detail",
  downloadsDisplayMode: "detail",
  collapsedGroupKeys: [],
  searchIncludeTags: [],
  searchExcludeTags: [],
};

const normalizeGlobalSearchTags = (value: unknown): string[] | null => {
  if (!Array.isArray(value) || value.length > 100) return null;
  const normalized: string[] = [];
  for (const item of value) {
    if (typeof item !== "string") return null;
    const tag = item.trim().toLocaleLowerCase();
    if (!tag || new TextEncoder().encode(tag).length > 200) return null;
    normalized.push(tag);
  }
  return [...new Set(normalized)].sort((left, right) => left.localeCompare(right));
};

const isGalleryGrouping = (value: unknown): value is SettingsSnapshot["autoFindGrouping"] =>
  value === "all" || value === "day" || value === "artist";

const isGalleryDisplayMode = (value: unknown): value is SettingsSnapshot["exploreDisplayMode"] =>
  value === "detail" || value === "compact";

const windowsPathForDisplay = (value: string): string => {
  const uncPrefix = "\\\\?\\UNC\\";
  if (value.startsWith(uncPrefix)) {
    const rest = value.slice(uncPrefix.length);
    const [server, share] = rest.split("\\");
    return server && share ? `\\\\${rest}` : value;
  }
  const verbatimPrefix = "\\\\?\\";
  if (!value.startsWith(verbatimPrefix)) return value;
  const rest = value.slice(verbatimPrefix.length);
  return /^[A-Za-z]:\\/.test(rest) ? rest : value;
};

const browserFolderPreviewFixtures = new Map<string, string>([
  ["[{artist}] {title} [{group}] {id}", "[작가] 작품 제목 [그룹] 4113714"],
  ["{title}:<{artist}>* [{group}] {id}", "작품 제목__작가__ [그룹] 4113714"],
]);

const BROWSER_SETTINGS_STORAGE_KEY = "atsumi.browser.settings.v1";

const readPersistedBrowserSettings = (): SettingsSnapshot => {
  if (typeof window === "undefined") return { ...defaultSettings };
  try {
    const raw = window.localStorage.getItem(BROWSER_SETTINGS_STORAGE_KEY);
    if (!raw) return { ...defaultSettings };
    const parsed = JSON.parse(raw) as Partial<SettingsSnapshot>;
    return {
      ...defaultSettings,
      ...parsed,
      downloadRoot: windowsPathForDisplay(parsed.downloadRoot ?? defaultSettings.downloadRoot),
      previewWidth: normalizeGalleryPreviewWidth(parsed.previewWidth ?? defaultSettings.previewWidth),
      relatedPreviewWidth: [180, 200, 220, 240, 260, 280, 300, 320].includes(parsed.relatedPreviewWidth ?? defaultSettings.relatedPreviewWidth)
        ? parsed.relatedPreviewWidth ?? defaultSettings.relatedPreviewWidth
        : defaultSettings.relatedPreviewWidth,
      explorePageSize: Number.isInteger(parsed.explorePageSize)
        && (parsed.explorePageSize ?? 0) >= 10
        && (parsed.explorePageSize ?? 0) <= 200
        ? parsed.explorePageSize ?? defaultSettings.explorePageSize
        : defaultSettings.explorePageSize,
      danbooruPageSize: Number.isInteger(parsed.danbooruPageSize)
        && (parsed.danbooruPageSize ?? 0) >= 10
        && (parsed.danbooruPageSize ?? 0) <= 100
        ? parsed.danbooruPageSize ?? defaultSettings.danbooruPageSize
        : defaultSettings.danbooruPageSize,
      danbooruPreviewWidth: GALLERY_PREVIEW_PRESETS.some((preset) => preset.width === parsed.danbooruPreviewWidth)
        ? parsed.danbooruPreviewWidth ?? defaultSettings.danbooruPreviewWidth
        : defaultSettings.danbooruPreviewWidth,
      privacyMode: parsed.privacyMode === true,
      autoFindGrouping: isGalleryGrouping(parsed.autoFindGrouping) ? parsed.autoFindGrouping : "all",
      downloadsGrouping: isGalleryGrouping(parsed.downloadsGrouping) ? parsed.downloadsGrouping : "all",
      exploreDisplayMode: isGalleryDisplayMode(parsed.exploreDisplayMode) ? parsed.exploreDisplayMode : "detail",
      autoFindDisplayMode: isGalleryDisplayMode(parsed.autoFindDisplayMode) ? parsed.autoFindDisplayMode : "detail",
      downloadsDisplayMode: isGalleryDisplayMode(parsed.downloadsDisplayMode) ? parsed.downloadsDisplayMode : "detail",
      collapsedGroupKeys: Array.isArray(parsed.collapsedGroupKeys)
        ? [...new Set(parsed.collapsedGroupKeys.filter((key): key is string => typeof key === "string" && key.trim().length > 0))].sort((left, right) => left.localeCompare(right))
        : [],
      searchIncludeTags: normalizeGlobalSearchTags(parsed.searchIncludeTags) ?? [],
      searchExcludeTags: normalizeGlobalSearchTags(parsed.searchExcludeTags) ?? [],
      autoFindHistoryMode: parsed.autoFindHistoryMode === "newer_than_oldest_downloaded"
        ? "newer_than_oldest_downloaded"
        : "include_all_history",
      downloadOverlapAutoMode: parsed.downloadOverlapAutoMode === "recommend"
        || parsed.downloadOverlapAutoMode === "strict_quarantine"
        ? parsed.downloadOverlapAutoMode
        : "off",
    };
  } catch {
    return { ...defaultSettings };
  }
};

const persistBrowserSettings = (settings: SettingsSnapshot): void => {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(BROWSER_SETTINGS_STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // Browser storage can be unavailable in private/test contexts; memory parity remains usable.
  }
};

const defaultPlacement: WindowPlacementSnapshot = {
  revision: 0,
  x: null,
  y: null,
  width: 1280,
  height: 820,
  maximized: false,
};

const ok = <T,>(data: T): ApiResult<T> => ({ ok: true, data });

const conflict = (kind: string): ApiResult<never> => ({
  ok: false,
  error: {
    code: "REVISION_CONFLICT",
    message: `${kind}이(가) 다른 창에서 변경되었습니다.`,
    retryable: false,
    action: "review",
  },
});

const validationError = (field: string, reason: string): ApiResult<never> => ({
  ok: false,
  error: {
    code: "VALIDATION_ERROR",
    message: `${field}: ${reason}`,
    retryable: false,
    action: "none",
    details: { field, reason },
  },
});

const notFoundError = (
  code: string,
  message: string,
  details?: Record<string, unknown>,
): ApiResult<never> => ({
  ok: false,
  error: { code, message, retryable: false, action: "none", ...(details ? { details } : {}) },
});

const validateIntegerRange = (
  value: number,
  field: string,
  minimum: number,
  maximum: number,
): ApiResult<never> | null => {
  if (!Number.isInteger(value) || value < minimum || value > maximum) {
    return validationError(field, `${minimum} 이상 ${maximum} 이하의 정수여야 합니다.`);
  }
  return null;
};

const validateFolderNameTemplate = (template: string): ApiResult<never> | null => {
  if (!template.trim()) return validationError("folderNameTemplate", "비어 있을 수 없습니다.");
  if (new TextEncoder().encode(template).length > 512) {
    return validationError("folderNameTemplate", "UTF-8 기준 512바이트 이하여야 합니다.");
  }
  if ([...template].some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f);
  })) {
    return validationError("folderNameTemplate", "제어 문자를 포함할 수 없습니다.");
  }

  const allowed = new Set(["artist", "title", "group", "id"]);
  let hasId = false;
  for (let index = 0; index < template.length;) {
    if (template[index] === "}") {
      return validationError("folderNameTemplate", "중괄호가 올바르게 닫혀야 합니다.");
    }
    if (template[index] !== "{") {
      index += 1;
      continue;
    }
    const end = template.indexOf("}", index + 1);
    if (end < 0 || template.slice(index + 1, end).includes("{")) {
      return validationError("folderNameTemplate", "중괄호가 올바르게 닫혀야 합니다.");
    }
    const token = template.slice(index + 1, end);
    if (!allowed.has(token)) {
      return validationError("folderNameTemplate", "artist/title/group/id 토큰만 사용할 수 있습니다.");
    }
    hasId ||= token === "id";
    index = end + 1;
  }
  return hasId ? null : validationError("folderNameTemplate", "{id} 토큰이 필요합니다.");
};

const activeDownloadStates: ReadonlySet<DownloadEntry["state"]> = new Set([
  "queued",
  "resolving_metadata",
  "downloading",
  "hashing",
  "verifying",
  "retry_wait",
]);

const browserWorkSetFingerprint = (
  activeDownloadEntryIds: string[],
  autoFindRunId?: string,
  duplicateRunId?: string,
  internalDuplicateRunId?: string,
): string => JSON.stringify({
  downloads: [...activeDownloadEntryIds].sort((left, right) => left.localeCompare(right)),
  autoFind: autoFindRunId ?? null,
  duplicateScan: duplicateRunId ?? null,
  internalDuplicateScan: internalDuplicateRunId ?? null,
});

const cancellableDownloadStates: ReadonlySet<DownloadEntry["state"]> = new Set([
  ...activeDownloadStates,
  "review_required",
  "interrupted",
  "failed",
  "cancelled",
]);

const cloneDownloadEntry = (entry: DownloadEntry): DownloadEntry => ({ ...entry });

const compareDownloadLibraryRecency = (left: DownloadEntry, right: DownloadEntry): number => (
  (left.createdAt ?? "").localeCompare(right.createdAt ?? "")
  || (left.updatedAt ?? "").localeCompare(right.updatedAt ?? "")
  || (left.revision ?? 0) - (right.revision ?? 0)
  || left.entryId.localeCompare(right.entryId)
);

const downloadLibraryDisplayTime = (entry: DownloadEntry): string =>
  entry.createdAt ?? entry.updatedAt ?? "";

const normalizedGallerySet = (galleries: GalleryId[]): GalleryId[] =>
  [...new Set(galleries)].sort((left, right) => left - right);

const gallerySetKey = (galleries: GalleryId[]): string => galleries.join(",");

const cloneFavorite = (favorite: FavoriteRecord): FavoriteRecord => ({ ...favorite });

const cloneSearchHistory = (entry: SearchHistoryEntry): SearchHistoryEntry => ({
  ...entry,
  includeTags: [...entry.includeTags],
  excludeTags: [...entry.excludeTags],
  languages: [...entry.languages],
});

const cloneAutoFindRun = (run: AutoFindRun): AutoFindRun => ({ ...run });

const cloneAutoFindSnapshot = (snapshot: AutoFindSnapshot): AutoFindSnapshot => ({
  ...(snapshot.run ? { run: cloneAutoFindRun(snapshot.run) } : {}),
  candidates: snapshot.candidates.map((candidate) => ({
    ...candidate,
    tags: [...candidate.tags],
    series: [...(candidate.series ?? [])],
    characters: [...(candidate.characters ?? [])],
    matchedFavorite: { ...candidate.matchedFavorite },
  })),
  cutoffEvidence: snapshot.cutoffEvidence.map((evidence) => ({ ...evidence })),
  truncations: snapshot.truncations.map((truncation) => ({ ...truncation })),
});

const historyModeAllows = (
  galleryIdValue: GalleryId,
  evidence: AutoFindSnapshot["cutoffEvidence"][number] | undefined,
  mode: AutoFindRun["historyMode"],
): boolean => mode === "include_all_history"
  || evidence?.oldestOwnedGalleryId === undefined
  || galleryIdValue > evidence.oldestOwnedGalleryId;

const duplicateProfile: DuplicateSnapshot["profile"] = {
  profileVersion: 1,
  algorithmVersion: 1,
  dHashBits: 1024,
  pHashBits: 64,
  visualMatchThreshold: 0.8,
  lowInformationStdDevThreshold: 10,
};

const duplicateGalleryRef = (
  id: number,
  entryId: string,
  title: string,
  artist: string,
  pageCount: number,
): DuplicateGalleryRef => ({
  galleryId: galleryId(id),
  entryId,
  title,
  artist,
  pageCount,
});

const browserDuplicateReviewFixture = (now: string): DuplicateReview => ({
  candidate: {
    candidateId: "browser-duplicate-archive-tram",
    revision: 0,
    parent: duplicateGalleryRef(4_051_038, "browser-artifact-4051038", "Archive of Rain", "serein", 38),
    candidate: duplicateGalleryRef(4_050_754, "browser-artifact-4050754", "The Last Tram", "serein", 24),
    relation: "contains",
    confidence: 0.94,
    matchedPages: 3,
    parentCoverage: 0.079,
    candidateCoverage: 0.125,
    createdAt: now,
    updatedAt: now,
  },
  evidence: [
    {
      evidenceId: "browser-evidence-sequence",
      kind: "sequence_alignment",
      confidence: 0.94,
      matchedPages: 3,
      description: "원본 페이지 번호를 보존한 순서 정렬에서 세 페이지가 대응합니다.",
    },
    {
      evidenceId: "browser-evidence-exact",
      kind: "exact_sha256",
      confidence: 1,
      matchedPages: 1,
      description: "한 페이지의 검증된 SHA-256이 정확히 일치합니다.",
    },
    {
      evidenceId: "browser-evidence-visual",
      kind: "visual_hash",
      confidence: 0.91,
      matchedPages: 2,
      description: "재압축 가능성이 있는 두 페이지가 시각 해시 기준을 통과했습니다.",
    },
  ],
  pagePairs: [
    {
      parentSourcePage: 1,
      candidateSourcePage: 3,
      exactSha256: true,
      dHashDistance: 0,
      pHashDistance: 0,
      detailHashDistance: 0,
      edgeSimilarity: 1,
      visualSimilarity: 1,
      lowInformation: false,
    },
    {
      parentSourcePage: 7,
      candidateSourcePage: 8,
      exactSha256: false,
      dHashDistance: 3,
      pHashDistance: 4,
      detailHashDistance: 31,
      edgeSimilarity: 0.92,
      visualSimilarity: 0.94,
      lowInformation: false,
    },
    {
      parentSourcePage: 12,
      candidateSourcePage: 14,
      exactSha256: false,
      dHashDistance: 5,
      pHashDistance: 6,
      detailHashDistance: 44,
      edgeSimilarity: 0.89,
      visualSimilarity: 0.91,
      lowInformation: false,
    },
  ],
  decisions: [],
  seriesGroups: [],
});

const cloneDuplicateGalleryRef = (gallery: DuplicateGalleryRef): DuplicateGalleryRef => ({ ...gallery });

const cloneDuplicateCandidate = (candidate: DuplicateCandidate): DuplicateCandidate => ({
  ...candidate,
  parent: cloneDuplicateGalleryRef(candidate.parent),
  candidate: cloneDuplicateGalleryRef(candidate.candidate),
});

const cloneDuplicateScanRun = (run: DuplicateScanRun): DuplicateScanRun => ({ ...run });

const cloneDuplicateReview = (review: DuplicateReview): DuplicateReview => ({
  candidate: cloneDuplicateCandidate(review.candidate),
  evidence: review.evidence.map((evidence) => ({ ...evidence })),
  pagePairs: review.pagePairs.map((pair) => ({ ...pair })),
  decisions: review.decisions.map((decision) => ({ ...decision })),
  seriesGroups: review.seriesGroups.map((group) => ({
    ...group,
    members: group.members.map(cloneDuplicateGalleryRef),
  })),
});

const cloneDuplicateSnapshot = (snapshot: DuplicateSnapshot): DuplicateSnapshot => ({
  profile: { ...snapshot.profile },
  ...(snapshot.run ? { run: cloneDuplicateScanRun(snapshot.run) } : {}),
  candidates: snapshot.candidates.map(cloneDuplicateCandidate),
});

const cloneInternalScanRun = (run: InternalScanRun): InternalScanRun => ({ ...run });
const cloneInternalArtifactProgress = (progress: InternalArtifactScanProgress): InternalArtifactScanProgress => ({ ...progress });
const cloneInternalReview = (review: InternalDuplicateReview): InternalDuplicateReview => ({
  ...review,
  groups: review.groups.map((group) => ({
    ...group,
    pages: group.pages.map((page) => ({ ...page })),
  })),
  quarantineRecords: review.quarantineRecords.map((record) => ({ ...record })),
});
const cloneInternalSnapshot = (snapshot: InternalDuplicateSnapshot): InternalDuplicateSnapshot => ({
  ...(snapshot.run ? { run: cloneInternalScanRun(snapshot.run) } : {}),
  groups: snapshot.groups.map((group) => ({
    ...group,
    pages: group.pages.map((page) => ({ ...page })),
  })),
  quarantineRecords: snapshot.quarantineRecords.map((record) => ({ ...record })),
  skips: snapshot.skips.map((skip) => ({ ...skip })),
});

type Handler<K extends keyof BackendEventMap> = (payload: BackendEventMap[K]) => void;
const namespaceRank = (namespace: TagNamespace) => {
  switch (namespace) {
    case "artist": return 0;
    case "group": return 1;
    case "female": return 2;
    case "male": return 3;
    case "tag": return 4;
  }
};

const normalizeSuggestionValue = (value: string) =>
  value.trim().toLowerCase().replaceAll("_", " ").replace(/\s+/g, " ");

const suggestionFavoriteKey = (namespace: TagNamespace, name: string) => {
  const favoriteNamespace = namespace === "artist" || namespace === "group" ? namespace : "tag";
  const value = namespace === "female" || namespace === "male" ? `${namespace}:${name}` : name;
  return `${favoriteNamespace}\u0000${normalizeSuggestionValue(value)}`;
};

const browserOverlapReview = (reviewId: string): DownloadOverlapReview => {
  const incoming = {
    entryId: "browser-incoming-entry",
    galleryId: galleryId(4136275),
    title: "새로 내려받은 판본",
    artists: ["fixture artist"],
    pageCount: 24,
  };
  const relations: DownloadOverlapReview["candidates"][number]["relation"][] = [
    "near_equivalent",
    "incoming_contains_existing",
    "existing_contains_incoming",
    "partial_overlap",
  ];
  return {
    reviewId,
    entryId: incoming.entryId,
    incoming,
    revision: 0,
    state: "pending",
    profileVersion: 1,
    policyVersion: 1,
    incomingFingerprint: "browser-incoming-fingerprint",
    candidates: relations.map((relation, index) => ({
      candidateId: `${reviewId}-candidate-${index + 1}`,
      existing: {
        entryId: `browser-existing-entry-${index + 1}`,
        galleryId: galleryId(4105000 + index),
        title: `기존 보유 판본 ${index + 1}`,
        artists: ["fixture artist"],
        pageCount: 20 + index,
      },
      existingFingerprint: `browser-existing-fingerprint-${index + 1}`,
      relation,
      confidence: 0.96 - index * 0.03,
      matchedPages: 18 - index * 2,
      exactPages: Math.max(0, 12 - index * 4),
      visualPages: 6 + index * 2,
      existingCoverage: 0.9 - index * 0.08,
      incomingCoverage: 0.86 - index * 0.07,
      existingUniquePages: 2 + index,
      incomingUniquePages: 4 + index,
      longestAlignedRun: 8 - index,
      rank: index + 1,
      pagePairs: Array.from({ length: 4 }, (_, pairIndex) => ({
        incomingSourcePage: pairIndex + 1,
        existingSourcePage: pairIndex + 2,
        exactSha256: pairIndex < 2 && index === 0,
        dHashDistance: index + pairIndex,
        pHashDistance: index + pairIndex + 1,
        detailHashDistance: 8 + index + pairIndex,
        edgeSimilarity: 0.94 - index * 0.03,
        visualSimilarity: 0.97 - index * 0.03,
        lowInformation: false,
      })),
    })),
    createdAt: "2026-08-25T00:00:00.000Z",
    updatedAt: "2026-08-25T00:00:00.000Z",
  };
};

const cloneDownloadOverlapReview = (review: DownloadOverlapReview): DownloadOverlapReview => ({
  ...review,
  incoming: { ...review.incoming, artists: [...review.incoming.artists] },
  candidates: review.candidates.map((candidate) => ({
    ...candidate,
    existing: { ...candidate.existing, artists: [...candidate.existing.artists] },
    pagePairs: candidate.pagePairs.map((pair) => ({ ...pair })),
  })),
});

const danbooruFixtureImage = (id: number, accent: string): string => `data:image/svg+xml,${encodeURIComponent(`
  <svg xmlns="http://www.w3.org/2000/svg" width="360" height="480" viewBox="0 0 360 480">
    <rect width="360" height="480" fill="#17262d"/>
    <circle cx="180" cy="190" r="112" fill="${accent}" opacity=".72"/>
    <path d="M70 390L180 250l110 140" fill="none" stroke="#eaf7f9" stroke-width="18" stroke-linecap="round"/>
    <text x="180" y="442" fill="#eaf7f9" text-anchor="middle" font-family="sans-serif" font-size="24">POST #${id}</text>
  </svg>
`)}`;

const browserDanbooruPosts: DanbooruPost[] = Array.from({ length: 18 }, (_, index) => {
  const id = 12_000_000 + index;
  const previewUrl = danbooruFixtureImage(id, ["#ef8098", "#58c6cd", "#f0ad5b"][index % 3]!);
  return {
    id,
    createdAt: new Date(Date.UTC(2026, 7, 31, 18, index)).toISOString(),
    rating: index % 4 === 0 ? "s" : "g",
    score: 120 - index,
    favoriteCount: 40 + index,
    imageWidth: index % 2 ? 1600 : 1200,
    imageHeight: index % 2 ? 1200 : 1800,
    fileExt: "jpg",
    fileSize: 2_000_000 + index * 1_000,
    previewUrl,
    largeUrl: previewUrl,
    fileUrl: previewUrl,
    artists: [`fixture_artist_${index % 3 + 1}`],
    copyrights: [index % 2 ? "original" : "atsumi_fixture"],
    characters: index % 2 ? [`sample_character_${index % 4 + 1}`] : [],
    tags: ["blue_sky", index % 2 ? "landscape" : "portrait"],
    ...(index === 1 || index === 2 ? { parentId: 12_000_000 } : {}),
    ...(index === 6 || index === 7 ? { parentId: 12_000_005 } : {}),
    hasChildren: index === 0 || index === 5,
  };
});

const browserDanbooruPools = [
  { id: 91, name: "atsumi_fixture_sequence", category: "series", postIds: browserDanbooruPosts.slice(0, 7).map(({ id }) => id) },
  { id: 92, name: "fixture_variations", category: "collection", postIds: browserDanbooruPosts.slice(4, 12).map(({ id }) => id) },
];

const danbooruUnlimitedMetatags = new Set([
  "status", "rating", "limit", "is", "id", "date", "age", "filesize", "filetype",
  "parent", "child", "md5", "width", "height", "duration", "mpixels", "ratio", "score",
  "upvote", "downvotes", "favcount", "embedded", "tagcount", "pixiv_id", "pixiv",
]);

const danbooruMetatagName = (term: string): string | null => {
  const normalized = term.replace(/^-/, "");
  const separator = normalized.indexOf(":");
  return separator > 0 ? normalized.slice(0, separator) : null;
};

const browserDanbooruLimitedTermCount = (terms: string[]): number => terms.filter((term) => {
  const name = danbooruMetatagName(term);
  return !name || !danbooruUnlimitedMetatags.has(name);
}).length;

const browserDanbooruMatchesNumeric = (actual: number, expression: string): boolean => {
  const match = /^(>=|<=|>|<)?(-?\d+(?:\.\d+)?)$/.exec(expression);
  if (!match) return true;
  const expected = Number(match[2]);
  if (match[1] === ">=") return actual >= expected;
  if (match[1] === "<=") return actual <= expected;
  if (match[1] === ">") return actual > expected;
  if (match[1] === "<") return actual < expected;
  return actual === expected;
};

const browserDanbooruMatchesDate = (createdAt: string, expression: string): boolean => {
  const date = createdAt.slice(0, 10);
  const range = /^(\d{4}-\d{2}-\d{2})\.\.(\d{4}-\d{2}-\d{2})$/.exec(expression);
  if (range) return date >= range[1]! && date <= range[2]!;
  if (expression.startsWith(">=")) return date >= expression.slice(2);
  if (expression.startsWith("<=")) return date <= expression.slice(2);
  return date === expression;
};

const filterAndSortBrowserDanbooruPosts = (terms: string[]): DanbooruPost[] => {
  const order = terms.find((term) => term.startsWith("order:"))?.slice(6) ?? "id";
  const filtered = browserDanbooruPosts.filter((post) => terms.every((term) => {
    const negative = term.startsWith("-");
    const normalized = term.replace(/^-/, "");
    const separator = normalized.indexOf(":");
    const name = separator > 0 ? normalized.slice(0, separator) : null;
    const value = separator > 0 ? normalized.slice(separator + 1) : normalized;
    let matches: boolean;
    if (name === "rating") matches = value.split(",").includes(post.rating);
    else if (name === "filetype") matches = value.split(",").includes(post.fileExt.toLowerCase());
    else if (name === "date") matches = browserDanbooruMatchesDate(post.createdAt, value);
    else if (name === "score") matches = browserDanbooruMatchesNumeric(post.score, value);
    else if (name === "favcount") matches = browserDanbooruMatchesNumeric(post.favoriteCount, value);
    else if (name === "width") matches = browserDanbooruMatchesNumeric(post.imageWidth, value);
    else if (name === "height") matches = browserDanbooruMatchesNumeric(post.imageHeight, value);
    else if (name === "mpixels") matches = browserDanbooruMatchesNumeric((post.imageWidth * post.imageHeight) / 1_000_000, value);
    else if (name === "parent") matches = value === "any" ? Boolean(post.parentId) : value === "none" ? !post.parentId : String(post.parentId) === value;
    else if (name === "child") matches = value === "any" ? post.hasChildren : value === "none" ? !post.hasChildren : true;
    else if (name && danbooruUnlimitedMetatags.has(name)) matches = true;
    else if (name === "order") matches = true;
    else {
      const tagValue = value.toLowerCase();
      matches = [
        ...post.artists,
        ...post.copyrights,
        ...post.characters,
        ...post.tags,
      ].some((candidate) => candidate.toLowerCase().includes(tagValue));
    }
    return negative ? !matches : matches;
  }));
  return [...filtered].sort((left, right) => {
    if (order === "id_asc") return left.id - right.id;
    if (order === "score") return right.score - left.score || right.id - left.id;
    if (order === "favcount") return right.favoriteCount - left.favoriteCount || right.id - left.id;
    if (order === "mpixels") return (right.imageWidth * right.imageHeight) - (left.imageWidth * left.imageHeight) || right.id - left.id;
    if (order === "filesize") return right.fileSize - left.fileSize || right.id - left.id;
    if (order === "tagcount") return right.tags.length - left.tags.length || right.id - left.id;
    if (order === "portrait") return (right.imageHeight / right.imageWidth) - (left.imageHeight / left.imageWidth) || right.id - left.id;
    if (order === "landscape") return (right.imageWidth / right.imageHeight) - (left.imageWidth / left.imageHeight) || right.id - left.id;
    return right.id - left.id;
  });
};

class BrowserMockBackend implements BackendClient {
  readonly runtime = "browser-mock" as const;
  private settings = readPersistedBrowserSettings();
  private placement = { ...defaultPlacement };
  private listeners: { [K in keyof BackendEventMap]: Set<Handler<K>> } = {
    "auto-find:changed": new Set(),
    "duplicate:changed": new Set(),
    "internal-duplicate:changed": new Set(),
    "internal-duplicate:artifact-progress": new Set(),
    "job:changed": new Set(),
    "download:changed": new Set(),
    "thumbnail:ready": new Set(),
    "settings:changed": new Set(),
    "app:exit-requested": new Set(),
  };
  private searchQueries = new Map<string, SearchFixtureResult>();
  private downloadEntries = new Map<string, DownloadEntry>();
  private activeDownloadEntryByGallery = new Map<number, string>();
  private downloadQueueRequests = new Map<string, { gallerySetKey: string; entries: DownloadEntry[] }>();
  private danbooruDownloads = new Map<number, DanbooruDownloadRecord>();
  private nextDownloadEntryId = 1;
  private nextThumbnailRequestId = 1;
  private pendingThumbnailRequests = new Map<string, ThumbnailRequestDto>();
  private thumbnailRequestsTotal = 0;
  private favorites = new Map<string, FavoriteRecord>();
  private searchHistory = new Map<string, SearchHistoryEntry>();
  private nextSearchHistoryId = 1;
  private autoFind: AutoFindSnapshot = { candidates: [], cutoffEvidence: [], truncations: [] };
  private autoFindExclusions = new Map<GalleryId, { reason: string; createdAt: string; title: string; artist: string }>();
  private excludedAutoFindCandidates = new Map<GalleryId, AutoFindSnapshot["candidates"][number]>();
  private explorationRestoredGalleryIds = new Set<GalleryId>();
  private autoFindGeneration = 0;
  private nextAutoFindRunId = 1;
  private duplicateSnapshotState: DuplicateSnapshot = {
    profile: { ...duplicateProfile },
    candidates: [],
  };
  private duplicateReviews = new Map<string, DuplicateReview>();
  private downloadOverlapReviews = new Map<string, DownloadOverlapReview>();
  private duplicateResolvedCandidates = new Set<string>();
  private duplicateHiddenGalleryIds = new Set<GalleryId>();
  private duplicateGeneration = 0;
  private nextDuplicateRunId = 1;
  private nextDuplicateDecisionId = 1;
  private internalSnapshotState: InternalDuplicateSnapshot = { groups: [], quarantineRecords: [], skips: [] };
  private internalGeneration = 0;
  private nextInternalRunId = 1;
  private internalActiveEntrySetKey: string | null = null;
  private internalArtifactProgress: InternalArtifactScanProgress | null = null;
  private internalPlans = new Map<string, InternalRemovalPlan>();
  private nextInternalPlanId = 1;
  private maintenancePreviews = new Map<string, MaintenanceAction>();
  private tagCatalogStatusValue: TagCatalogStatus = { revision: 1, entryCount: 11, neutralCount: 1, femaleCount: 5, maleCount: 1, artistCount: 2, groupCount: 2, lastSuccessAt: "2026-08-21T00:00:00.000Z" };
  private readonly tagCatalog: TagSuggestion[] = [
    { namespace: "artist", name: "mizuno tooru", token: "artist:mizuno_tooru", galleryCount: 142, favorite: false },
    { namespace: "artist", name: "mizuryu kei", token: "artist:mizuryu_kei", galleryCount: 938, favorite: false },
    { namespace: "group", name: "circle energy", token: "group:circle_energy", galleryCount: 76, favorite: false },
    { namespace: "group", name: "mizuryu kei land", token: "group:mizuryu_kei_land", galleryCount: 451, favorite: false },
    { namespace: "female", name: "big balls", token: "female:big_balls", galleryCount: 4822, favorite: false },
    { namespace: "female", name: "ball sucking", token: "female:ball_sucking", galleryCount: 4367, favorite: false },
    { namespace: "female", name: "balls expansion", token: "female:balls_expansion", galleryCount: 651, favorite: false },
    { namespace: "female", name: "mind control", token: "female:mind_control", galleryCount: 810, favorite: false },
    { namespace: "female", name: "mind break", token: "female:mind_break", galleryCount: 730, favorite: false },
    { namespace: "male", name: "ball sucking", token: "male:ball_sucking", galleryCount: 410, favorite: false },
    { namespace: "tag", name: "football", token: "tag:football", galleryCount: 320, favorite: false },
  ];

  async settingsGet(): Promise<ApiResult<SettingsSnapshot>> {
    return ok({ ...this.settings });
  }

  async settingsUpdate(patch: SettingsPatch, expectedRevision: number): Promise<ApiResult<SettingsSnapshot>> {
    if (expectedRevision !== this.settings.revision) return conflict("설정");
    const next = { ...this.settings, ...patch };
    next.downloadRoot = windowsPathForDisplay(next.downloadRoot);
    const searchIncludeTags = normalizeGlobalSearchTags(next.searchIncludeTags);
    const searchExcludeTags = normalizeGlobalSearchTags(next.searchExcludeTags);
    const invalid =
      validateFolderNameTemplate(next.folderNameTemplate) ??
      (next.autoFindHistoryMode !== "include_all_history" && next.autoFindHistoryMode !== "newer_than_oldest_downloaded"
        ? validationError("autoFindHistoryMode", "must be a supported history mode")
        : null) ??
      (!["off", "recommend", "strict_quarantine"].includes(next.downloadOverlapAutoMode)
        ? validationError("downloadOverlapAutoMode", "must be off, recommend or strict_quarantine")
        : null) ??
      validateIntegerRange(next.explorePageSize, "explorePageSize", 10, 200) ??
      validateIntegerRange(next.danbooruPageSize, "danbooruPageSize", 10, 100) ??
      validateIntegerRange(next.maxColumns, "maxColumns", 1, 4) ??
      (!GALLERY_PREVIEW_PRESETS.some((preset) => preset.width === next.previewWidth)
        ? validationError("previewWidth", "must be one of the supported preview presets")
        : null) ??
      (!GALLERY_PREVIEW_PRESETS.some((preset) => preset.width === next.danbooruPreviewWidth)
        ? validationError("danbooruPreviewWidth", "must be one of the supported preview presets")
        : null) ??
      (![180, 200, 220, 240, 260, 280, 300, 320].includes(next.relatedPreviewWidth)
        ? validationError("relatedPreviewWidth", "must be one of the supported related preview presets")
        : null) ??
      (typeof next.privacyMode !== "boolean"
        ? validationError("privacyMode", "must be a boolean")
        : null) ??
      (!isGalleryGrouping(next.autoFindGrouping)
        ? validationError("autoFindGrouping", "must be all, day or artist")
        : null) ??
      (!isGalleryGrouping(next.downloadsGrouping)
        ? validationError("downloadsGrouping", "must be all, day or artist")
        : null) ??
      (!isGalleryDisplayMode(next.exploreDisplayMode)
        ? validationError("exploreDisplayMode", "must be detail or compact")
        : null) ??
      (!isGalleryDisplayMode(next.autoFindDisplayMode)
        ? validationError("autoFindDisplayMode", "must be detail or compact")
        : null) ??
      (!isGalleryDisplayMode(next.downloadsDisplayMode)
        ? validationError("downloadsDisplayMode", "must be detail or compact")
        : null) ??
      (!Array.isArray(next.collapsedGroupKeys)
        || next.collapsedGroupKeys.length > 2_048
        || next.collapsedGroupKeys.some((key) => typeof key !== "string" || !key.trim() || key.length > 256)
        ? validationError("collapsedGroupKeys", "must contain at most 2048 non-empty group keys")
        : null) ??
      (!searchIncludeTags
        ? validationError("searchIncludeTags", "must contain at most 100 non-empty tags of at most 200 bytes")
        : null) ??
      (!searchExcludeTags
        ? validationError("searchExcludeTags", "must contain at most 100 non-empty tags of at most 200 bytes")
        : null) ??
      (searchIncludeTags?.some((tag) => searchExcludeTags?.includes(tag))
        ? validationError("searchIncludeTags", "must not overlap searchExcludeTags")
        : null) ??
      validateIntegerRange(next.cacheLimitGb, "cacheLimitGb", 1, 30) ??
      validateIntegerRange(next.concurrentImageRequests, "concurrentImageRequests", 1, 30) ??
      validateIntegerRange(next.requestStartIntervalMs, "requestStartIntervalMs", 0, 5_000);
    if (invalid) return invalid;
    this.settings = {
      ...next,
      collapsedGroupKeys: [...new Set(next.collapsedGroupKeys.map((key) => key.trim()))]
        .sort((left, right) => left.localeCompare(right)),
      searchIncludeTags: searchIncludeTags!,
      searchExcludeTags: searchExcludeTags!,
      revision: this.settings.revision + 1,
    };
    persistBrowserSettings(this.settings);
    this.emit("settings:changed", { ...this.settings });
    return ok({ ...this.settings });
  }

  async storageUsageGet(): Promise<ApiResult<StorageUsageSnapshot>> {
    const downloadRoot = this.settings.downloadRoot.trim();
    const downloadBytes = downloadRoot ? 37 * 1024 * 1024 * 1024 : 0;
    const downloadVolumeRoot = /^([a-z]):[\\/]/i.exec(downloadRoot)?.[1]?.toUpperCase();
    const onDataVolume = downloadVolumeRoot === "C";
    const appDataDiskBytes = 170 * 1024 * 1024;
    const volumes: StorageUsageSnapshot["volumes"] = [{
      root: "C:\\",
      totalBytes: 512 * 1024 * 1024 * 1024,
      availableBytes: 211 * 1024 * 1024 * 1024,
      atsumiBytes: appDataDiskBytes + (onDataVolume ? downloadBytes : 0),
    }];
    if (downloadRoot && !onDataVolume) {
      volumes.push({
        root: downloadVolumeRoot ? `${downloadVolumeRoot}:\\` : "다운로드 볼륨",
        totalBytes: 2 * 1024 * 1024 * 1024 * 1024,
        availableBytes: 1_163 * 1024 * 1024 * 1024,
        atsumiBytes: downloadBytes,
      });
    }
    return ok({
      memoryCacheBytes: 18 * 1024 * 1024,
      diskCache: {
        bytes: 6 * 1024 * 1024,
        exists: true,
        scanComplete: true,
        volumeRoot: "C:\\",
      },
      appData: {
        bytes: 164 * 1024 * 1024,
        exists: true,
        scanComplete: true,
        volumeRoot: "C:\\",
      },
      downloads: {
        bytes: downloadBytes,
        exists: Boolean(downloadRoot),
        scanComplete: true,
        ...(downloadRoot ? { volumeRoot: downloadVolumeRoot ? `${downloadVolumeRoot}:\\` : "다운로드 볼륨" } : {}),
      },
      volumes,
      warnings: [],
    });
  }

  async danbooruSearch(request: DanbooruSearchRequest): Promise<ApiResult<DanbooruSearchPage>> {
    if (!Number.isInteger(request.page) || request.page < 1 || request.page > 1_000) {
      return validationError("page", "must be between 1 and 1000");
    }
    if (!Number.isInteger(request.pageSize) || request.pageSize < 1 || request.pageSize > 100) {
      return validationError("pageSize", "must be between 1 and 100");
    }
    const tags = request.tags.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (browserDanbooruLimitedTermCount(tags) > 2) {
      return {
        ok: false,
        error: {
          code: "DANBOORU_TAG_LIMIT",
          message: "Danbooru 비로그인 검색은 제한 대상 조건을 최대 2개까지 사용할 수 있습니다.",
          retryable: false,
          action: "none",
        },
      };
    }
    const numeric = /^\d+$/.test(request.tags.trim()) ? Number(request.tags.trim()) : null;
    const filtered = numeric === null
      ? filterAndSortBrowserDanbooruPosts(tags)
      : browserDanbooruPosts.filter((post) => post.id === numeric);
    const offset = (request.page - 1) * request.pageSize;
    return ok({
      items: filtered.slice(offset, offset + request.pageSize).map((post) => ({ ...post })),
      page: request.page,
      hasMore: offset + request.pageSize < filtered.length,
    });
  }

  async danbooruRandom(): Promise<ApiResult<DanbooruPost>> {
    const index = Math.floor(Math.random() * browserDanbooruPosts.length);
    return ok({ ...browserDanbooruPosts[index]! });
  }

  async danbooruRelated(request: DanbooruRelatedRequest): Promise<ApiResult<DanbooruRelatedPosts>> {
    const current = browserDanbooruPosts.find((post) => post.id === request.postId);
    if (!current) return notFoundError("DANBOORU_POST_NOT_FOUND", "해당 Danbooru post를 찾을 수 없습니다.");
    const parentId = request.parentId ?? current.parentId;
    const parent = parentId === undefined
      ? undefined
      : browserDanbooruPosts.find((post) => post.id === parentId);
    const clone = (post: DanbooruPost) => ({ ...post });
    const siblings = parentId === undefined
      ? []
      : browserDanbooruPosts.filter((post) => post.parentId === parentId && post.id !== request.postId).map(clone);
    const children = (request.hasChildren || current.hasChildren)
      ? browserDanbooruPosts.filter((post) => post.parentId === request.postId).map(clone)
      : [];
    const pools = browserDanbooruPools.flatMap((pool) => {
      const currentIndex = pool.postIds.indexOf(request.postId);
      if (currentIndex < 0) return [];
      const start = Math.min(Math.max(0, currentIndex - 4), Math.max(0, pool.postIds.length - 9));
      const selectedIds = new Set(pool.postIds.slice(start, start + 9));
      return [{
        id: pool.id,
        name: pool.name,
        category: pool.category,
        postCount: pool.postIds.length,
        currentIndex,
        items: pool.postIds
          .filter((id) => selectedIds.has(id))
          .map((id) => browserDanbooruPosts.find((post) => post.id === id))
          .filter((post): post is DanbooruPost => Boolean(post))
          .map(clone),
      }];
    });
    return ok({
      ...(parent ? { parent: clone(parent) } : {}),
      siblings,
      children,
      pools,
    });
  }

  async danbooruAutocomplete(query: string, limit: number): Promise<ApiResult<DanbooruAutocompleteItem[]>> {
    const normalized = query.trim().toLowerCase();
    if (normalized.length < 2) return ok([]);
    const tags = [...new Set(browserDanbooruPosts.flatMap((post) => [
      ...post.artists,
      ...post.copyrights,
      ...post.characters,
      ...post.tags,
    ]))];
    return ok(tags
      .filter((tag) => tag.includes(normalized))
      .slice(0, Math.max(1, Math.min(10, limit)))
      .map((tag, index) => ({ label: tag, value: tag, category: index % 5, postCount: 1000 - index })));
  }

  async danbooruDownload(postId: number): Promise<ApiResult<DanbooruDownloadRecord>> {
    const existing = this.danbooruDownloads.get(postId);
    if (existing) return ok({ ...existing, post: { ...existing.post } });
    const post = browserDanbooruPosts.find((candidate) => candidate.id === postId);
    if (!post) return notFoundError("DANBOORU_POST_NOT_FOUND", "해당 Danbooru post를 찾을 수 없습니다.");
    const record: DanbooruDownloadRecord = {
      post: { ...post },
      fileName: `${post.id}.${post.fileExt}`,
      downloadedAt: String(Date.now()),
      bytes: post.fileSize,
    };
    this.danbooruDownloads.set(post.id, record);
    return ok({ ...record, post: { ...record.post } });
  }

  async danbooruDownloadsList(request: DanbooruDownloadsRequest): Promise<ApiResult<DanbooruDownloadsPage>> {
    const query = request.query.trim().toLowerCase();
    const records = [...this.danbooruDownloads.values()]
      .filter((record) => !query || [
        String(record.post.id),
        ...record.post.artists,
        ...record.post.copyrights,
        ...record.post.characters,
        ...record.post.tags,
      ].some((value) => value.toLowerCase().includes(query)))
      .sort((left, right) => Number(right.downloadedAt) - Number(left.downloadedAt));
    const totalPages = Math.max(1, Math.ceil(records.length / request.pageSize));
    const page = Math.max(1, Math.min(totalPages, request.page));
    const offset = (page - 1) * request.pageSize;
    return ok({ items: records.slice(offset, offset + request.pageSize), page, total: records.length, totalPages });
  }

  async tagCatalogStatus(): Promise<ApiResult<TagCatalogStatus>> { return ok({ ...this.tagCatalogStatusValue }); }
  async tagCatalogRefresh(): Promise<ApiResult<TagCatalogStatus>> {
    this.tagCatalogStatusValue = { ...this.tagCatalogStatusValue, revision: this.tagCatalogStatusValue.revision + 1, lastAttemptAt: new Date().toISOString(), lastSuccessAt: new Date().toISOString(), lastErrorCode: undefined, lastErrorMessage: undefined };
    return ok({ ...this.tagCatalogStatusValue });
  }
  async tagSuggestionsSearch(request: TagSuggestionRequest): Promise<ApiResult<TagSuggestion[]>> {
    const query = normalizeSuggestionValue(request.query);
    if (query.length < 2) return ok([]);
    const favorites = new Set([...this.favorites.values()].map((value) =>
      `${value.namespace}\u0000${normalizeSuggestionValue(value.value)}`));
    return ok(this.tagCatalog.filter((entry) => (!request.namespace || entry.namespace === request.namespace) && entry.name.includes(query)).map((entry) => ({ ...entry, favorite: favorites.has(suggestionFavoriteKey(entry.namespace, entry.name)) })).sort((a, b) => Number(b.favorite) - Number(a.favorite) || b.galleryCount - a.galleryCount || a.name.localeCompare(b.name) || namespaceRank(a.namespace) - namespaceRank(b.namespace) || a.token.localeCompare(b.token)).slice(0, Math.min(8, request.limit)));
  }

  async folderNameTemplatePreview(template: string): Promise<ApiResult<string>> {
    const invalid = validateFolderNameTemplate(template);
    if (invalid) return invalid;
    const preview = browserFolderPreviewFixtures.get(template);
    return preview === undefined
      ? validationError(
        "folderNameTemplate",
        "브라우저 검토 모드에는 이 템플릿의 Rust 미리보기 fixture가 없습니다.",
      )
      : ok(preview);
  }

  async windowPlacementGet(): Promise<ApiResult<WindowPlacementSnapshot>> {
    return ok({ ...this.placement });
  }

  async windowPlacementUpdate(
    placement: WindowPlacement,
    expectedRevision: number,
  ): Promise<ApiResult<WindowPlacementSnapshot>> {
    if (expectedRevision !== this.placement.revision) return conflict("창 위치");
    this.placement = { ...placement, revision: this.placement.revision + 1 };
    return ok({ ...this.placement });
  }

  async searchSubmit(request: SearchRequest): Promise<ApiResult<SearchSubmission>> {
    const invalid = searchRequestValidationError(request);
    if (invalid) return validationError(invalid.field, invalid.reason);

    const effectiveRequest = applyGlobalSearchRules(
      normalizeSearchRequest(request),
      this.settings.searchIncludeTags,
      this.settings.searchExcludeTags,
    );
    const queryId = searchFixtureQueryId(effectiveRequest);
    const fixture = runSearchFixture(effectiveRequest);
    this.searchQueries.set(queryId, fixture);
    this.recordSearchHistory(request);
    const firstPage = searchFixturePage(fixture, 1);
    return ok({ queryId, firstPage });
  }

  async searchPageGet(queryId: string, page: number, requestId: string): Promise<ApiResult<GalleryPage>> {
    const normalizedQueryId = queryId.trim();
    const normalizedRequestId = requestId.trim();
    if (!normalizedRequestId || new TextEncoder().encode(normalizedRequestId).length > 200) {
      return validationError("requestId", "must contain between 1 and 200 bytes");
    }
    if (!normalizedQueryId) return validationError("queryId", "must not be empty");
    if (new TextEncoder().encode(normalizedQueryId).length > 200) {
      return validationError("queryId", "must be at most 200 bytes");
    }
    if (!Number.isInteger(page) || page < 1) return validationError("page", "must be one-based");
    const fixture = this.searchQueries.get(normalizedQueryId);
    if (!fixture) {
      return notFoundError(
        "QUERY_NOT_FOUND",
        "The search query is no longer available; submit it again",
        { queryId: normalizedQueryId },
      );
    }
    const pageResult = searchFixturePage(fixture, page);
    if ((pageResult.totalPages === 0 && page !== 1) || (pageResult.totalPages > 0 && page > pageResult.totalPages)) {
      return validationError("page", "must not exceed the available search pages");
    }
    return ok(pageResult);
  }

  async searchPageCancel(requestId: string): Promise<ApiResult<boolean>> {
    const normalizedRequestId = requestId.trim();
    if (!normalizedRequestId || new TextEncoder().encode(normalizedRequestId).length > 200) {
      return validationError("requestId", "must contain between 1 and 200 bytes");
    }
    return ok(true);
  }

  async galleryDetailGet(galleryId: GalleryDetail["id"]): Promise<ApiResult<GalleryDetail>> {
    if (!Number.isInteger(galleryId) || galleryId <= 0) {
      return validationError("galleryId", "must be a positive integer");
    }
    const detail = galleryDetailFixture(galleryId);
    return detail
      ? ok(detail)
      : notFoundError(
        "SOURCE_NOT_FOUND",
        "The gallery could not be found in the current source",
        { galleryId },
      );
  }

  async favoritesList(): Promise<ApiResult<FavoriteRecord[]>> {
    return ok([...this.favorites.values()]
      .sort((left, right) => left.namespace.localeCompare(right.namespace) || left.value.localeCompare(right.value))
      .map(cloneFavorite));
  }

  async favoriteSet(
    key: FavoriteKey,
    enabled: boolean,
  ): Promise<ApiResult<FavoriteMutationResult>> {
    const value = key.value.trim().toLocaleLowerCase().split(/\s+/).join(" ");
    if (!value) return validationError("favorite.value", "must not be empty");
    if (new TextEncoder().encode(value).length > 200) {
      return validationError("favorite.value", "must be at most 200 bytes");
    }
    if (/[\u0000-\u001f\u007f]/.test(value)) {
      return validationError("favorite.value", "must not contain control characters");
    }
    const normalized: FavoriteKey = { namespace: key.namespace, value };
    const mapKey = `${normalized.namespace}:${normalized.value}`;
    if (!enabled) {
      this.favorites.delete(mapKey);
      return ok({ enabled: false });
    }

    const current = this.favorites.get(mapKey);
    const now = new Date().toISOString();
    const favorite: FavoriteRecord = current
      ? { ...current, revision: current.revision + 1, updatedAt: now }
      : { ...normalized, revision: 0, createdAt: now, updatedAt: now };
    this.favorites.set(mapKey, favorite);
    return ok({ enabled: true, favorite: cloneFavorite(favorite) });
  }

  async searchHistoryList(limit: number): Promise<ApiResult<SearchHistoryEntry[]>> {
    if (!Number.isInteger(limit) || limit < 1 || limit > 100) {
      return validationError("limit", "must be between 1 and 100");
    }
    return ok([...this.searchHistory.values()]
      .sort((left, right) => right.lastUsedAt.localeCompare(left.lastUsedAt) || right.historyId - left.historyId)
      .slice(0, limit)
      .map(cloneSearchHistory));
  }

  async autoFindSnapshot(): Promise<ApiResult<AutoFindSnapshot>> {
    return ok(cloneAutoFindSnapshot(this.autoFind));
  }

  async autoFindRefresh(): Promise<ApiResult<AutoFindRun>> {
    if (this.autoFind.run?.state === "running") return ok(cloneAutoFindRun(this.autoFind.run));

    const artists = [...this.favorites.values()].filter((favorite) => favorite.namespace === "artist");
    const now = new Date().toISOString();
    const generation = ++this.autoFindGeneration;
    const historyMode = this.settings.autoFindHistoryMode;
    const completedOwned = [...this.downloadEntries.values()]
      .filter((entry) => entry.state === "completed" || entry.state === "quarantined");
    const cutoffEvidence = artists.map((favorite) => {
      const ownedIds = runSearchFixture({
        text: `artist:${favorite.value}`,
        includeTags: [],
        excludeTags: [],
        languages: ["korean", "japanese", "chinese", "english"],
        sort: "recent",
        pageSize: 200,
      }).items
        .filter((gallery) => completedOwned.some((entry) => entry.galleryId === gallery.id))
        .map((gallery) => gallery.id)
        .sort((left, right) => left - right);
      return {
        artist: favorite.value,
        ...(ownedIds[0] !== undefined ? { oldestOwnedGalleryId: ownedIds[0] } : {}),
        qualifiedOwnedCount: ownedIds.length,
        source: "verified_owned_artifact" as const,
        policyVersion: 1 as const,
      };
    });
    const run: AutoFindRun = {
      runId: `browser-auto-find-${this.nextAutoFindRunId++}`,
      revision: 0,
      state: artists.length ? "running" : "completed",
      totalFavorites: artists.length,
      completedFavorites: 0,
      candidatesFound: 0,
      startedAt: now,
      updatedAt: now,
      historyMode,
      ...(!artists.length ? { finishedAt: now } : {}),
    };
    this.autoFind = { run, candidates: [], cutoffEvidence, truncations: [] };
    queueMicrotask(() => this.emit("auto-find:changed", cloneAutoFindRun(run)));

    artists.forEach((favorite, index) => {
      window.setTimeout(() => this.runAutoFindFixture(generation, favorite, index === artists.length - 1), 60 * (index + 1));
    });
    return ok(cloneAutoFindRun(run));
  }

  async autoFindCancel(): Promise<ApiResult<AutoFindRun>> {
    const current = this.autoFind.run;
    if (!current || current.state !== "running") {
      return {
        ok: false,
        error: {
          code: "AUTO_FIND_NOT_RUNNING",
          message: "실행 중인 자동 탐색이 없습니다.",
          retryable: false,
          action: "none",
        },
      };
    }
    this.autoFindGeneration += 1;
    const now = new Date().toISOString();
    const cancelled: AutoFindRun = {
      ...current,
      revision: current.revision + 1,
      state: "cancelled",
      updatedAt: now,
      finishedAt: now,
    };
    this.autoFind = { ...this.autoFind, run: cancelled };
    this.emit("auto-find:changed", cloneAutoFindRun(cancelled));
    return ok(cloneAutoFindRun(cancelled));
  }

  async autoFindExclude(
    galleryIds: GalleryId[],
    reason: string,
  ): Promise<ApiResult<AutoFindExclusionResult>> {
    if (!galleryIds.length) return validationError("galleryIds", "must not be empty");
    if (galleryIds.length > 200) return validationError("galleryIds", "must contain at most 200 IDs");
    if (galleryIds.some((galleryId) => !Number.isInteger(galleryId) || galleryId <= 0)) {
      return validationError("galleryIds", "gallery IDs must be positive integers");
    }
    const normalizedReason = reason.trim();
    if (!normalizedReason || new TextEncoder().encode(normalizedReason).length > 500) {
      return validationError("reason", "must contain between 1 and 500 bytes");
    }
    const normalizedIds = normalizedGallerySet(galleryIds);
    const createdAt = new Date().toISOString();
    normalizedIds.forEach((galleryId) => {
      const candidate = this.autoFind.candidates.find((item) => item.id === galleryId);
      if (candidate) this.excludedAutoFindCandidates.set(galleryId, candidate);
      this.explorationRestoredGalleryIds.delete(galleryId);
      this.autoFindExclusions.set(galleryId, {
        reason: normalizedReason,
        createdAt,
        title: candidate?.title ?? `Gallery #${galleryId}`,
        artist: candidate?.artist ?? "알 수 없는 작가",
      });
    });
    const excluded = new Set(normalizedIds);
    this.autoFind = {
      ...this.autoFind,
      candidates: this.autoFind.candidates.filter((candidate) => !excluded.has(candidate.id)),
    };
    return ok({
      excludedGalleryIds: normalizedIds,
      snapshot: cloneAutoFindSnapshot(this.autoFind),
    });
  }

  async explorationExclusionsList(): Promise<ApiResult<ExplorationExclusion[]>> {
    const grouped = new Map<GalleryId, ExplorationExclusion>();
    const addReason = (
      galleryId: GalleryId,
      title: string,
      artist: string | undefined,
      reason: ExplorationExclusion["reasons"][number],
    ) => {
      const current = grouped.get(galleryId) ?? {
        galleryId,
        title,
        artist: artist || "알 수 없는 작가",
        reasons: [],
      };
      current.reasons.push(reason);
      grouped.set(galleryId, current);
    };
    const hasDuplicateReason = (galleryId: GalleryId): boolean =>
      grouped.get(galleryId)?.reasons.some((reason) => reason.kind === "duplicate_hidden") ?? false;
    for (const [galleryId, exclusion] of this.autoFindExclusions) {
      addReason(galleryId, exclusion.title, exclusion.artist, {
        kind: "manual",
        detail: exclusion.reason,
        excludedAt: exclusion.createdAt,
      });
    }
    for (const [candidateId, review] of this.duplicateReviews) {
      if (!this.duplicateResolvedCandidates.has(candidateId)) continue;
      const hiddenInReview = new Set<GalleryId>();
      for (const decision of [...review.decisions].reverse()) {
        if (decision.action !== "hide_parent" && decision.action !== "hide_candidate") continue;
        const hidden = decision.action === "hide_parent"
          ? review.candidate.parent
          : review.candidate.candidate;
        if (hiddenInReview.has(hidden.galleryId)) continue;
        hiddenInReview.add(hidden.galleryId);
        if (!this.explorationRestoredGalleryIds.has(hidden.galleryId)) {
          addReason(hidden.galleryId, hidden.title, hidden.artist, {
            kind: "duplicate_hidden",
            detail: "중복 판정에서 숨김",
            excludedAt: decision.createdAt,
          });
        }
      }
    }
    for (const review of this.downloadOverlapReviews.values()) {
      for (const candidate of review.candidates) {
        const galleryId = candidate.existing.galleryId;
        if (candidate.decision !== "existing_removed"
          || !this.duplicateHiddenGalleryIds.has(galleryId)
          || this.explorationRestoredGalleryIds.has(galleryId)
          || hasDuplicateReason(galleryId)) {
          continue;
        }
        addReason(galleryId, candidate.existing.title, candidate.existing.artists.join(", "), {
          kind: "duplicate_hidden",
          detail: "다운로드 판본 검토에서 기존 앨범 제거",
          excludedAt: review.resolvedAt ?? review.updatedAt,
        });
      }
      const galleryId = review.incoming.galleryId;
      if (review.state !== "cancelled"
        || !this.duplicateHiddenGalleryIds.has(galleryId)
        || this.explorationRestoredGalleryIds.has(galleryId)
        || hasDuplicateReason(galleryId)) {
        continue;
      }
      addReason(galleryId, review.incoming.title, review.incoming.artists.join(", "), {
        kind: "duplicate_hidden",
        detail: "다운로드 판본 검토에서 신규 앨범 제거",
        excludedAt: review.resolvedAt ?? review.updatedAt,
      });
    }
    return ok([...grouped.values()]
      .map((item) => ({ ...item, reasons: item.reasons.map((reason) => ({ ...reason })) }))
      .sort((left, right) => right.galleryId - left.galleryId));
  }

  async explorationExclusionsRestore(
    galleryIds: GalleryId[],
  ): Promise<ApiResult<ExplorationExclusionRestoreResult>> {
    if (!galleryIds.length) return validationError("galleryIds", "must not be empty");
    if (galleryIds.length > 200) return validationError("galleryIds", "must contain at most 200 IDs");
    if (galleryIds.some((galleryId) => !Number.isInteger(galleryId) || galleryId <= 0)) {
      return validationError("galleryIds", "gallery IDs must be positive integers");
    }
    const restoredGalleryIds = normalizedGallerySet(galleryIds);
    for (const galleryId of restoredGalleryIds) {
      this.autoFindExclusions.delete(galleryId);
      this.explorationRestoredGalleryIds.add(galleryId);
      const candidate = this.excludedAutoFindCandidates.get(galleryId);
      if (candidate && !this.autoFind.candidates.some((item) => item.id === galleryId)) {
        this.autoFind = { ...this.autoFind, candidates: [...this.autoFind.candidates, candidate] };
      }
      this.excludedAutoFindCandidates.delete(galleryId);
    }
    return ok({ restoredGalleryIds, snapshot: cloneAutoFindSnapshot(this.autoFind) });
  }

  async duplicateSnapshot(): Promise<ApiResult<DuplicateSnapshot>> {
    return ok(cloneDuplicateSnapshot(this.duplicateSnapshotState));
  }

  async duplicateScanStart(): Promise<ApiResult<DuplicateScanRun>> {
    const current = this.duplicateSnapshotState.run;
    if (current?.state === "running") return ok(cloneDuplicateScanRun(current));

    const generation = ++this.duplicateGeneration;
    const now = new Date().toISOString();
    const run: DuplicateScanRun = {
      runId: `browser-duplicate-run-${this.nextDuplicateRunId++}`,
      revision: 0,
      state: "running",
      totalArtifacts: 2,
      hashedArtifacts: 0,
      totalPairs: 1,
      comparedPairs: 0,
      candidatesFound: 0,
      startedAt: now,
      updatedAt: now,
    };
    this.duplicateSnapshotState = {
      profile: { ...duplicateProfile },
      run,
      candidates: this.duplicateSnapshotState.candidates.map(cloneDuplicateCandidate),
    };
    queueMicrotask(() => this.emit("duplicate:changed", cloneDuplicateScanRun(run)));
    window.setTimeout(() => this.advanceDuplicateScanFixture(generation, false), 45);
    window.setTimeout(() => this.advanceDuplicateScanFixture(generation, true), 90);
    return ok(cloneDuplicateScanRun(run));
  }

  async duplicateScanCancel(): Promise<ApiResult<DuplicateScanRun>> {
    const current = this.duplicateSnapshotState.run;
    if (!current || current.state !== "running") {
      return {
        ok: false,
        error: {
          code: "DUPLICATE_SCAN_NOT_RUNNING",
          message: "실행 중인 작품 중복 검사가 없습니다.",
          retryable: false,
          action: "none",
        },
      };
    }
    this.duplicateGeneration += 1;
    const now = new Date().toISOString();
    const cancelled: DuplicateScanRun = {
      ...current,
      revision: current.revision + 1,
      state: "cancelled",
      updatedAt: now,
      finishedAt: now,
    };
    this.duplicateSnapshotState = { ...this.duplicateSnapshotState, run: cancelled };
    this.emit("duplicate:changed", cloneDuplicateScanRun(cancelled));
    return ok(cloneDuplicateScanRun(cancelled));
  }

  async duplicateReviewGet(candidateId: string): Promise<ApiResult<DuplicateReview>> {
    const normalizedId = candidateId.trim();
    const review = this.duplicateReviews.get(normalizedId);
    return review
      ? ok(cloneDuplicateReview(review))
      : notFoundError(
          "DUPLICATE_CANDIDATE_NOT_FOUND",
          "중복 후보를 찾을 수 없습니다.",
          { candidateId: normalizedId },
        );
  }

  async duplicateDecisionApply(
    request: DuplicateDecisionRequest,
  ): Promise<ApiResult<DuplicateReview>> {
    const candidateId = request.candidateId.trim();
    const review = this.duplicateReviews.get(candidateId);
    if (!review) {
      return notFoundError(
        "DUPLICATE_CANDIDATE_NOT_FOUND",
        "중복 후보를 찾을 수 없습니다.",
        { candidateId },
      );
    }
    if (request.expectedRevision !== review.candidate.revision) {
      return {
        ok: false,
        error: {
          code: "REVISION_CONFLICT",
          message: "중복 후보가 다른 창에서 변경되었습니다.",
          retryable: false,
          action: "review",
          details: {
            resource: "duplicateCandidate",
            expectedRevision: request.expectedRevision,
            actualRevision: review.candidate.revision,
          },
        },
      };
    }

    if (request.action === "series_link" && !request.seriesGroupId?.trim() && !request.seriesName?.trim()) {
      return validationError("request.seriesName", "seriesGroupId 또는 seriesName 중 하나가 필요합니다");
    }
    if (request.action === "series_unlink" && request.targetGalleryId === undefined) {
      return validationError("request.targetGalleryId", "series_unlink에는 대상 갤러리가 필요합니다");
    }

    const now = new Date().toISOString();
    const candidate = {
      ...review.candidate,
      revision: review.candidate.revision + 1,
      updatedAt: now,
    };
    let seriesGroups = review.seriesGroups.map((group) => ({
      ...group,
      members: group.members.map(cloneDuplicateGalleryRef),
    }));
    let appliedSeriesGroupId = request.seriesGroupId?.trim() || undefined;

    if (request.action === "series_link") {
      let group = appliedSeriesGroupId
        ? seriesGroups.find((item) => item.seriesGroupId === appliedSeriesGroupId)
        : undefined;
      if (!group) {
        appliedSeriesGroupId = `browser-series-${candidate.candidateId}-${seriesGroups.length + 1}`;
        group = {
          seriesGroupId: appliedSeriesGroupId,
          name: request.seriesName?.trim() || "연작",
          revision: 0,
          members: [],
          createdAt: now,
          updatedAt: now,
        };
        seriesGroups.push(group);
      }
      const memberIds = new Set(group.members.map((member) => member.galleryId));
      const additions = [candidate.parent, candidate.candidate]
        .filter((member) => !memberIds.has(member.galleryId))
        .map(cloneDuplicateGalleryRef);
      seriesGroups = seriesGroups.map((item) => item.seriesGroupId === group?.seriesGroupId
        ? {
            ...item,
            revision: item.revision + (additions.length ? 1 : 0),
            members: [...item.members, ...additions],
            updatedAt: now,
          }
        : item);
    }

    if (request.action === "series_unlink") {
      const target = request.targetGalleryId;
      seriesGroups = seriesGroups.map((group) => {
        if (request.seriesGroupId && group.seriesGroupId !== request.seriesGroupId) return group;
        const members = group.members.filter((member) => member.galleryId !== target);
        return members.length === group.members.length
          ? group
          : { ...group, revision: group.revision + 1, members, updatedAt: now };
      });
      appliedSeriesGroupId = request.seriesGroupId
        ?? seriesGroups.find((group) => group.members.some((member) => member.galleryId === target))?.seriesGroupId;
    }

    const decision = {
      decisionId: `browser-decision-${this.nextDuplicateDecisionId++}`,
      candidateId,
      candidateRevision: candidate.revision,
      action: request.action,
      ...(request.targetGalleryId !== undefined ? { targetGalleryId: request.targetGalleryId } : {}),
      ...(appliedSeriesGroupId ? { seriesGroupId: appliedSeriesGroupId } : {}),
      createdAt: now,
    };
    const nextReview: DuplicateReview = {
      ...review,
      candidate,
      decisions: [...review.decisions, decision],
      seriesGroups,
    };
    this.duplicateReviews.set(candidateId, nextReview);
    if (request.action === "hide_parent") {
      this.explorationRestoredGalleryIds.delete(candidate.parent.galleryId);
      this.duplicateHiddenGalleryIds.add(candidate.parent.galleryId);
    }
    if (request.action === "hide_candidate") {
      this.explorationRestoredGalleryIds.delete(candidate.candidate.galleryId);
      this.duplicateHiddenGalleryIds.add(candidate.candidate.galleryId);
    }
    if (["hide_parent", "hide_candidate", "exclude_pair"].includes(request.action)) {
      this.duplicateResolvedCandidates.add(candidateId);
      this.duplicateSnapshotState = {
        ...this.duplicateSnapshotState,
        candidates: this.duplicateSnapshotState.candidates.filter((item) => item.candidateId !== candidateId),
      };
    } else {
      this.duplicateSnapshotState = {
        ...this.duplicateSnapshotState,
        candidates: this.duplicateSnapshotState.candidates.map((item) =>
          item.candidateId === candidateId ? cloneDuplicateCandidate(candidate) : item),
      };
    }
    return ok(cloneDuplicateReview(nextReview));
  }

  async downloadOverlapReviewGet(reviewId: string): Promise<ApiResult<DownloadOverlapReview>> {
    const normalizedId = reviewId.trim();
    if (!normalizedId) return validationError("reviewId", "비어 있을 수 없습니다");
    const review = this.downloadOverlapReviews.get(normalizedId) ?? browserOverlapReview(normalizedId);
    this.downloadOverlapReviews.set(normalizedId, review);
    return ok(cloneDownloadOverlapReview(review));
  }

  async downloadOverlapDecisionApply(
    request: DownloadOverlapDecisionRequest,
  ): Promise<ApiResult<DownloadOverlapDecisionResult>> {
    if (request.actor === "automation") {
      if (!request.candidateId
        || (request.action !== "remove_existing_continue" && request.action !== "remove_incoming")
        || request.reasonCode !== "balanced_overlap_v2"
        || request.ruleVersion !== 2
        || !request.featureSnapshotJson) {
        return validationError("request", "자동 중복 판정 감사 정보가 올바르지 않습니다");
      }
      try {
        const snapshot = JSON.parse(request.featureSnapshotJson) as unknown;
        if (typeof snapshot !== "object" || snapshot === null || Array.isArray(snapshot)) {
          return validationError("request.featureSnapshotJson", "JSON 객체여야 합니다");
        }
      } catch {
        return validationError("request.featureSnapshotJson", "올바른 JSON이어야 합니다");
      }
    }
    const review = this.downloadOverlapReviews.get(request.reviewId);
    if (!review) {
      return notFoundError(
        "DOWNLOAD_OVERLAP_REVIEW_NOT_FOUND",
        "다운로드 판본 중복 검토를 찾을 수 없습니다.",
        { reviewId: request.reviewId },
      );
    }
    if (review.revision !== request.expectedRevision) return conflict("downloadOverlapReview");
    const candidateScoped = request.action !== "remove_incoming";
    const selectedCandidate = review.candidates.find((item) => item.candidateId === request.candidateId);
    if (candidateScoped && (!selectedCandidate || selectedCandidate.decision !== undefined)) {
      return validationError("request.candidateId", "검토에 포함된 후보를 선택해야 합니다");
    }
    const existingEntry = request.action === "remove_existing_continue" && selectedCandidate
      ? this.downloadEntries.get(selectedCandidate.existing.entryId)
      : undefined;
    const chainedReview = existingEntry?.state === "review_required" && existingEntry.reviewKind === "gallery_duplicate"
      && existingEntry.reviewId && existingEntry.reviewId !== request.reviewId
      ? this.downloadOverlapReviews.get(existingEntry.reviewId)
      : undefined;
    if (existingEntry?.state === "review_required"
      && (!chainedReview || chainedReview.state !== "pending" || chainedReview.entryId !== existingEntry.entryId)) {
      return validationError("request.candidateId", "기존 앨범 A의 검토 상태가 변경되었습니다");
    }
    const remaining = review.candidates.map((candidate) => {
      if (candidate.decision !== undefined) return candidate;
      if (candidate.candidateId !== request.candidateId
        && this.downloadEntries.get(candidate.existing.entryId)?.state === "quarantined") {
        return { ...candidate, decision: "existing_removed" as const };
      }
      if (candidate.candidateId !== request.candidateId) return candidate;
      if (request.action === "false_positive_continue") {
        return { ...candidate, decision: "false_positive" as const };
      }
      if (request.action === "keep_both_continue") {
        return { ...candidate, decision: "keep_both" as const };
      }
      if (request.action === "remove_existing_continue") {
        return { ...candidate, decision: "existing_removed" as const };
      }
      return candidate;
    });
    const pending = remaining.some((candidate) => candidate.decision === undefined);
    const cancelled = request.action === "remove_incoming";
    const next: DownloadOverlapReview = {
      ...review,
      revision: review.revision + 1,
      state: cancelled ? "cancelled" : pending ? "pending" : "resolved",
      candidates: remaining,
      updatedAt: new Date().toISOString(),
      ...(!pending ? { resolvedAt: new Date().toISOString() } : {}),
    };
    this.downloadOverlapReviews.set(request.reviewId, next);
    if (cancelled) {
      this.explorationRestoredGalleryIds.delete(review.incoming.galleryId);
      this.duplicateHiddenGalleryIds.add(review.incoming.galleryId);
    }
    if (request.action === "remove_existing_continue" && selectedCandidate && existingEntry) {
      this.explorationRestoredGalleryIds.delete(selectedCandidate.existing.galleryId);
      this.duplicateHiddenGalleryIds.add(selectedCandidate.existing.galleryId);
      if (existingEntry.state === "review_required" && chainedReview) {
        const now = new Date().toISOString();
        this.downloadOverlapReviews.set(chainedReview.reviewId, {
          ...chainedReview,
          revision: chainedReview.revision + 1,
          state: "cancelled",
          updatedAt: now,
          resolvedAt: now,
        });
        this.downloadEntries.set(existingEntry.entryId, {
          ...existingEntry,
          revision: existingEntry.revision + 1,
          state: "cancelled",
          reviewKind: undefined,
          reviewId: undefined,
        });
      } else if (existingEntry.state === "completed") {
        this.downloadEntries.set(existingEntry.entryId, {
          ...existingEntry,
          revision: existingEntry.revision + 1,
          state: "quarantined",
        });
      } else if (["failed", "interrupted", "cancelled"].includes(existingEntry.state)) {
        this.downloadEntries.set(existingEntry.entryId, {
          ...existingEntry,
          revision: existingEntry.revision + (existingEntry.state === "cancelled" ? 0 : 1),
          state: "cancelled",
          reviewKind: undefined,
          reviewId: undefined,
        });
      }
    }
    const entry = this.downloadEntries.get(review.entryId);
    if (entry && (cancelled || !pending)) {
      this.downloadEntries.set(review.entryId, {
        ...entry,
        revision: entry.revision + 1,
        state: cancelled ? "cancelled" : "queued",
        reviewKind: undefined,
        reviewId: undefined,
      });
    }
    return ok({ review: cloneDownloadOverlapReview(next), resumed: !cancelled && !pending, cancelled });
  }

  async internalDuplicateSnapshot(): Promise<ApiResult<InternalDuplicateSnapshot>> {
    return ok(cloneInternalSnapshot(this.internalSnapshotState));
  }

  async internalDuplicateActiveArtifact(): Promise<ApiResult<InternalArtifactScanProgress | null>> {
    return ok(this.internalArtifactProgress ? cloneInternalArtifactProgress(this.internalArtifactProgress) : null);
  }

  async internalDuplicateScanStart(request: InternalScanRequest): Promise<ApiResult<InternalScanRun>> {
    if (!request.entryIds.length) return validationError("entryIds", "must not be empty");
    if (request.entryIds.length > 200) return validationError("entryIds", "must contain at most 200 entries");
    const entryIds = [...new Set(request.entryIds.map((entryId) => entryId.trim()))];
    if (entryIds.some((entryId) => !entryId || new TextEncoder().encode(entryId).length > 200)) {
      return validationError("entryIds", "must contain non-empty IDs of at most 200 bytes");
    }
    entryIds.sort((left, right) => left.localeCompare(right));
    const entrySetKey = JSON.stringify(entryIds);
    const current = this.internalSnapshotState.run;
    if (current?.state === "running") {
      if (this.internalActiveEntrySetKey === entrySetKey) return ok(cloneInternalScanRun(current));
      return {
        ok: false,
        error: {
          code: "OPERATION_ACTIVE",
          message: "다른 선택 항목의 내부 중복 검사가 이미 실행 중입니다.",
          retryable: false,
          action: "none",
        },
      };
    }
    const generation = ++this.internalGeneration;
    this.internalActiveEntrySetKey = entrySetKey;
    const now = new Date().toISOString();
    const run: InternalScanRun = {
      runId: `browser-internal-run-${this.nextInternalRunId++}`,
      revision: 0,
      state: "running",
      totalArtifacts: entryIds.length,
      scannedArtifacts: 0,
      totalPages: 24 * entryIds.length,
      comparedPairs: 0,
      groupsFound: 0,
      algorithmVersion: 4,
      skippedArtifacts: 0,
      skippedPages: 0,
      startedAt: now,
      updatedAt: now,
    };
    const mockArtifactProgress = (
      entryId: string,
      artifactIndex: number,
      sequence: number,
      stage: InternalArtifactScanProgress["stage"],
    ): InternalArtifactScanProgress => ({
      runId: run.runId,
      sequence,
      entryId,
      galleryId: this.downloadEntries.get(entryId)?.galleryId ?? galleryId(4_051_038 + artifactIndex - 1),
      artifactIndex,
      totalArtifacts: entryIds.length,
      processedPages: stage === "hashing" ? 0 : 24,
      totalPages: 24,
      comparedPairs: stage === "hashing" ? 0 : stage === "comparing" ? 138 : 276,
      totalPairs: 276,
      progressPercent: stage === "hashing" ? 0 : stage === "comparing" ? 65 : 99,
      stage,
    });
    this.internalArtifactProgress = mockArtifactProgress(entryIds[0]!, 1, 1, "hashing");
    this.internalSnapshotState = { ...this.internalSnapshotState, run };
    queueMicrotask(() => {
      this.emit("internal-duplicate:changed", cloneInternalScanRun(run));
      if (this.internalArtifactProgress) {
        this.emit("internal-duplicate:artifact-progress", cloneInternalArtifactProgress(this.internalArtifactProgress));
      }
    });
    entryIds.forEach((entryId, entryIndex) => {
      const artifactIndex = entryIndex + 1;
      const baseSequence = entryIndex * 3 + 1;
      const baseDelay = entryIndex * 80;
      if (entryIndex > 0) {
        window.setTimeout(() => {
          if (generation !== this.internalGeneration || this.internalSnapshotState.run?.state !== "running") return;
          this.internalArtifactProgress = mockArtifactProgress(entryId, artifactIndex, baseSequence, "hashing");
          this.emit("internal-duplicate:artifact-progress", cloneInternalArtifactProgress(this.internalArtifactProgress));
        }, baseDelay);
      }
      window.setTimeout(() => {
        if (generation !== this.internalGeneration || this.internalSnapshotState.run?.state !== "running") return;
        this.internalArtifactProgress = mockArtifactProgress(entryId, artifactIndex, baseSequence + 1, "comparing");
        this.emit("internal-duplicate:artifact-progress", cloneInternalArtifactProgress(this.internalArtifactProgress));
      }, baseDelay + 30);
      window.setTimeout(() => {
        if (generation !== this.internalGeneration || this.internalSnapshotState.run?.state !== "running") return;
        this.internalArtifactProgress = mockArtifactProgress(entryId, artifactIndex, baseSequence + 2, "finalizing");
        this.emit("internal-duplicate:artifact-progress", cloneInternalArtifactProgress(this.internalArtifactProgress));
      }, baseDelay + 55);
    });
    window.setTimeout(() => {
      if (generation !== this.internalGeneration || this.internalSnapshotState.run?.state !== "running") return;
      const finishedAt = new Date().toISOString();
      const groups: InternalDuplicateSnapshot["groups"] = entryIds.flatMap((entryId, targetIndex) => {
        const targetGalleryId = this.downloadEntries.get(entryId)?.galleryId ?? galleryId(4_051_038 + targetIndex);
        return [
          {
            groupId: `${entryId}-exact-1`, blockId: `${entryId}-block-1`, sequenceIndex: 0,
            revision: 0, entryId, galleryId: targetGalleryId, relation: "exact" as const, confidence: 1,
            recommendedKeepSourcePage: 2,
            pages: [2, 8].map((sourcePage) => ({ sourcePage, exactSha256: true, visualSimilarity: 1, detailHashDistance: 0, lowInformation: false })),
            resolved: false, createdAt: finishedAt, updatedAt: finishedAt,
          },
          {
            groupId: `${entryId}-visual-1`, blockId: `${entryId}-block-2`, sequenceIndex: 0,
            revision: 0, entryId, galleryId: targetGalleryId, relation: "translation_visual" as const, confidence: 0.94,
            recommendedKeepSourcePage: 14,
            pages: [14, 20].map((sourcePage) => ({ sourcePage, exactSha256: false, visualSimilarity: 0.94, detailHashDistance: 17, lowInformation: false })),
            resolved: false, createdAt: finishedAt, updatedAt: finishedAt,
          },
          {
            groupId: `${entryId}-visual-2`, blockId: `${entryId}-block-2`, sequenceIndex: 1,
            revision: 0, entryId, galleryId: targetGalleryId, relation: "translation_visual" as const, confidence: 0.91,
            recommendedKeepSourcePage: 15,
            pages: [15, 21].map((sourcePage) => ({ sourcePage, exactSha256: false, visualSimilarity: 0.91, detailHashDistance: 22, lowInformation: false })),
            resolved: false, createdAt: finishedAt, updatedAt: finishedAt,
          },
        ];
      });
      const selected = new Set(entryIds);
      const preservedGroups = this.internalSnapshotState.groups.filter((group) => !selected.has(group.entryId));
      const finished: InternalScanRun = {
        ...run,
        revision: 2,
        state: "completed",
        scannedArtifacts: entryIds.length,
        comparedPairs: 276 * entryIds.length,
        groupsFound: groups.length,
        updatedAt: finishedAt,
        finishedAt,
      };
      this.internalActiveEntrySetKey = null;
      this.internalArtifactProgress = null;
      this.internalSnapshotState = { ...this.internalSnapshotState, run: finished, groups: [...preservedGroups, ...groups] };
      this.emit("internal-duplicate:changed", cloneInternalScanRun(finished));
    }, entryIds.length * 80);
    return ok(cloneInternalScanRun(run));
  }

  async internalDuplicateScanCancel(): Promise<ApiResult<InternalScanRun>> {
    const current = this.internalSnapshotState.run;
    if (!current || current.state !== "running") {
      return notFoundError("INTERNAL_DUPLICATE_SCAN_NOT_RUNNING", "실행 중인 내부 중복 검사가 없습니다.");
    }
    this.internalGeneration += 1;
    this.internalActiveEntrySetKey = null;
    this.internalArtifactProgress = null;
    const now = new Date().toISOString();
    const cancelled = { ...current, revision: current.revision + 1, state: "cancelled" as const, updatedAt: now, finishedAt: now };
    this.internalSnapshotState = { ...this.internalSnapshotState, run: cancelled };
    this.emit("internal-duplicate:changed", cloneInternalScanRun(cancelled));
    return ok(cloneInternalScanRun(cancelled));
  }

  async internalDuplicateReviewGet(entryId: string): Promise<ApiResult<InternalDuplicateReview>> {
    const normalized = entryId.trim();
    const groups = this.internalSnapshotState.groups.filter((group) => group.entryId === normalized && !group.resolved);
    const records = this.internalSnapshotState.quarantineRecords.filter((record) => record.entryId === normalized);
    const gallery = [...this.downloadEntries.values()].find((entry) => entry.entryId === normalized);
    const targetGalleryId = groups[0]?.galleryId ?? gallery?.galleryId;
    if (!targetGalleryId && !records.length) {
      return notFoundError("INTERNAL_DUPLICATE_ENTRY_NOT_FOUND", "내부 중복 검토 항목을 찾을 수 없습니다.", { entryId: normalized });
    }
    const resolvedGalleryId = targetGalleryId ?? records[0]?.galleryId;
    if (!resolvedGalleryId) {
      return notFoundError("INTERNAL_DUPLICATE_ENTRY_NOT_FOUND", "내부 중복 검토 항목을 찾을 수 없습니다.", { entryId: normalized });
    }
    const detail = targetGalleryId ? galleryDetailFixture(targetGalleryId) : undefined;
    return ok(cloneInternalReview({
      entryId: normalized,
      galleryId: resolvedGalleryId,
      title: detail?.title ?? `다운로드 ${normalized}`,
      groups,
      quarantineRecords: records,
    }));
  }

  async internalRemovalPlan(request: InternalRemovalPlanRequest): Promise<ApiResult<InternalRemovalPlan>> {
    if (!request.selections.length) return validationError("request.selections", "must not be empty");
    for (const selection of request.selections) {
      const group = this.internalSnapshotState.groups.find((item) => item.groupId === selection.groupId && item.entryId === request.entryId);
      if (!group) return notFoundError("INTERNAL_DUPLICATE_ENTRY_NOT_FOUND", "내부 중복 그룹을 찾을 수 없습니다.");
      if (group.revision !== selection.expectedRevision) return conflict("internalDuplicateGroup");
      const pages = new Set(group.pages.map((page) => page.sourcePage));
      if (!pages.has(selection.keepSourcePage) || !selection.removeSourcePages.length || selection.removeSourcePages.some((page) => !pages.has(page) || page === selection.keepSourcePage)) {
        return validationError("request.selections", "keep/remove 페이지가 검토 행과 일치해야 합니다");
      }
    }
    const files = new Set(request.selections.flatMap((selection) => selection.removeSourcePages));
    const plan: InternalRemovalPlan = {
      ...request,
      selections: request.selections.map((selection) => ({ ...selection, removeSourcePages: [...selection.removeSourcePages] })),
      planId: `browser-internal-plan-${this.nextInternalPlanId++}`,
      filesToQuarantine: files.size,
      bytesToQuarantine: files.size * 512_000,
      expiresAt: String(Date.now() + 15 * 60 * 1_000),
    };
    this.internalPlans.set(plan.planId, plan);
    return ok({ ...plan, selections: plan.selections.map((selection) => ({ ...selection, removeSourcePages: [...selection.removeSourcePages] })) });
  }

  async internalRemovalApply(request: InternalRemovalApplyRequest): Promise<ApiResult<InternalRemovalResult>> {
    const plan = this.internalPlans.get(request.plan.planId);
    if (!plan || Number(plan.expiresAt) < Date.now()) {
      return notFoundError("INTERNAL_REMOVAL_PLAN_INVALID", "제거 계획이 만료되었거나 존재하지 않습니다.");
    }
    const now = new Date().toISOString();
    const records = plan.selections.flatMap((selection) => selection.removeSourcePages.map((sourcePage) => ({
      recordId: `browser-page-quarantine-${plan.planId}-${sourcePage}`,
      planId: plan.planId,
      entryId: plan.entryId,
      galleryId: this.internalSnapshotState.groups.find((group) => group.groupId === selection.groupId)?.galleryId ?? galleryId(1),
      sourcePage,
      originalRelativePath: `browser/${plan.entryId}/${String(sourcePage).padStart(4, "0")}.webp`,
      quarantineRelativePath: `browser/${plan.entryId}/.atsumi-page-quarantine/${plan.planId}/${String(sourcePage).padStart(4, "0")}.webp`,
      reason: request.reason.trim() || "internal duplicate review",
      state: "quarantined" as const,
      createdAt: now,
      updatedAt: now,
    })));
    const selected = new Set(plan.selections.map((selection) => selection.groupId));
    this.internalSnapshotState = {
      ...this.internalSnapshotState,
      groups: this.internalSnapshotState.groups.map((group) => selected.has(group.groupId) ? { ...group, revision: group.revision + 1, resolved: true, updatedAt: now } : group),
      quarantineRecords: [...this.internalSnapshotState.quarantineRecords, ...records],
    };
    const reviewResult = await this.internalDuplicateReviewGet(plan.entryId);
    if (!reviewResult.ok) return reviewResult;
    return ok({ review: reviewResult.data, records: records.map((record) => ({ ...record })) });
  }

  async internalRemovalUndo(request: InternalRemovalUndoRequest): Promise<ApiResult<InternalRemovalResult>> {
    if (!request.recordIds.length) return validationError("request.recordIds", "must not be empty");
    const requested = new Set(request.recordIds);
    const records = this.internalSnapshotState.quarantineRecords.filter((record) => requested.has(record.recordId) && record.state === "quarantined");
    if (!records.length) return notFoundError("INTERNAL_REMOVAL_PLAN_INVALID", "되돌릴 격리 기록을 찾을 수 없습니다.");
    const firstRecord = records[0];
    if (!firstRecord) return notFoundError("INTERNAL_REMOVAL_PLAN_INVALID", "되돌릴 격리 기록을 찾을 수 없습니다.");
    const entryId = firstRecord.entryId;
    if (records.some((record) => record.entryId !== entryId)) return validationError("request.recordIds", "한 다운로드의 페이지만 선택해야 합니다");
    const plans = new Set(records.map((record) => record.planId));
    const groupIds = new Set([...plans].flatMap((planId) => this.internalPlans.get(planId)?.selections.map((selection) => selection.groupId) ?? []));
    const now = new Date().toISOString();
    this.internalSnapshotState = {
      ...this.internalSnapshotState,
      groups: this.internalSnapshotState.groups.map((group) => groupIds.has(group.groupId) ? { ...group, revision: group.revision + 1, resolved: false, updatedAt: now } : group),
      quarantineRecords: this.internalSnapshotState.quarantineRecords.map((record) => requested.has(record.recordId) ? { ...record, state: "restored" as const, updatedAt: now } : record),
    };
    const reviewResult = await this.internalDuplicateReviewGet(entryId);
    if (!reviewResult.ok) return reviewResult;
    return ok({ review: reviewResult.data, records: reviewResult.data.quarantineRecords.filter((record) => requested.has(record.recordId)) });
  }

  async downloadQueueAdd(
    galleries: GalleryId[],
    requestId: string,
  ): Promise<ApiResult<DownloadEntry[]>> {
    const normalizedRequestId = requestId.trim();
    if (!normalizedRequestId) return validationError("requestId", "must not be empty");
    if (new TextEncoder().encode(normalizedRequestId).length > 200) {
      return validationError("requestId", "must be at most 200 bytes");
    }
    if (!galleries.length) return validationError("galleries", "must not be empty");
    if (galleries.length > 200) {
      return validationError("galleries", "must contain at most 200 IDs");
    }
    const invalidGallery = galleries.find((galleryId) => !Number.isInteger(galleryId) || galleryId <= 0);
    if (invalidGallery !== undefined) {
      return validationError("galleries", "gallery IDs must be positive integers");
    }

    const normalizedGalleries = normalizedGallerySet(galleries);
    const normalizedSetKey = gallerySetKey(normalizedGalleries);
    const replay = this.downloadQueueRequests.get(normalizedRequestId);
    if (replay) {
      if (replay.gallerySetKey !== normalizedSetKey) {
        return {
          ok: false,
          error: {
            code: "IDEMPOTENCY_CONFLICT",
            message: "The request ID was already used for a different gallery set",
            retryable: false,
            action: "review",
            details: { requestId: normalizedRequestId },
          },
        };
      }
      return ok(replay.entries.map(cloneDownloadEntry));
    }

    const entries = normalizedGalleries.map((galleryId) => {
      const activeEntryId = this.activeDownloadEntryByGallery.get(galleryId);
      const activeEntry = activeEntryId === undefined ? undefined : this.downloadEntries.get(activeEntryId);
      if (activeEntry && activeDownloadStates.has(activeEntry.state)) return cloneDownloadEntry(activeEntry);
      if (activeEntryId !== undefined) this.activeDownloadEntryByGallery.delete(galleryId);

      const entry: DownloadEntry = {
        entryId: `browser-entry-${galleryId}-${this.nextDownloadEntryId++}`,
        galleryId,
        revision: 0,
        state: "queued",
        progress: 0,
        attempt: 1,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
      };
      this.downloadEntries.set(entry.entryId, entry);
      this.activeDownloadEntryByGallery.set(galleryId, entry.entryId);
      if (this.listeners["download:changed"].size > 0) this.runFixtureDownload(entry.entryId, 1);
      return cloneDownloadEntry(entry);
    });
    this.downloadQueueRequests.set(normalizedRequestId, {
      gallerySetKey: normalizedSetKey,
      entries: entries.map(cloneDownloadEntry),
    });
    return ok(entries.map(cloneDownloadEntry));
  }

  async downloadEntriesList(request: DownloadListRequest): Promise<ApiResult<DownloadPage>> {
    if (!Number.isInteger(request.page) || request.page < 1) {
      return validationError("page", "must be one-based");
    }
    if (!Number.isInteger(request.pageSize) || request.pageSize < 1 || request.pageSize > 200) {
      return validationError("pageSize", "must be between 1 and 200");
    }
    const query = request.query?.trim().toLowerCase() ?? "";
    if (new TextEncoder().encode(query).length > 500) {
      return validationError("query", "must be at most 500 bytes");
    }
    const entries = [...this.downloadEntries.values()]
      .filter((entry) => entry.state !== "cancelled"
        || ![...this.downloadOverlapReviews.values()].some((review) => (
          (review.entryId === entry.entryId && review.state === "cancelled")
          || review.candidates.some((candidate) => (
            candidate.existing.entryId === entry.entryId && candidate.decision === "existing_removed"
          ))
        )))
      .filter((entry) => !this.duplicateHiddenGalleryIds.has(entry.galleryId)
        || this.explorationRestoredGalleryIds.has(entry.galleryId))
      .filter((entry) => request.state === undefined || entry.state === request.state)
      .filter((entry) => {
        if (!query) return true;
        return `${entry.entryId} ${entry.galleryId}`.toLowerCase().includes(query);
      })
      .sort((left, right) => left.galleryId - right.galleryId || left.entryId.localeCompare(right.entryId));
    const offset = (request.page - 1) * request.pageSize;
    return ok({
      page: request.page,
      totalItems: entries.length,
      entries: entries.slice(offset, offset + request.pageSize).map(cloneDownloadEntry),
    });
  }

  async downloadLibraryPageList(request: DownloadListRequest): Promise<ApiResult<DownloadLibraryPage>> {
    if (!Number.isInteger(request.page) || request.page < 1) {
      return validationError("page", "must be one-based");
    }
    if (!Number.isInteger(request.pageSize) || request.pageSize < 1 || request.pageSize > 200) {
      return validationError("pageSize", "must be between 1 and 200");
    }
    const query = request.query?.trim().toLowerCase() ?? "";
    if (new TextEncoder().encode(query).length > 500) {
      return validationError("query", "must be at most 500 bytes");
    }
    const visibleEntries = [...this.downloadEntries.values()]
      .filter((entry) => entry.state !== "cancelled"
        || ![...this.downloadOverlapReviews.values()].some((review) => (
          (review.entryId === entry.entryId && review.state === "cancelled")
          || review.candidates.some((candidate) => (
            candidate.existing.entryId === entry.entryId && candidate.decision === "existing_removed"
          ))
        )))
      .filter((entry) => !this.duplicateHiddenGalleryIds.has(entry.galleryId)
        || this.explorationRestoredGalleryIds.has(entry.galleryId));
    const canonicalByGallery = new Map<GalleryId, DownloadEntry>();
    for (const entry of visibleEntries) {
      const current = canonicalByGallery.get(entry.galleryId);
      if (!current || compareDownloadLibraryRecency(entry, current) > 0) {
        canonicalByGallery.set(entry.galleryId, entry);
      }
    }
    const items = [...canonicalByGallery.values()]
      .filter((entry) => request.state === undefined || entry.state === request.state)
      .map((download) => ({ gallery: this.localDownloadGallery(download.galleryId), download }))
      .filter(({ gallery, download }) => {
        if (!query) return true;
        return [
          download.entryId,
          String(download.galleryId),
          gallery.title ?? "",
          gallery.artist ?? "",
          gallery.group ?? "",
        ].join(" ").toLowerCase().includes(query);
      })
      .sort((left, right) => downloadLibraryDisplayTime(right.download)
        .localeCompare(downloadLibraryDisplayTime(left.download))
        || right.gallery.id - left.gallery.id
        || right.download.entryId.localeCompare(left.download.entryId));
    const offset = (request.page - 1) * request.pageSize;
    return ok({
      page: request.page,
      totalItems: items.length,
      items: items.slice(offset, offset + request.pageSize).map(({ gallery, download }) => ({
        gallery: { ...gallery },
        download: cloneDownloadEntry(download),
      })),
    });
  }

  private localDownloadGallery(id: GalleryId): DownloadLibraryGallery {
    const summary = galleryDetailFixture(id)
      ?? this.autoFind.candidates.find((candidate) => candidate.id === id)
      ?? [...this.searchQueries.values()].flatMap((result) => result.items).find((item) => item.id === id);
    if (!summary) return { id };
    return {
      id,
      title: summary.title,
      artist: summary.artist,
      ...(summary.group ? { group: summary.group } : {}),
      pages: summary.pages,
      language: summary.language,
      publishedRank: summary.publishedRank,
    };
  }

  async downloadRetry(entryIds: string[]): Promise<ApiResult<JobRef[]>> {
    const normalized = [...new Set(entryIds.map((entryId) => entryId.trim()))];
    if (!normalized.length || normalized.some((entryId) => !entryId)) {
      return validationError("entryIds", "must contain at least one non-empty entry ID");
    }
    if (normalized.length > 200) return validationError("entryIds", "must contain at most 200 IDs");
    const entries = normalized.map((entryId) => this.downloadEntries.get(entryId));
    const missingIndex = entries.findIndex((entry) => entry === undefined);
    if (missingIndex >= 0) {
      return notFoundError("DOWNLOAD_ENTRY_NOT_FOUND", "The download entry does not exist", {
        entryId: normalized[missingIndex],
      });
    }
    const duplicateExcluded = entries.find((entry) => entry
      && this.duplicateHiddenGalleryIds.has(entry.galleryId)
      && !this.explorationRestoredGalleryIds.has(entry.galleryId));
    if (duplicateExcluded) {
      return {
        ok: false,
        error: {
          code: "INVALID_DOWNLOAD_STATE",
          message: `Download entry ${duplicateExcluded.entryId} was excluded after duplicate review and must be restored before retry`,
          retryable: false,
          action: "review",
          details: {
            entryId: duplicateExcluded.entryId,
            state: duplicateExcluded.state,
            operation: "retry",
            reason: "duplicate_excluded",
          },
        },
      };
    }
    const invalid = entries.find((entry) => entry && !activeDownloadStates.has(entry.state)
      && !["failed", "interrupted", "cancelled"].includes(entry.state));
    if (invalid) {
      return {
        ok: false,
        error: {
          code: "INVALID_DOWNLOAD_STATE",
          message: `Download entry ${invalid.entryId} cannot be retried from ${invalid.state}`,
          retryable: false,
          action: "review",
          details: { entryId: invalid.entryId, state: invalid.state, operation: "retry" },
        },
      };
    }

    return ok(entries.map((entry) => {
      const current = entry!;
      const reused = activeDownloadStates.has(current.state);
      if (!reused) {
        const attempt = (current.attempt ?? 1) + 1;
        const next: DownloadEntry = {
          ...current,
          revision: current.revision + 1,
          state: "queued",
          progress: 0,
          attempt,
          errorCode: undefined,
          errorMessage: undefined,
        };
        this.downloadEntries.set(next.entryId, next);
        this.activeDownloadEntryByGallery.set(next.galleryId, next.entryId);
        this.emit("download:changed", cloneDownloadEntry(next));
        if (this.listeners["download:changed"].size > 0) this.runFixtureDownload(next.entryId, attempt);
      }
      return { jobId: `browser-fixture-${current.entryId}`, reused };
    }));
  }

  async downloadCancel(entryIds: string[]): Promise<ApiResult<DownloadEntry[]>> {
    const normalized = [...new Set(entryIds.map((entryId) => entryId.trim()))];
    if (!normalized.length || normalized.some((entryId) => !entryId)) {
      return validationError("entryIds", "must contain at least one non-empty entry ID");
    }
    if (normalized.length > 200) return validationError("entryIds", "must contain at most 200 IDs");
    const entries = normalized.map((entryId) => this.downloadEntries.get(entryId));
    const missingIndex = entries.findIndex((entry) => entry === undefined);
    if (missingIndex >= 0) {
      return notFoundError("DOWNLOAD_ENTRY_NOT_FOUND", "The download entry does not exist", {
        entryId: normalized[missingIndex],
      });
    }
    const invalid = entries.find((entry) => entry && !cancellableDownloadStates.has(entry.state));
    if (invalid) {
      return {
        ok: false,
        error: {
          code: "INVALID_DOWNLOAD_STATE",
          message: `Download entry ${invalid.entryId} cannot be cancelled from ${invalid.state}`,
          retryable: false,
          action: "review",
          details: { entryId: invalid.entryId, state: invalid.state, operation: "cancel" },
        },
      };
    }

    const cancelled = entries.map((entry) => {
      const current = entry!;
      if (current.state === "cancelled") return cloneDownloadEntry(current);
      const preserveFailure = current.state === "failed" || current.state === "interrupted";
      const next: DownloadEntry = {
        ...current,
        revision: current.revision + 1,
        state: "cancelled",
        ...(preserveFailure ? {} : { errorCode: undefined, errorMessage: undefined }),
      };
      this.downloadEntries.set(next.entryId, next);
      if (this.activeDownloadEntryByGallery.get(next.galleryId) === next.entryId) {
        this.activeDownloadEntryByGallery.delete(next.galleryId);
      }
      this.emit("download:changed", cloneDownloadEntry(next));
      return cloneDownloadEntry(next);
    });
    return ok(cancelled);
  }

  async downloadQuarantine(): Promise<ApiResult<DownloadEntry[]>> {
    return {
      ok: false,
      error: {
        code: "ARTIFACT_UNAVAILABLE_IN_BROWSER",
        message: "브라우저 fixture에는 격리할 실제 다운로드 파일이 없습니다.",
        retryable: false,
        action: "none",
      },
    };
  }

  async downloadQuarantineUndo(): Promise<ApiResult<DownloadEntry[]>> {
    return {
      ok: false,
      error: {
        code: "ARTIFACT_UNAVAILABLE_IN_BROWSER",
        message: "브라우저 fixture에는 복원할 실제 다운로드 파일이 없습니다.",
        retryable: false,
        action: "none",
      },
    };
  }

  async thumbnailRequest(request: ThumbnailRequestDto): Promise<ApiResult<ThumbnailRequestToken>> {
    if (request.key.kind !== "artifactPage" && (!Number.isInteger(request.key.galleryId) || request.key.galleryId <= 0)) {
      return validationError("key.galleryId", "must be a positive integer");
    }
    if (request.key.kind === "artifactPage" && !request.key.entryId.trim()) {
      return validationError("key.entryId", "must not be empty");
    }
    if (request.key.kind !== "galleryCover" && (!Number.isInteger(request.key.sourcePage) || request.key.sourcePage < 1)) {
      return validationError("key.sourcePage", "must be one-based");
    }
    const requestId = `browser-thumbnail-${this.nextThumbnailRequestId++}`;
    this.thumbnailRequestsTotal += 1;
    this.pendingThumbnailRequests.set(requestId, request);
    const token: ThumbnailRequestToken = { requestId, key: request.key };
    queueMicrotask(() => {
      if (!this.pendingThumbnailRequests.delete(requestId)) return;
      const label = request.key.kind === "galleryCover"
        ? `G${request.key.galleryId} · COVER`
        : request.key.kind === "galleryPage"
          ? `G${request.key.galleryId} · PAGE ${request.key.sourcePage}`
          : `${request.key.entryId} · VERIFIED PAGE ${request.key.sourcePage}`;
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><rect width="512" height="512" fill="#49656b"/><text x="28" y="470" fill="white" font-family="Segoe UI" font-size="24">${label}</text></svg>`;
      this.emit("thumbnail:ready", {
        ...token,
        outcome: {
          status: "ready",
          delivery: {
            key: request.key,
            cacheStatus: "resolved",
            thumbnail: {
              contentType: "image/svg+xml",
              bytes: [...new TextEncoder().encode(svg)],
              width: 512,
              height: 512,
              sourceRevision: "browser-fixture-v1",
            },
          },
        },
      });
    });
    return ok(token);
  }

  async thumbnailCancel(requestId: string): Promise<ApiResult<boolean>> {
    return ok(this.pendingThumbnailRequests.delete(requestId.trim()));
  }

  async thumbnailReprioritize(
    requestId: string,
    priority: ThumbnailRequestDto["priority"],
  ): Promise<ApiResult<boolean>> {
    const current = this.pendingThumbnailRequests.get(requestId.trim());
    if (!current) return ok(false);
    this.pendingThumbnailRequests.set(requestId.trim(), { ...current, priority });
    return ok(true);
  }

  async thumbnailInvalidate(key: ThumbnailRequestDto["key"]): Promise<ApiResult<ThumbnailInvalidation>> {
    return ok({ key, successCacheRemoved: false, negativeCacheRemoved: false });
  }

  async thumbnailStats(): Promise<ApiResult<ThumbnailWorkerStats>> {
    return ok({
      workerCount: this.settings.concurrentImageRequests,
      concurrencyLimit: this.settings.concurrentImageRequests,
      requestStartIntervalMs: this.settings.requestStartIntervalMs,
      activeWorkers: 0,
      queuedKeys: this.pendingThumbnailRequests.size,
      inFlightKeys: this.pendingThumbnailRequests.size,
      subscriberCount: this.pendingThumbnailRequests.size,
      successCacheEntries: 0,
      successCacheBytes: 0,
      negativeCacheEntries: 0,
      requestsTotal: this.thumbnailRequestsTotal,
      successCacheHits: 0,
      negativeCacheHits: 0,
      joinedInFlight: 0,
      resolvedSuccess: this.thumbnailRequestsTotal - this.pendingThumbnailRequests.size,
      resolvedFailure: 0,
      cancelledSubscribers: 0,
      cancelledWork: 0,
    });
  }

  async thumbnailCacheClear(): Promise<ApiResult<ThumbnailCacheClearResult>> {
    return ok({
      successEntriesRemoved: 0,
      successBytesRemoved: 0,
      negativeEntriesRemoved: 0,
    });
  }

  async detailOriginalPrepare(request: DetailOriginalPrepareRequest): Promise<ApiResult<DetailOriginalPrepared>> {
    const canonicalUuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
    const localEntryId = request.entryId?.trim();
    if (
      !canonicalUuid.test(request.requestId)
      || !Number.isInteger(request.galleryId)
      || request.galleryId <= 0
      || !Number.isInteger(request.sourcePage)
      || request.sourcePage < 1
      || (request.entryId !== undefined && !localEntryId)
      || (request.entryId === undefined && request.sourcePage !== 1)
    ) {
      return validationError("detailOriginal", "requestId, galleryId, sourcePage, or entryId is invalid");
    }
    if (localEntryId) {
      const entry = this.downloadEntries.get(localEntryId);
      if (!entry || entry.galleryId !== request.galleryId || entry.state !== "completed") {
        return notFoundError(
          "DETAIL_ORIGINAL_UNAVAILABLE",
          "The verified local artifact page is unavailable",
          { galleryId: request.galleryId, sourcePage: request.sourcePage, entryId: localEntryId },
        );
      }
    }
    return ok({
      requestId: request.requestId,
      galleryId: request.galleryId,
      sourcePage: request.sourcePage,
      mediaUrl: "/mock-gallery-sheet.png",
      contentType: "image/png" as const,
      width: 512,
      height: 512,
    });
  }

  async detailOriginalDispose(): Promise<ApiResult<boolean>> { return ok(true); }

  async explorationDataReset(
    request: ExplorationDataResetRequest,
  ): Promise<ApiResult<ExplorationDataResetResult>> {
    if (request.confirmation !== "RESET_EXPLORATION_DATA") {
      return validationError("confirmation", "must explicitly confirm exploration data reset");
    }
    if (this.autoFind.run?.state === "running") {
      return {
        ok: false,
        error: {
          code: "OPERATION_ACTIVE",
          message: "Auto Find 실행을 취소하거나 완료한 뒤 탐색 데이터를 초기화하세요.",
          retryable: false,
          action: "none",
        },
      };
    }
    const result: ExplorationDataResetResult = {
      favoritesRemoved: this.favorites.size,
      searchHistoryRemoved: this.searchHistory.size,
      autoFindRunsRemoved: this.autoFind.run ? 1 : 0,
      autoFindCandidatesRemoved: this.autoFind.candidates.length,
      autoFindExclusionsRemoved: this.autoFindExclusions.size,
    };
    this.autoFindGeneration += 1;
    this.favorites.clear();
    this.searchHistory.clear();
    this.autoFindExclusions.clear();
    this.excludedAutoFindCandidates.clear();
    this.explorationRestoredGalleryIds.clear();
    this.autoFind = { candidates: [], cutoffEvidence: [], truncations: [] };
    return ok(result);
  }

  async appMinimizeToTray(): Promise<ApiResult<null>> {
    return ok(null);
  }

  async appActiveWorkSnapshot(): Promise<ApiResult<AppActiveWorkSnapshot>> {
    const activeDownloadEntryIds = [...this.downloadEntries.values()]
      .filter((entry) => activeDownloadStates.has(entry.state))
      .map((entry) => entry.entryId)
      .sort((left, right) => left.localeCompare(right));
    const autoFind = this.autoFind.run?.state === "running" ? this.autoFind.run : undefined;
    const duplicateScan = this.duplicateSnapshotState.run?.state === "running"
      ? this.duplicateSnapshotState.run
      : undefined;
    const internalDuplicateScan = this.internalSnapshotState.run?.state === "running"
      ? this.internalSnapshotState.run
      : undefined;
    return ok({
      // Runtime snapshots use an epoch-millisecond string; keep the browser
      // fixture wire-compatible even though the UI does not render this value.
      queriedAt: Date.now().toString(),
      workSetFingerprint: browserWorkSetFingerprint(
        activeDownloadEntryIds,
        autoFind?.runId,
        duplicateScan?.runId,
        internalDuplicateScan?.runId,
      ),
      downloads: { activeCount: activeDownloadEntryIds.length },
      ...(autoFind ? {
        autoFind: {
          runId: autoFind.runId,
          completedFavorites: autoFind.completedFavorites,
          totalFavorites: autoFind.totalFavorites,
          candidatesFound: autoFind.candidatesFound,
        },
      } : {}),
      ...(duplicateScan ? {
        duplicateScan: {
          runId: duplicateScan.runId,
          hashedArtifacts: duplicateScan.hashedArtifacts,
          totalArtifacts: duplicateScan.totalArtifacts,
          comparedPairs: duplicateScan.comparedPairs,
          totalPairs: duplicateScan.totalPairs,
          candidatesFound: duplicateScan.candidatesFound,
        },
      } : {}),
      ...(internalDuplicateScan ? {
        internalDuplicateScan: {
          runId: internalDuplicateScan.runId,
          scannedArtifacts: internalDuplicateScan.scannedArtifacts,
          totalArtifacts: internalDuplicateScan.totalArtifacts,
          skippedArtifacts: internalDuplicateScan.skippedArtifacts,
          groupsFound: internalDuplicateScan.groupsFound,
        },
      } : {}),
    });
  }

  async artifactOpenFirst(): Promise<ApiResult<null>> {
    return {
      ok: false,
      error: {
        code: "ARTIFACT_UNAVAILABLE_IN_BROWSER",
        message: "브라우저 fixture에는 실제 다운로드 파일이 없습니다.",
        retryable: false,
        action: "none",
      },
    };
  }

  async artifactOpenFolder(): Promise<ApiResult<null>> {
    return {
      ok: false,
      error: {
        code: "ARTIFACT_FOLDER_UNAVAILABLE_IN_BROWSER",
        message: "브라우저 fixture에는 실제 다운로드 저장 폴더가 없습니다.",
        retryable: false,
        action: "none",
      },
    };
  }

  async appReconcile(): Promise<ApiResult<ReconcileReport>> {
    return ok({
      inspectedArtifacts: 0,
      verifiedArtifacts: 0,
      resumedJobs: 0,
      issues: [],
    });
  }

  async maintenancePreview(action: MaintenanceAction): Promise<ApiResult<MaintenancePreview>> {
    const previewId = `browser-maintenance-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    this.maintenancePreviews.set(previewId, action);
    const factory = action.kind === "factoryReset";
    return ok({
      previewId,
      action,
      originalFilesDeleted: false,
      userDecisionsPreserved: !factory,
      restartRequired: factory,
      warnings: factory ? ["브라우저 fixture에서는 앱 종료 없이 메모리 상태만 초기화합니다."] : [],
      steps: factory ? ["앱 데이터를 첫 실행 상태로 되돌립니다."] : ["파생 cache와 중단 상태를 정리합니다."],
    });
  }

  async maintenanceExecute(previewId: string, action: MaintenanceAction): Promise<ApiResult<MaintenanceResult>> {
    const previewed = this.maintenancePreviews.get(previewId);
    this.maintenancePreviews.delete(previewId);
    if (!previewed || JSON.stringify(previewed) !== JSON.stringify(action)) {
      return validationError("previewId", "a matching maintenance preview is required before execution");
    }
    if (action.kind === "factoryReset" && action.confirmation !== "RESET_ALL_APP_DATA") {
      return validationError("confirmation", "must explicitly confirm complete app data reset");
    }
    if (action.kind === "quickRepair") {
      return ok({ action, completedSteps: ["thumbnail and source caches cleared", "interrupted work recovery completed"], warnings: [], restartRequired: false });
    }
    if (action.kind === "rebuildLibrary") {
      return ok({ action, completedSteps: ["0 artifacts inspected"], warnings: [], restartRequired: false });
    }
    this.downloadEntries.clear();
    this.favorites.clear();
    this.searchHistory.clear();
    this.autoFindExclusions.clear();
    this.excludedAutoFindCandidates.clear();
    this.explorationRestoredGalleryIds.clear();
    this.autoFind = { candidates: [], cutoffEvidence: [], truncations: [] };
    return ok({ action, completedSteps: ["factory reset completed in browser fixture"], warnings: [], restartRequired: true });
  }

  async appQuit(request: AppQuitRequest): Promise<ApiResult<AppQuitResult>> {
    const current = await this.appActiveWorkSnapshot();
    if (!current.ok) return current;
    if (request.expectedWorkSetFingerprint !== current.data.workSetFingerprint) {
      return ok({ accepted: false, reason: "active_work_changed", snapshot: current.data });
    }
    if (hasActiveWork(current.data) && !request.confirmActiveWork) {
      return ok({ accepted: false, reason: "active_work_confirmation_required", snapshot: current.data });
    }
    return ok({ accepted: true, snapshot: current.data });
  }

  async on<K extends keyof BackendEventMap>(
    event: K,
    handler: (payload: BackendEventMap[K]) => void,
  ): Promise<Unsubscribe> {
    const handlers = this.listeners[event] as Set<(payload: BackendEventMap[K]) => void>;
    handlers.add(handler);
    return () => handlers.delete(handler);
  }

  private emit<K extends keyof BackendEventMap>(event: K, payload: BackendEventMap[K]): void {
    const handlers = this.listeners[event] as Set<(payload: BackendEventMap[K]) => void>;
    handlers.forEach((handler) => handler(payload));
  }

  private recordSearchHistory(input: SearchRequest): void {
    const request = normalizeSearchRequest(input);
    if (!request.text && !request.includeTags.length && !request.excludeTags.length) return;
    const fingerprint = JSON.stringify(request);
    const current = this.searchHistory.get(fingerprint);
    const now = new Date().toISOString();
    const entry: SearchHistoryEntry = current
      ? { ...current, useCount: current.useCount + 1, lastUsedAt: now }
      : {
          historyId: this.nextSearchHistoryId++,
          ...request,
          useCount: 1,
          lastUsedAt: now,
        };
    this.searchHistory.set(fingerprint, entry);
  }

  private isDuplicateExplorationExcluded(galleryId: GalleryId): boolean {
    if (this.explorationRestoredGalleryIds.has(galleryId)) return false;
    return this.duplicateHiddenGalleryIds.has(galleryId);
  }

  private advanceDuplicateScanFixture(generation: number, complete: boolean): void {
    const current = this.duplicateSnapshotState.run;
    if (generation !== this.duplicateGeneration || !current || current.state !== "running") return;

    const now = new Date().toISOString();
    let candidates = this.duplicateSnapshotState.candidates;
    if (complete) {
      const fixture = this.duplicateReviews.get("browser-duplicate-archive-tram")
        ?? browserDuplicateReviewFixture(now);
      this.duplicateReviews.set(fixture.candidate.candidateId, fixture);
      candidates = this.duplicateResolvedCandidates.has(fixture.candidate.candidateId)
        ? []
        : [cloneDuplicateCandidate(fixture.candidate)];
    }
    const next: DuplicateScanRun = {
      ...current,
      revision: current.revision + 1,
      state: complete ? "completed" : "running",
      hashedArtifacts: complete ? current.totalArtifacts : 1,
      comparedPairs: complete ? current.totalPairs : 0,
      candidatesFound: candidates.length,
      updatedAt: now,
      ...(complete ? { finishedAt: now } : {}),
    };
    this.duplicateSnapshotState = {
      ...this.duplicateSnapshotState,
      run: next,
      candidates,
    };
    this.emit("duplicate:changed", cloneDuplicateScanRun(next));
  }

  private runAutoFindFixture(
    generation: number,
    favorite: FavoriteRecord,
    finalFavorite: boolean,
  ): void {
    const current = this.autoFind.run;
    if (generation !== this.autoFindGeneration || !current || current.state !== "running") return;

    const fixture = runSearchFixture({
      text: `artist:${favorite.value}`,
      includeTags: [],
      excludeTags: [],
      languages: ["korean", "japanese", "chinese", "english"],
      sort: "recent",
      pageSize: 200,
    });
    const downloaded = new Set([...this.downloadEntries.values()].map((entry) => entry.galleryId));
    const existing = new Set(this.autoFind.candidates.map((candidate) => candidate.id));
    const discoveredAt = new Date().toISOString();
    const cutoff = this.autoFind.cutoffEvidence.find((evidence) => evidence.artist === favorite.value);
    const eligible = fixture.items.filter((gallery) => historyModeAllows(gallery.id, cutoff, current.historyMode));
    const candidateLimit = 200;
    const truncation = eligible.length > candidateLimit
      ? {
        artist: favorite.value,
        reason: "candidate_limit_after_cutoff" as const,
        eligibleCount: eligible.length,
        limit: candidateLimit,
      }
      : null;
    const candidates = eligible
      .slice(0, candidateLimit)
      .filter((gallery) => !downloaded.has(gallery.id))
      .filter((gallery) => !this.isDuplicateExplorationExcluded(gallery.id))
      .filter((gallery) => !this.autoFindExclusions.has(gallery.id))
      .filter((gallery) => !existing.has(gallery.id))
      .map((gallery) => ({
        ...gallery,
        runId: current.runId,
        matchedFavorite: { namespace: favorite.namespace, value: favorite.value },
        discoveredAt,
      }));
    const allCandidates = [...this.autoFind.candidates, ...candidates];
    const completedFavorites = current.completedFavorites + 1;
    const completed = finalFavorite || completedFavorites >= current.totalFavorites;
    const now = new Date().toISOString();
    const run: AutoFindRun = {
      ...current,
      revision: current.revision + 1,
      state: completed ? "completed" : "running",
      completedFavorites,
      candidatesFound: allCandidates.length,
      updatedAt: now,
      ...(completed ? { finishedAt: now } : {}),
    };
    const truncations = this.autoFind.truncations.filter((item) => item.artist !== favorite.value);
    if (truncation) truncations.push(truncation);
    this.autoFind = { ...this.autoFind, run, candidates: allCandidates, truncations };
    this.emit("auto-find:changed", cloneAutoFindRun(run));
  }

  private runFixtureDownload(entryId: string, workerAttempt: number): void {
    const steps: Array<{
      delay: number;
      expectedState: DownloadEntry["state"];
      state: DownloadEntry["state"];
      message: string;
    }> = [
      {
        delay: 75,
        expectedState: "queued",
        state: "resolving_metadata",
        message: "저장 fixture의 대기열 요청을 확인하고 있습니다.",
      },
      {
        delay: 225,
        expectedState: "resolving_metadata",
        state: "interrupted",
        message: "실제 원격 artifact 다운로드 기반은 아직 구현되지 않았습니다.",
      },
    ];

    for (const step of steps) {
      window.setTimeout(() => {
        const current = this.downloadEntries.get(entryId);
        if (
          !current
          || current.state !== step.expectedState
          || current.attempt !== workerAttempt
        ) return;
        const failed = step.state === "interrupted";
        const next: DownloadEntry = {
          ...current,
          revision: current.revision + 1,
          state: step.state,
          progress: 0,
          attempt: workerAttempt,
          errorCode: failed ? "DOWNLOAD_FOUNDATION_UNAVAILABLE" : undefined,
          errorMessage: failed ? step.message : undefined,
        };
        this.downloadEntries.set(entryId, next);
        if (step.state === "interrupted" && this.activeDownloadEntryByGallery.get(current.galleryId) === entryId) {
          this.activeDownloadEntryByGallery.delete(current.galleryId);
        }
        this.emit("job:changed", {
          jobId: `browser-fixture-${entryId}`,
          galleryId: current.galleryId,
          revision: next.revision,
          state: next.state,
          completedUnits: 0,
          totalUnits: 1,
          message: step.message,
        });
        this.emit("download:changed", cloneDownloadEntry(next));
      }, step.delay);
    }
  }

}

class TauriBackend implements BackendClient {
  readonly runtime = "tauri" as const;

  settingsGet(): Promise<ApiResult<SettingsSnapshot>> {
    return invoke("settings_get");
  }

  settingsUpdate(patch: SettingsPatch, expectedRevision: number): Promise<ApiResult<SettingsSnapshot>> {
    return invoke("settings_update", { patch, expectedRevision });
  }

  storageUsageGet(): Promise<ApiResult<StorageUsageSnapshot>> {
    return invoke("storage_usage_get");
  }

  danbooruSearch(request: DanbooruSearchRequest): Promise<ApiResult<DanbooruSearchPage>> {
    return invoke("danbooru_search", { request });
  }

  danbooruRandom(): Promise<ApiResult<DanbooruPost>> {
    return invoke("danbooru_random");
  }

  danbooruRelated(request: DanbooruRelatedRequest): Promise<ApiResult<DanbooruRelatedPosts>> {
    return invoke("danbooru_related", { request });
  }

  danbooruAutocomplete(query: string, limit: number): Promise<ApiResult<DanbooruAutocompleteItem[]>> {
    return invoke("danbooru_autocomplete", { query, limit });
  }

  danbooruDownload(postId: number): Promise<ApiResult<DanbooruDownloadRecord>> {
    return invoke("danbooru_download", { postId });
  }

  danbooruDownloadsList(request: DanbooruDownloadsRequest): Promise<ApiResult<DanbooruDownloadsPage>> {
    return invoke("danbooru_downloads_list", { request });
  }

  tagCatalogStatus(): Promise<ApiResult<TagCatalogStatus>> { return invoke("tag_catalog_status"); }
  tagCatalogRefresh(): Promise<ApiResult<TagCatalogStatus>> { return invoke("tag_catalog_refresh"); }
  tagSuggestionsSearch(request: TagSuggestionRequest): Promise<ApiResult<TagSuggestion[]>> { return invoke("tag_suggestions_search", { request }); }

  folderNameTemplatePreview(template: string): Promise<ApiResult<string>> {
    return invoke("folder_name_template_preview", { template });
  }

  windowPlacementGet(): Promise<ApiResult<WindowPlacementSnapshot>> {
    return invoke("window_placement_get");
  }

  windowPlacementUpdate(
    placement: WindowPlacement,
    expectedRevision: number,
  ): Promise<ApiResult<WindowPlacementSnapshot>> {
    return invoke("window_placement_update", { placement, expectedRevision });
  }

  searchSubmit(request: SearchRequest): Promise<ApiResult<SearchSubmission>> {
    return invoke("search_submit", { request });
  }

  searchPageGet(queryId: string, page: number, requestId: string): Promise<ApiResult<GalleryPage>> {
    return invoke("search_page_get", { queryId, page, requestId });
  }

  searchPageCancel(requestId: string): Promise<ApiResult<boolean>> {
    return invoke("search_page_cancel", { requestId });
  }

  galleryDetailGet(galleryId: GalleryDetail["id"]): Promise<ApiResult<GalleryDetail>> {
    return invoke("gallery_detail_get", { galleryId });
  }

  favoritesList(): Promise<ApiResult<FavoriteRecord[]>> {
    return invoke("favorites_list");
  }

  favoriteSet(key: FavoriteKey, enabled: boolean): Promise<ApiResult<FavoriteMutationResult>> {
    return invoke("favorite_set", { key, enabled });
  }

  searchHistoryList(limit: number): Promise<ApiResult<SearchHistoryEntry[]>> {
    return invoke("search_history_list", { limit });
  }

  autoFindSnapshot(): Promise<ApiResult<AutoFindSnapshot>> {
    return invoke("auto_find_snapshot");
  }

  autoFindRefresh(): Promise<ApiResult<AutoFindRun>> {
    return invoke("auto_find_refresh");
  }

  autoFindCancel(): Promise<ApiResult<AutoFindRun>> {
    return invoke("auto_find_cancel");
  }

  autoFindExclude(
    galleryIds: GalleryId[],
    reason: string,
  ): Promise<ApiResult<AutoFindExclusionResult>> {
    return invoke("auto_find_exclude", { galleryIds, reason });
  }

  explorationExclusionsList(): Promise<ApiResult<ExplorationExclusion[]>> {
    return invoke("exploration_exclusions_list");
  }

  explorationExclusionsRestore(
    galleryIds: GalleryId[],
  ): Promise<ApiResult<ExplorationExclusionRestoreResult>> {
    return invoke("exploration_exclusions_restore", { galleryIds });
  }

  duplicateSnapshot(): Promise<ApiResult<DuplicateSnapshot>> {
    return invoke("duplicate_snapshot");
  }

  duplicateScanStart(): Promise<ApiResult<DuplicateScanRun>> {
    return invoke("duplicate_scan_start");
  }

  duplicateScanCancel(): Promise<ApiResult<DuplicateScanRun>> {
    return invoke("duplicate_scan_cancel");
  }

  duplicateReviewGet(candidateId: string): Promise<ApiResult<DuplicateReview>> {
    return invoke("duplicate_review_get", { candidateId });
  }

  duplicateDecisionApply(request: DuplicateDecisionRequest): Promise<ApiResult<DuplicateReview>> {
    return invoke("duplicate_decision_apply", { request });
  }

  downloadOverlapReviewGet(reviewId: string): Promise<ApiResult<DownloadOverlapReview>> {
    return invoke("download_overlap_review_get", { reviewId });
  }

  downloadOverlapDecisionApply(request: DownloadOverlapDecisionRequest): Promise<ApiResult<DownloadOverlapDecisionResult>> {
    return invoke("download_overlap_decision_apply", { request });
  }

  internalDuplicateSnapshot(): Promise<ApiResult<InternalDuplicateSnapshot>> {
    return invoke("internal_duplicate_snapshot");
  }

  internalDuplicateActiveArtifact(): Promise<ApiResult<InternalArtifactScanProgress | null>> {
    return invoke("internal_duplicate_active_artifact");
  }

  internalDuplicateScanStart(request: InternalScanRequest): Promise<ApiResult<InternalScanRun>> {
    return invoke("internal_duplicate_scan_start", { request });
  }

  internalDuplicateScanCancel(): Promise<ApiResult<InternalScanRun>> {
    return invoke("internal_duplicate_scan_cancel");
  }

  internalDuplicateReviewGet(entryId: string): Promise<ApiResult<InternalDuplicateReview>> {
    return invoke("internal_duplicate_review_get", { entryId });
  }

  internalRemovalPlan(request: InternalRemovalPlanRequest): Promise<ApiResult<InternalRemovalPlan>> {
    return invoke("internal_removal_plan", { request });
  }

  internalRemovalApply(request: InternalRemovalApplyRequest): Promise<ApiResult<InternalRemovalResult>> {
    return invoke("internal_removal_apply", { request });
  }

  internalRemovalUndo(request: InternalRemovalUndoRequest): Promise<ApiResult<InternalRemovalResult>> {
    return invoke("internal_removal_undo", { request });
  }

  downloadQueueAdd(
    galleries: GalleryId[],
    requestId: string,
  ): Promise<ApiResult<DownloadEntry[]>> {
    return invoke("download_queue_add", { galleries, requestId });
  }

  downloadEntriesList(request: DownloadListRequest): Promise<ApiResult<DownloadPage>> {
    return invoke("download_entries_list", { request });
  }

  downloadLibraryPageList(request: DownloadListRequest): Promise<ApiResult<DownloadLibraryPage>> {
    return invoke("download_library_page_list", { request });
  }

  downloadRetry(entryIds: string[]): Promise<ApiResult<JobRef[]>> {
    return invoke("download_retry", { entryIds });
  }

  downloadCancel(entryIds: string[]): Promise<ApiResult<DownloadEntry[]>> {
    return invoke("download_cancel", { entryIds });
  }

  downloadQuarantine(entryIds: string[], reason: string): Promise<ApiResult<DownloadEntry[]>> {
    return invoke("download_quarantine", { entryIds, reason });
  }

  downloadQuarantineUndo(entryIds: string[]): Promise<ApiResult<DownloadEntry[]>> {
    return invoke("download_quarantine_undo", { entryIds });
  }

  appActiveWorkSnapshot(): Promise<ApiResult<AppActiveWorkSnapshot>> {
    return invoke("app_active_work_snapshot");
  }

  artifactOpenFirst(entryId: string): Promise<ApiResult<null>> {
    return invoke("artifact_open_first", { entryId });
  }

  artifactOpenFolder(entryId: string): Promise<ApiResult<null>> {
    return invoke("artifact_open_folder", { entryId });
  }

  appReconcile(): Promise<ApiResult<ReconcileReport>> {
    return invoke("app_reconcile");
  }

  maintenancePreview(action: MaintenanceAction): Promise<ApiResult<MaintenancePreview>> {
    return invoke("maintenance_preview", { action });
  }

  maintenanceExecute(previewId: string, action: MaintenanceAction): Promise<ApiResult<MaintenanceResult>> {
    return invoke("maintenance_execute", { previewId, action });
  }

  thumbnailRequest(request: ThumbnailRequestDto): Promise<ApiResult<ThumbnailRequestToken>> {
    return invoke("thumbnail_request", { request });
  }

  thumbnailCancel(requestId: string): Promise<ApiResult<boolean>> {
    return invoke("thumbnail_cancel", { requestId });
  }

  thumbnailReprioritize(
    requestId: string,
    priority: ThumbnailRequestDto["priority"],
  ): Promise<ApiResult<boolean>> {
    return invoke("thumbnail_reprioritize", { requestId, priority });
  }

  thumbnailInvalidate(key: ThumbnailRequestDto["key"]): Promise<ApiResult<ThumbnailInvalidation>> {
    return invoke("thumbnail_invalidate", { key });
  }

  thumbnailStats(): Promise<ApiResult<ThumbnailWorkerStats>> {
    return invoke("thumbnail_stats");
  }

  thumbnailCacheClear(): Promise<ApiResult<ThumbnailCacheClearResult>> {
    return invoke("thumbnail_cache_clear");
  }

  detailOriginalPrepare(request: DetailOriginalPrepareRequest): Promise<ApiResult<DetailOriginalPrepared>> {
    return invoke("detail_original_prepare", { request });
  }
  detailOriginalDispose(requestId: string): Promise<ApiResult<boolean>> {
    return invoke("detail_original_dispose", { requestId });
  }

  explorationDataReset(
    request: ExplorationDataResetRequest,
  ): Promise<ApiResult<ExplorationDataResetResult>> {
    return invoke("exploration_data_reset", { request });
  }

  appMinimizeToTray(): Promise<ApiResult<null>> {
    return invoke("app_minimize_to_tray");
  }

  appQuit(request: AppQuitRequest): Promise<ApiResult<AppQuitResult>> {
    return invoke("app_quit", { request });
  }

  async on<K extends keyof BackendEventMap>(
    event: K,
    handler: (payload: BackendEventMap[K]) => void,
  ): Promise<Unsubscribe> {
    const unlisten: UnlistenFn = await listen<BackendEventMap[K]>(event, ({ payload }) => handler(payload));
    return unlisten;
  }
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export const backend: BackendClient = window.__TAURI_INTERNALS__
  ? new TauriBackend()
  : new BrowserMockBackend();
