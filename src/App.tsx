import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type ReactNode } from "react";
import { backend } from "./api/backend";
import { hasActiveWork } from "./api/contracts";
import type {
  AppActiveWorkSnapshot,
  AutoFindRun,
  AutoFindSnapshot,
  DownloadChangedEvent,
  DownloadEntry,
  DownloadOverlapDecisionRequest,
  DownloadOverlapReview,
  DuplicateDecisionRequest,
  DuplicateReview,
  DuplicateScanRun,
  DuplicateSnapshot,
  FavoriteKey,
  FavoriteNamespace,
  FavoriteRecord,
  GalleryPage,
  InternalDuplicateReview,
  InternalDuplicateSnapshot,
  InternalArtifactScanProgress,
  InternalRemovalPlan,
  InternalRemovalPlanRequest,
  InternalScanRun,
  SearchHistoryEntry,
  SearchRequest,
  SettingsPatch,
  MaintenanceAction,
  MaintenanceResult,
  ApiResult,
  TagCatalogStatus,
  TagNamespace,
  TagSuggestion,
} from "./api/contracts";
import { ActivityDrawer } from "./components/ActivityDrawer";
import { DetailWorkspace } from "./components/DetailWorkspace";
import { DuplicateReviewDialog } from "./components/DuplicateReviewDialog";
import { DownloadOverlapReviewDialog } from "./components/DownloadOverlapReviewDialog";
import { ExploreContextBar, type ExploreContextTab } from "./components/ExploreContextBar";
import { InternalDuplicateDialog } from "./components/InternalDuplicateDialog";
import { ExitConfirmDialog } from "./components/ExitConfirmDialog";
import { FluentIcon } from "./components/FluentIcon";
import { GalleryCard } from "./components/GalleryCard";
import { GalleryGrid } from "./components/GalleryGrid";
import { GalleryGridSkeleton } from "./components/GalleryGridSkeleton";
import { KeyboardShortcutsDialog } from "./components/KeyboardShortcutsDialog";
import { SelectionToolbar } from "./components/SelectionToolbar";
import { SettingsDialog } from "./components/SettingsDialog";
import { SideRail } from "./components/SideRail";
import { TutorialDialog } from "./components/TutorialDialog";
import { UpdateDialog } from "./components/UpdateDialog";
import { ViewHeader, type SearchSuggestion } from "./components/ViewHeader";
import { galleryId, retryableDownloadStates, type DownloadFilter, type DownloadState, type Gallery, type GalleryId, type Language, type SearchSort, type ViewId } from "./core/types";
import { useSettings } from "./hooks/useSettings";
import { useWindowPlacement } from "./hooks/useWindowPlacement";
import { resolveGalleryColumns } from "./layout/galleryColumns";
import { buildSearchSuggestionCatalog, catalogSuggestion } from "./search/searchSuggestions";
import { activeSearchToken, metadataSearchToken, searchTokenKind } from "./search/searchTokens";
import { applyDownloadChanged } from "./state/downloadProjection";
import {
  duplicateEventNeedsSnapshot,
  duplicateRunIsNewer,
  mergeHydratedDuplicateSnapshot,
  validDuplicateRun,
} from "./state/duplicateProjection";
import { mergeDownloadEntries, mergeGalleryDetail, mergeGalleryPage } from "./state/galleryProjection";
import { galleryQueryReducer, initialGalleryQueryState, type GalleryQueryState } from "./state/galleryQuery";
import { ExplorePageSession } from "./state/explorePageSession";
import { visibleGalleries } from "./state/selectors";
import { galleryGroupStorageKey, groupGalleries, type GalleryGroup, type GalleryGrouping } from "./state/galleryGrouping";
import { initialUiState, uiReducer } from "./state/uiState";
import { useThumbnailClient } from "./thumbnail";
import { isTutorialDismissed, setTutorialDismissed } from "./tutorial/tutorialPreference";
import { useAppUpdater } from "./update/useAppUpdater";

const viewConfig: Record<ViewId, { eyebrow: string; title: string }> = {
  explore: { eyebrow: "EXPLORE", title: "갤러리 탐색" },
  "auto-find": { eyebrow: "AUTO FIND", title: "즐겨찾기 작가 자동 탐색" },
  downloads: { eyebrow: "DOWNLOADS", title: "다운로드 목록" },
};

const viewOrder: ViewId[] = ["explore", "auto-find", "downloads"];

const sortOptions: Array<{ value: SearchSort; label: string }> = [
  { value: "recent", label: "최신순" },
  { value: "popular_today", label: "인기순 · 오늘" },
  { value: "popular_week", label: "인기순 · 이번 주" },
  { value: "popular_month", label: "인기순 · 이번 달" },
  { value: "popular_year", label: "인기순 · 올해" },
  { value: "random", label: "무작위" },
];

const previewFolderNameTemplate = (template: string) => backend.folderNameTemplatePreview(template);
const loadExplorationExclusions = () => backend.explorationExclusionsList();
const restoreExplorationExclusions = (galleryIds: GalleryId[]) =>
  backend.explorationExclusionsRestore(galleryIds);

const activeDownloadStates: ReadonlySet<DownloadState> = new Set([
  "queued",
  "resolving_metadata",
  "downloading",
  "hashing",
  "verifying",
  "retry_wait",
]);

type Toast = { id: number; message: string } | null;

type UndoAction =
  | { kind: "auto-find-exclusion"; galleryIds: GalleryId[] }
  | { kind: "download-quarantine"; entryIds: string[] };

type ExploreContext = {
  id: string;
  label: string;
  root: boolean;
  session: ExplorePageSession;
  request: SearchRequest | null;
  requestKey: string | null;
  displayValue: string;
  languages: Language[];
  sort: SearchSort;
  query: GalleryQueryState;
  exploreIds: GalleryId[];
  scrollTop: number;
  keyboardFocusId: GalleryId | null;
  selectionIds: GalleryId[];
  selectionAnchorId: GalleryId | null;
  lastAccessed: number;
};

const maximumExploreContexts = 5;

const cloneSearchRequest = (request: SearchRequest): SearchRequest => ({
  ...request,
  includeTags: [...request.includeTags],
  excludeTags: [...request.excludeTags],
  languages: [...request.languages],
});

const searchRequestKey = (request: SearchRequest): string => JSON.stringify({
  text: request.text.trim().toLocaleLowerCase(),
  includeTags: [...request.includeTags].map(normalizeMetadataToken).sort(),
  excludeTags: [...request.excludeTags].map(normalizeMetadataToken).sort(),
  languages: [...request.languages].sort(),
  sort: request.sort,
  pageSize: request.pageSize,
});

const normalizeMetadataToken = (value: string): string => value.trim().toLocaleLowerCase();

const favoriteToken = (favorite: Pick<FavoriteRecord, "namespace" | "value">): string =>
  favorite.namespace === "tag"
    ? normalizeMetadataToken(favorite.value)
    : `${favorite.namespace}:${normalizeMetadataToken(favorite.value)}`;

const favoriteKeyFromToken = (token: string): FavoriteKey => {
  const normalized = normalizeMetadataToken(token);
  const separator = normalized.indexOf(":");
  const possibleNamespace = separator > 0 ? normalized.slice(0, separator) : "";
  const namespaces: ReadonlySet<string> = new Set(["artist", "group", "series", "character"]);
  if (separator > 0 && namespaces.has(possibleNamespace)) {
    return {
      namespace: possibleNamespace as Exclude<FavoriteNamespace, "tag">,
      value: normalized.slice(separator + 1),
    };
  }
  return { namespace: "tag", value: normalized };
};

const autoFindStatusLabel = (loading: boolean, error: string | null, run?: AutoFindRun): string => {
  if (loading) return "저장된 자동 탐색 결과를 불러오는 중";
  if (error) return `자동 탐색 오류 · ${error}`;
  if (!run) return "아직 실행한 자동 탐색이 없습니다.";
  if (run.state === "running") {
    return `탐색 중 · 작가 ${run.completedFavorites}/${run.totalFavorites} · 후보 ${run.candidatesFound}개`;
  }
  if (run.state === "failed") return `탐색 실패 · ${run.errorMessage ?? run.errorCode ?? "원인을 확인해 주세요."}`;
  if (run.state === "cancelled") return `탐색 취소됨 · 후보 ${run.candidatesFound}개 보존`;
  return `탐색 완료 · 작가 ${run.completedFavorites}/${run.totalFavorites} · 후보 ${run.candidatesFound}개`;
};

const duplicateStatusLabel = (loading: boolean, error: string | null, run?: DuplicateScanRun): string => {
  if (loading) return "저장된 작품 중복 검사 결과를 불러오는 중";
  if (error) return `작품 중복 검사 오류 · ${error}`;
  if (!run) return "아직 실행한 작품 중복 검사가 없습니다.";
  if (run.state === "running") {
    return `중복 검사 중 · 아티팩트 ${run.hashedArtifacts}/${run.totalArtifacts} · 비교 ${run.comparedPairs}/${run.totalPairs} · 후보 ${run.candidatesFound}개`;
  }
  if (run.state === "failed") return `중복 검사 실패 · ${run.errorMessage ?? run.errorCode ?? "원인을 확인해 주세요."}`;
  if (run.state === "cancelled") return `중복 검사 취소됨 · 비교 ${run.comparedPairs}/${run.totalPairs} · 기존 후보 보존`;
  return `중복 검사 완료 · 비교 ${run.comparedPairs}/${run.totalPairs} · 후보 ${run.candidatesFound}개`;
};

const internalStatusLabel = (loading: boolean, error: string | null, run?: InternalScanRun): string => {
  if (loading) return "저장된 내부 중복 결과를 불러오는 중";
  if (error) return `내부 중복 오류 · ${error}`;
  if (!run) return "내부 중복 검사를 아직 실행하지 않았습니다.";
  if (run.state === "running") return `내부 중복 검사 중 · 대상 ${run.scannedArtifacts}/${run.totalArtifacts}개 · 제외 ${run.skippedArtifacts}개`;
  if (run.state === "failed") return `내부 중복 검사 실패 · ${run.errorMessage ?? run.errorCode ?? "원인을 확인해 주세요."}`;
  if (run.state === "cancelled") return `내부 중복 검사 취소됨 · 기존 검토 결과 보존`;
  return `내부 중복 검사 완료 · 앨범 ${run.scannedArtifacts}개 · 500p 이상 제외 ${run.skippedArtifacts}개 · 검토 행 ${run.groupsFound}개`;
};

export default function App() {
  const thumbnailClient = useThumbnailClient();
  const [ui, dispatch] = useReducer(uiReducer, initialUiState);
  const [query, dispatchQuery] = useReducer(galleryQueryReducer, initialGalleryQueryState);
  const [galleries, setGalleries] = useState<ReadonlyMap<GalleryId, Gallery>>(() => new Map());
  const [exploreIds, setExploreIds] = useState<GalleryId[]>([]);
  const [downloadIds, setDownloadIds] = useState<GalleryId[]>([]);
  const [duplicateHiddenGalleryIds, setDuplicateHiddenGalleryIds] = useState<ReadonlySet<GalleryId>>(() => new Set());
  const [downloadsLoading, setDownloadsLoading] = useState(true);
  const [downloadsError, setDownloadsError] = useState<string | null>(null);
  const [searchRefresh, setSearchRefresh] = useState(0);
  const [exploreContextIds, setExploreContextIds] = useState<string[]>([]);
  const [activeExploreContextId, setActiveExploreContextId] = useState<string | null>(null);
  const [downloadsRefresh, setDownloadsRefresh] = useState(0);
  const [favoriteMetadata, setFavoriteMetadata] = useState<ReadonlySet<string>>(() => new Set());
  const [favoriteRecords, setFavoriteRecords] = useState<FavoriteRecord[]>([]);
  const [searchHistory, setSearchHistory] = useState<SearchHistoryEntry[]>([]);
  const [tagCatalogStatus, setTagCatalogStatus] = useState<TagCatalogStatus | undefined>(undefined);
  const [tagCatalogRefreshing, setTagCatalogRefreshing] = useState(false);
  const [privacyModePending, setPrivacyModePending] = useState(false);
  const [tagSuggestions, setTagSuggestions] = useState<TagSuggestion[]>([]);
  const tagSuggestionSequence = useRef(0);
  const [autoFindSnapshot, setAutoFindSnapshot] = useState<AutoFindSnapshot>({ candidates: [], cutoffEvidence: [], truncations: [] });
  const [autoFindIds, setAutoFindIds] = useState<GalleryId[]>([]);
  const [autoFindLoading, setAutoFindLoading] = useState(true);
  const [autoFindError, setAutoFindError] = useState<string | null>(null);
  const [autoFindPending, setAutoFindPending] = useState(false);
  const [duplicateSnapshot, setDuplicateSnapshot] = useState<DuplicateSnapshot | null>(null);
  const [duplicateRun, setDuplicateRun] = useState<DuplicateScanRun | undefined>(undefined);
  const [duplicateLoading, setDuplicateLoading] = useState(true);
  const [duplicateError, setDuplicateError] = useState<string | null>(null);
  const [duplicatePending, setDuplicatePending] = useState(false);
  const [duplicateReviewCandidateId, setDuplicateReviewCandidateId] = useState<string | null>(null);
  const [duplicateReview, setDuplicateReview] = useState<DuplicateReview | null>(null);
  const [duplicateReviewLoading, setDuplicateReviewLoading] = useState(false);
  const [duplicateReviewError, setDuplicateReviewError] = useState<string | null>(null);
  const [duplicateDecisionPending, setDuplicateDecisionPending] = useState(false);
  const [downloadOverlapReviewId, setDownloadOverlapReviewId] = useState<string | null>(null);
  const [downloadOverlapReview, setDownloadOverlapReview] = useState<DownloadOverlapReview | null>(null);
  const [downloadOverlapLoading, setDownloadOverlapLoading] = useState(false);
  const [downloadOverlapError, setDownloadOverlapError] = useState<string | null>(null);
  const [downloadOverlapDecisionPending, setDownloadOverlapDecisionPending] = useState(false);
  const [internalSnapshot, setInternalSnapshot] = useState<InternalDuplicateSnapshot>({ groups: [], quarantineRecords: [], skips: [] });
  const [internalRun, setInternalRun] = useState<InternalScanRun | undefined>(undefined);
  const [internalArtifactProgress, setInternalArtifactProgress] = useState<InternalArtifactScanProgress | null>(null);
  const [internalLoading, setInternalLoading] = useState(true);
  const [internalError, setInternalError] = useState<string | null>(null);
  const [internalPending, setInternalPending] = useState(false);
  const [internalReviewEntryId, setInternalReviewEntryId] = useState<string | null>(null);
  const [internalReview, setInternalReview] = useState<InternalDuplicateReview | null>(null);
  const [internalReviewLoading, setInternalReviewLoading] = useState(false);
  const [internalReviewError, setInternalReviewError] = useState<string | null>(null);
  const [internalPlan, setInternalPlan] = useState<InternalRemovalPlan | null>(null);
  const [toast, setToast] = useState<Toast>(null);
  const [keyboardFocusId, setKeyboardFocusId] = useState<GalleryId | null>(null);
  const [keyboardShortcutsOpen, setKeyboardShortcutsOpen] = useState(false);
  const [tutorialOpen, setTutorialOpen] = useState(() => !isTutorialDismissed());
  const [lastUndoAction, setLastUndoAction] = useState<UndoAction | null>(null);
  const [reconcilingArtifacts, setReconcilingArtifacts] = useState(false);
  const [settingsPreview, setSettingsPreview] = useState<{ maxColumns: number; previewWidth: number } | null>(null);
  const [exitWorkSnapshot, setExitWorkSnapshot] = useState<AppActiveWorkSnapshot | null>(null);
  const [exitStatusError, setExitStatusError] = useState(false);
  const [forceQuitArmed, setForceQuitArmed] = useState(false);
  const [exitActionPending, setExitActionPending] = useState(false);
  const [pendingDownloadEntries, setPendingDownloadEntries] = useState<ReadonlySet<string>>(() => new Set());
  const exitConfirmOpenRef = useRef(false);
  const exitActionPendingRef = useRef(false);
  const exitSnapshotSequence = useRef(0);
  const toastTimer = useRef<number | undefined>(undefined);
  const searchToken = useRef(0);
  const autoFindHydrationToken = useRef(0);
  const duplicateHydrationToken = useRef(0);
  const duplicateReviewToken = useRef(0);
  const duplicateRunRef = useRef<DuplicateScanRun | undefined>(undefined);
  const duplicateSnapshotRef = useRef<DuplicateSnapshot | null>(null);
  const duplicatePendingRef = useRef(false);
  const duplicateDecisionPendingRef = useRef(false);
  const downloadOverlapReviewToken = useRef(0);
  const downloadOverlapDecisionPendingRef = useRef(false);
  const internalHydrationToken = useRef(0);
  const internalReviewToken = useRef(0);
  const internalRunRef = useRef<InternalScanRun | undefined>(undefined);
  const internalArtifactProgressRef = useRef<InternalArtifactScanProgress | null>(null);
  const internalPendingRef = useRef(false);
  const downloadHydrationToken = useRef(0);
  const queueRequestSequence = useRef(0);
  const pendingDownloadEntriesRef = useRef(new Set<string>());
  const undoPendingRef = useRef(false);
  const pendingFavoriteTokens = useRef(new Set<string>());
  const openingDownloadFolders = useRef(new Set<string>());
  const hydratedDetails = useRef(new Set<GalleryId>());
  const galleriesRef = useRef(galleries);
  const visibleIdsRef = useRef<GalleryId[]>([]);
  const activityOpener = useRef<HTMLElement | null>(null);
  const galleryViewport = useRef<HTMLElement>(null);
  const explorePageSession = useRef<ExplorePageSession | null>(null);
  const exploreContexts = useRef(new Map<string, ExploreContext>());
  const exploreContextIdsRef = useRef<string[]>([]);
  const activeExploreContextIdRef = useRef<string | null>(null);
  const exploreContextSequence = useRef(0);
  const exploreContextAccessSequence = useRef(0);
  const queryRef = useRef(query);
  const exploreIdsRef = useRef(exploreIds);
  const keyboardFocusIdRef = useRef(keyboardFocusId);
  const uiRef = useRef(ui);
  const pendingExploreSearch = useRef<{
    generation: number;
    token: number;
    contextId: string;
    request: SearchRequest;
  } | null>(null);
  const exploreSearchGeneration = useRef(0);
  const exploreNavigationToken = useRef(0);
  const exploreRestoreFrame = useRef<number | null>(null);
  exploreContextIdsRef.current = exploreContextIds;
  activeExploreContextIdRef.current = activeExploreContextId;
  queryRef.current = query;
  exploreIdsRef.current = exploreIds;
  keyboardFocusIdRef.current = keyboardFocusId;
  uiRef.current = ui;

  const createExplorePageSession = useCallback(() => {
    const subscribePage = (page: GalleryPage, resolvedOnly: boolean) => {
      const releases = page.items.flatMap((item, index) => {
        const request = {
          key: {
            kind: "gallery-cover" as const,
            galleryId: item.id,
            ...(item.thumbnailKey?.trim() ? { sourceKey: item.thumbnailKey.trim() } : {}),
            fallback: { kind: "fixture-sheet-cell" as const, index: index % 6 },
          },
          consumer: "explore" as const,
          priority: "prefetch" as const,
        };
        if (resolvedOnly && thumbnailClient.getSnapshot(request.key).status !== "resolved") return [];
        return [thumbnailClient.subscribe(request, () => undefined)];
      });
      return () => releases.forEach((release) => release());
    };
    return new ExplorePageSession({
      fetchPage: (queryId, page, requestId) => backend.searchPageGet(queryId, page, requestId),
      cancelPage: (requestId) => backend.searchPageCancel(requestId),
      warmPage: (page) => subscribePage(page, false),
      retainPage: (page) => subscribePage(page, true),
    });
  }, [thumbnailClient]);

  if (!explorePageSession.current) {
    explorePageSession.current = createExplorePageSession();
  }
  const { settings, loading: settingsLoading, error: settingsError, save: saveSettings } = useSettings();
  const appUpdater = useAppUpdater(backend.runtime);
  const [collapsedGroupKeys, setCollapsedGroupKeys] = useState<ReadonlySet<string>>(() => new Set());
  const listPreferencePersistence = useRef<{ active: boolean; queued: SettingsPatch | null }>({ active: false, queued: null });

  useEffect(() => {
    document.documentElement.dataset.privacyMode = settings.privacyMode ? "on" : "off";
    return () => {
      delete document.documentElement.dataset.privacyMode;
    };
  }, [settings.privacyMode]);
  const maximumColumns = settingsPreview?.maxColumns ?? settings.maxColumns;
  const previewWidth = settingsPreview?.previewWidth ?? settings.previewWidth;
  const [galleryColumns, setGalleryColumns] = useState(1);

  useWindowPlacement();

  useEffect(() => () => {
    for (const context of exploreContexts.current.values()) context.session.clear();
    if (exploreContexts.current.size === 0) explorePageSession.current?.clear();
    if (exploreRestoreFrame.current !== null) window.cancelAnimationFrame(exploreRestoreFrame.current);
  }, []);

  const showToast = useCallback((message: string) => {
    window.clearTimeout(toastTimer.current);
    setToast({ id: Date.now(), message });
    toastTimer.current = window.setTimeout(() => setToast(null), 2400);
  }, []);

  const replaceExploreContextIds = useCallback((ids: string[]) => {
    exploreContextIdsRef.current = ids;
    setExploreContextIds(ids);
  }, []);

  const replaceActiveExploreContextId = useCallback((id: string | null) => {
    activeExploreContextIdRef.current = id;
    setActiveExploreContextId(id);
  }, []);

  const ensureActiveExploreContext = useCallback((): ExploreContext => {
    const activeId = activeExploreContextIdRef.current;
    const active = activeId ? exploreContexts.current.get(activeId) : undefined;
    if (active) return active;

    const id = `explore-context-${++exploreContextSequence.current}`;
    const context: ExploreContext = {
      id,
      label: "전체 탐색",
      root: true,
      session: explorePageSession.current ?? createExplorePageSession(),
      request: null,
      requestKey: null,
      displayValue: uiRef.current.search.explore.committed,
      languages: [...uiRef.current.search.explore.languages],
      sort: uiRef.current.exploreSort,
      query: queryRef.current,
      exploreIds: [...exploreIdsRef.current],
      scrollTop: galleryViewport.current?.scrollTop ?? 0,
      keyboardFocusId: keyboardFocusIdRef.current,
      selectionIds: [...uiRef.current.selection.ids],
      selectionAnchorId: uiRef.current.selection.anchorId,
      lastAccessed: ++exploreContextAccessSequence.current,
    };
    explorePageSession.current = context.session;
    exploreContexts.current.set(id, context);
    replaceExploreContextIds([id]);
    replaceActiveExploreContextId(id);
    return context;
  }, [createExplorePageSession, replaceActiveExploreContextId, replaceExploreContextIds]);

  const snapshotActiveExploreContext = useCallback((park = true): ExploreContext | null => {
    const activeId = activeExploreContextIdRef.current;
    const context = activeId ? exploreContexts.current.get(activeId) : undefined;
    if (!context) return null;

    const currentQuery = queryRef.current.phase === "loading-page" && queryRef.current.page
      ? { ...queryRef.current, phase: "ready" as const, pendingPage: null, error: null }
      : queryRef.current.phase === "submitting"
        ? {
          ...initialGalleryQueryState,
          phase: "error" as const,
          submitToken: queryRef.current.submitToken,
          error: {
            code: "SEARCH_CONTEXT_PAUSED",
            message: "다른 탐색으로 이동해 검색이 중단되었습니다. 다시 시도해 주세요.",
            retryable: true,
            action: "retry" as const,
          },
        }
        : queryRef.current;
    const viewportScroll = uiRef.current.view === "explore"
      ? galleryViewport.current?.scrollTop ?? context.scrollTop
      : context.scrollTop;
    if (currentQuery.page && uiRef.current.view === "explore") {
      context.session.recordScroll(currentQuery.page.page, viewportScroll);
    }
    context.query = currentQuery;
    context.exploreIds = [...exploreIdsRef.current];
    context.displayValue = uiRef.current.search.explore.committed;
    context.languages = [...uiRef.current.search.explore.languages];
    context.sort = uiRef.current.exploreSort;
    context.scrollTop = viewportScroll;
    if (uiRef.current.view === "explore") {
      context.keyboardFocusId = keyboardFocusIdRef.current;
      context.selectionIds = [...uiRef.current.selection.ids];
      context.selectionAnchorId = uiRef.current.selection.anchorId;
    }
    context.lastAccessed = ++exploreContextAccessSequence.current;
    if (park) context.session.park();
    return context;
  }, []);

  const restoreExploreContext = useCallback((context: ExploreContext) => {
    if (exploreRestoreFrame.current !== null) {
      window.cancelAnimationFrame(exploreRestoreFrame.current);
      exploreRestoreFrame.current = null;
    }
    context.lastAccessed = ++exploreContextAccessSequence.current;
    explorePageSession.current = context.session;
    context.session.resume();
    replaceActiveExploreContextId(context.id);
    queryRef.current = context.query;
    exploreIdsRef.current = [...context.exploreIds];
    keyboardFocusIdRef.current = context.keyboardFocusId;
    dispatch({ type: "navigate", view: "explore" });
    dispatch({ type: "search.languages", view: "explore", languages: [...context.languages] });
    dispatch({ type: "sort.set", sort: context.sort });
    dispatch({ type: "search.commit", view: "explore", value: context.displayValue });
    dispatch({
      type: "selection.restore",
      ids: [...context.selectionIds],
      anchorId: context.selectionAnchorId,
    });
    dispatchQuery({ type: "restore", state: context.query });
    setExploreIds([...context.exploreIds]);
    setKeyboardFocusId(context.keyboardFocusId);
    exploreRestoreFrame.current = window.requestAnimationFrame(() => {
      if (activeExploreContextIdRef.current === context.id && galleryViewport.current) {
        galleryViewport.current.scrollTop = context.scrollTop;
      }
      context.session.releaseRetainedPage();
      exploreRestoreFrame.current = null;
    });
  }, [replaceActiveExploreContextId]);

  const activateExploreContext = useCallback((id: string) => {
    const target = exploreContexts.current.get(id);
    if (!target) return;
    if (activeExploreContextIdRef.current === id) {
      if (uiRef.current.view !== "explore") restoreExploreContext(target);
      return;
    }
    searchToken.current += 1;
    exploreNavigationToken.current += 1;
    snapshotActiveExploreContext(true);
    restoreExploreContext(target);
  }, [restoreExploreContext, snapshotActiveExploreContext]);

  const closeExploreContext = useCallback((id: string) => {
    const context = exploreContexts.current.get(id);
    if (!context || context.root) return;
    const ids = exploreContextIdsRef.current;
    const closingIndex = ids.indexOf(id);
    const nextIds = ids.filter((contextId) => contextId !== id);
    searchToken.current += 1;
    exploreNavigationToken.current += 1;
    context.session.clear();
    exploreContexts.current.delete(id);
    replaceExploreContextIds(nextIds);
    if (activeExploreContextIdRef.current !== id) return;
    const fallbackId = nextIds[Math.max(0, closingIndex - 1)] ?? nextIds[0];
    const fallback = fallbackId ? exploreContexts.current.get(fallbackId) : undefined;
    if (fallback) restoreExploreContext(fallback);
    else replaceActiveExploreContextId(null);
  }, [replaceActiveExploreContextId, replaceExploreContextIds, restoreExploreContext]);

  const navigateView = useCallback((view: ViewId) => {
    if (view === uiRef.current.view) return;
    if (uiRef.current.view === "explore") snapshotActiveExploreContext(true);
    if (view === "explore") {
      const activeId = activeExploreContextIdRef.current;
      const active = activeId ? exploreContexts.current.get(activeId) : undefined;
      if (active) {
        restoreExploreContext(active);
        return;
      }
    }
    dispatch({ type: "navigate", view });
  }, [restoreExploreContext, snapshotActiveExploreContext]);

  const loadExplorationExclusionsAndSync = useCallback(async () => {
    const result = await loadExplorationExclusions();
    if (result.ok) {
      setDuplicateHiddenGalleryIds(new Set(result.data
        .filter((item) => item.reasons.some((reason) => reason.kind === "duplicate_hidden"))
        .map((item) => item.galleryId)));
    }
    return result;
  }, []);

  const restoreExplorationExclusionsAndSync = useCallback(async (galleryIds: GalleryId[]) => {
    const result = await restoreExplorationExclusions(galleryIds);
    if (result.ok) {
      const restored = new Set(result.data.restoredGalleryIds);
      setDuplicateHiddenGalleryIds((current) => new Set([...current].filter((id) => !restored.has(id))));
    }
    return result;
  }, []);

  useEffect(() => {
    void loadExplorationExclusionsAndSync().catch(() => undefined);
  }, [downloadsRefresh, loadExplorationExclusionsAndSync]);

  const closeTutorial = useCallback((doNotShowAgain: boolean) => {
    if (doNotShowAgain) setTutorialDismissed(true);
    setTutorialOpen(false);
  }, []);

  const persistListPreferences = useCallback((patch: SettingsPatch) => {
    listPreferencePersistence.current.queued = {
      ...(listPreferencePersistence.current.queued ?? {}),
      ...patch,
    };
    if (listPreferencePersistence.current.active) return;
    listPreferencePersistence.current.active = true;

    void (async () => {
      try {
        while (listPreferencePersistence.current.queued !== null) {
          const desired = listPreferencePersistence.current.queued;
          listPreferencePersistence.current.queued = null;
          const current = await backend.settingsGet();
          if (!current.ok) {
            showToast(`목록 표시 설정을 저장하지 못했습니다. ${current.error.message}`);
            continue;
          }
          const saved = await backend.settingsUpdate(desired, current.data.revision);
          if (!saved.ok) showToast(`목록 표시 설정을 저장하지 못했습니다. ${saved.error.message}`);
        }
      } catch {
        showToast("목록 표시 설정을 저장하지 못했습니다.");
      } finally {
        listPreferencePersistence.current.active = false;
      }
    })();
  }, [showToast]);

  useEffect(() => {
    if (listPreferencePersistence.current.active || listPreferencePersistence.current.queued !== null) return;
    dispatch({ type: "grouping.set", view: "auto-find", grouping: settings.autoFindGrouping });
    dispatch({ type: "grouping.set", view: "downloads", grouping: settings.downloadsGrouping });
  }, [settings.autoFindGrouping, settings.downloadsGrouping]);

  const persistGalleryGrouping = useCallback((view: "auto-find" | "downloads", grouping: GalleryGrouping) => {
    dispatch({ type: "grouping.set", view, grouping });
    const patch: SettingsPatch = view === "auto-find"
      ? { autoFindGrouping: grouping }
      : { downloadsGrouping: grouping };
    persistListPreferences(patch);
  }, [persistListPreferences]);

  useEffect(() => {
    if (listPreferencePersistence.current.active || listPreferencePersistence.current.queued !== null) return;
    setCollapsedGroupKeys(new Set(settings.collapsedGroupKeys));
  }, [settings.collapsedGroupKeys]);

  const persistCollapsedGroupKeys = useCallback((nextKeys: ReadonlySet<string>) => {
    const serialized = [...nextKeys].sort((left, right) => left.localeCompare(right));
    setCollapsedGroupKeys(new Set(serialized));
    persistListPreferences({ collapsedGroupKeys: serialized });
  }, [persistListPreferences]);

  const runMaintenance = useCallback(async (action: MaintenanceAction): Promise<ApiResult<MaintenanceResult>> => {
    try {
      const preview = await backend.maintenancePreview(action);
      if (!preview.ok) return preview;
      const result = await backend.maintenanceExecute(preview.data.previewId, action);
      if (!result.ok) return result;
      if (action.kind === "quickRepair" || (action.kind === "rebuildLibrary" && action.rebuildThumbnailData)) {
        thumbnailClient.clearRetainedCache();
        for (const context of exploreContexts.current.values()) context.session.clear();
        if (exploreContexts.current.size === 0) explorePageSession.current?.clear();
      }
      return result;
    } catch {
      return {
        ok: false,
        error: {
          code: "MAINTENANCE_FAILED",
          message: "유지보수 작업을 완료하지 못했습니다.",
          retryable: true,
          action: "retry",
        },
      };
    }
  }, [thumbnailClient]);

  const applyAutoFindSnapshot = useCallback((snapshot: AutoFindSnapshot) => {
    setAutoFindSnapshot(snapshot);
    setAutoFindIds(snapshot.candidates.map((candidate) => candidate.id));
    setGalleries((current) => mergeGalleryPage(current, {
      page: 1,
      totalPages: snapshot.candidates.length ? 1 : 0,
      items: snapshot.candidates,
    }).galleries);
  }, []);

  const hydrateFavorites = useCallback(async () => {
    try {
      const result = await backend.favoritesList();
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      setFavoriteRecords(result.data);
      setFavoriteMetadata(new Set(result.data.map(favoriteToken)));
    } catch {
      showToast("즐겨찾기 목록을 불러오지 못했습니다.");
    }
  }, [showToast]);

  const hydrateSearchHistory = useCallback(async () => {
    try {
      const result = await backend.searchHistoryList(20);
      if (result.ok) setSearchHistory(result.data);
    } catch {
      // Search history is an enhancement; a transient failure must not block searching.
    }
  }, []);

  const hydrateTagCatalogStatus = useCallback(async () => {
    try { const result = await backend.tagCatalogStatus(); if (result.ok) setTagCatalogStatus(result.data); } catch { /* catalog is optional until manually refreshed */ }
  }, []);

  const refreshTagCatalog = useCallback(async () => {
    setTagCatalogRefreshing(true);
    try {
      const result = await backend.tagCatalogRefresh();
      if (result.ok) { setTagCatalogStatus(result.data); showToast(`검색 자동완성 최신화 완료 · 작가 ${result.data.artistCount.toLocaleString()} · 그룹 ${result.data.groupCount.toLocaleString()} · 태그 ${result.data.neutralCount.toLocaleString()} · F ${result.data.femaleCount.toLocaleString()} · M ${result.data.maleCount.toLocaleString()}`); }
      else { showToast(result.error.details?.catalogRetained ? "자동완성 최신화에 실패했지만 기존 데이터를 유지했습니다." : result.error.message); }
    } catch { showToast(tagCatalogStatus?.entryCount ? "자동완성 최신화에 실패했지만 기존 데이터를 유지했습니다." : "자동완성 데이터가 없습니다."); }
    finally { setTagCatalogRefreshing(false); }
  }, [showToast, tagCatalogStatus?.entryCount]);

  const hydrateAutoFind = useCallback(async (showLoading = false) => {
    const token = ++autoFindHydrationToken.current;
    if (showLoading) setAutoFindLoading(true);
    try {
      const result = await backend.autoFindSnapshot();
      if (token !== autoFindHydrationToken.current) return;
      if (!result.ok) {
        setAutoFindError(result.error.message);
        return;
      }
      setAutoFindError(null);
      applyAutoFindSnapshot(result.data);
    } catch {
      if (token === autoFindHydrationToken.current) {
        setAutoFindError("자동 탐색 backend에 연결하지 못했습니다.");
      }
    } finally {
      if (token === autoFindHydrationToken.current) setAutoFindLoading(false);
    }
  }, [applyAutoFindSnapshot]);

  const hydrateDuplicateSnapshot = useCallback(async (showLoading = false) => {
    const token = ++duplicateHydrationToken.current;
    if (showLoading) setDuplicateLoading(true);
    try {
      const result = await backend.duplicateSnapshot();
      if (token !== duplicateHydrationToken.current) return;
      if (!result.ok) {
        setDuplicateError(result.error.message);
        return;
      }
      setDuplicateError(null);
      const merged = mergeHydratedDuplicateSnapshot(
        duplicateSnapshotRef.current,
        result.data,
        duplicateRunRef.current,
      );
      duplicateSnapshotRef.current = merged;
      duplicateRunRef.current = merged.run;
      setDuplicateRun(merged.run);
      setDuplicateSnapshot(merged);
    } catch {
      if (token === duplicateHydrationToken.current) {
        setDuplicateError("작품 중복 검사 backend에 연결하지 못했습니다.");
      }
    } finally {
      if (token === duplicateHydrationToken.current) setDuplicateLoading(false);
    }
  }, []);

  const hydrateInternalArtifactProgress = useCallback(async (expectedRunId: string) => {
    try {
      const result = await backend.internalDuplicateActiveArtifact();
      if (!result.ok) return;
      const progress = result.data;
      const run = internalRunRef.current;
      // A lookup started for an older run may resolve after cancel/restart. It must
      // never clear or replace the newer run's event-driven progress.
      if (run?.state !== "running" || run.runId !== expectedRunId) return;
      // The worker can publish an event between the command snapshot and Promise
      // resolution. A null/mismatched lookup is therefore not evidence that the
      // already-received progress should be cleared.
      if (!progress || progress.runId !== expectedRunId) return;
      const current = internalArtifactProgressRef.current;
      if (current?.runId === progress.runId && current.sequence >= progress.sequence) return;
      internalArtifactProgressRef.current = progress;
      setInternalArtifactProgress(progress);
    } catch {
      // The aggregate scan state remains authoritative; a transient activity lookup failure
      // must not turn a running scan into a UI error.
    }
  }, []);

  const hydrateInternalSnapshot = useCallback(async (showLoading = false) => {
    const token = ++internalHydrationToken.current;
    if (showLoading) setInternalLoading(true);
    try {
      const result = await backend.internalDuplicateSnapshot();
      if (token !== internalHydrationToken.current) return;
      if (!result.ok) {
        setInternalError(result.error.message);
        return;
      }
      const incoming = result.data.run;
      const current = internalRunRef.current;
      const stale = Boolean(
        incoming && current && (
          (incoming.runId === current.runId && incoming.revision < current.revision)
          || (incoming.runId !== current.runId && incoming.startedAt < current.startedAt)
        ),
      );
      if (stale) return;
      internalRunRef.current = incoming;
      setInternalRun(incoming);
      setInternalSnapshot(result.data);
      if (incoming?.state === "running") void hydrateInternalArtifactProgress(incoming.runId);
      else {
        internalArtifactProgressRef.current = null;
        setInternalArtifactProgress(null);
      }
      setInternalError(null);
    } catch {
      if (token === internalHydrationToken.current) {
        setInternalError("내부 중복 검사 backend에 연결하지 못했습니다.");
      }
    } finally {
      if (token === internalHydrationToken.current) setInternalLoading(false);
    }
  }, [hydrateInternalArtifactProgress]);

  const beginDownloadMutation = useCallback((entryId: string): boolean => {
    if (pendingDownloadEntriesRef.current.has(entryId)) return false;
    pendingDownloadEntriesRef.current.add(entryId);
    setPendingDownloadEntries(new Set(pendingDownloadEntriesRef.current));
    return true;
  }, []);

  const finishDownloadMutation = useCallback((entryId: string) => {
    pendingDownloadEntriesRef.current.delete(entryId);
    setPendingDownloadEntries(new Set(pendingDownloadEntriesRef.current));
  }, []);

  useEffect(() => () => window.clearTimeout(toastTimer.current), []);

  useEffect(() => {
    void hydrateFavorites();
    void hydrateSearchHistory();
    void hydrateTagCatalogStatus();
    void hydrateAutoFind(true);
    void hydrateDuplicateSnapshot(true);
    void hydrateInternalSnapshot(true);
  }, [hydrateAutoFind, hydrateDuplicateSnapshot, hydrateFavorites, hydrateInternalSnapshot, hydrateSearchHistory, hydrateTagCatalogStatus]);

  useLayoutEffect(() => {
    const viewport = galleryViewport.current;
    if (!viewport) return;
    const update = () => {
      const next = resolveGalleryColumns(viewport.clientWidth, maximumColumns, previewWidth);
      setGalleryColumns((current) => current === next ? current : next);
    };
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [maximumColumns, previewWidth, settingsLoading]);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void backend.on("download:changed", (event: DownloadChangedEvent) => {
      setGalleries((current) => {
        const projection = applyDownloadChanged(current, event);
        return projection.galleries;
      });
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unsubscribe = cleanup;
    }).catch(() => {
      if (!disposed) showToast("작업 상태 event stream에 연결하지 못했습니다.");
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [showToast]);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void backend.on("auto-find:changed", (run) => {
      setAutoFindSnapshot((current) => {
        if (current.run?.runId === run.runId && current.run.revision > run.revision) return current;
        return { ...current, run };
      });
      void hydrateAutoFind();
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unsubscribe = cleanup;
    }).catch(() => {
      if (!disposed) setAutoFindError("자동 탐색 상태 event stream에 연결하지 못했습니다.");
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [hydrateAutoFind]);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void backend.on("duplicate:changed", (run) => {
      if (!validDuplicateRun(run)) return;
      const previous = duplicateRunRef.current;
      if (!duplicateRunIsNewer(previous, run)) return;
      if (previous?.runId !== run.runId) duplicateHydrationToken.current += 1;
      duplicateRunRef.current = run;
      setDuplicateRun(run);
      if (duplicateSnapshotRef.current) {
        const next = { ...duplicateSnapshotRef.current, run };
        duplicateSnapshotRef.current = next;
        setDuplicateSnapshot(next);
      }
      if (duplicateEventNeedsSnapshot(previous, run)) void hydrateDuplicateSnapshot();
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unsubscribe = cleanup;
    }).catch(() => {
      if (!disposed) setDuplicateError("작품 중복 검사 event stream에 연결하지 못했습니다.");
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [hydrateDuplicateSnapshot]);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void backend.on("internal-duplicate:changed", (run) => {
      const current = internalRunRef.current;
      if (current?.runId === run.runId && current.revision >= run.revision) return;
      if (current?.runId !== run.runId && current && run.startedAt < current.startedAt) return;
      internalRunRef.current = run;
      setInternalRun(run);
      setInternalSnapshot((snapshot) => ({ ...snapshot, run }));
      if (run.state !== "running") {
        internalArtifactProgressRef.current = null;
        setInternalArtifactProgress(null);
        void hydrateInternalSnapshot();
      } else if (current?.runId !== run.runId) {
        internalArtifactProgressRef.current = null;
        setInternalArtifactProgress(null);
        void hydrateInternalArtifactProgress(run.runId);
      }
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unsubscribe = cleanup;
    }).catch(() => {
      if (!disposed) setInternalError("내부 중복 상태 event stream에 연결하지 못했습니다.");
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [hydrateInternalArtifactProgress, hydrateInternalSnapshot]);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void backend.on("internal-duplicate:artifact-progress", (progress) => {
      const run = internalRunRef.current;
      if (run?.state !== "running" || run.runId !== progress.runId) return;
      const current = internalArtifactProgressRef.current;
      if (current?.runId === progress.runId && current.sequence >= progress.sequence) return;
      internalArtifactProgressRef.current = progress;
      setInternalArtifactProgress(progress);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unsubscribe = cleanup;
    }).catch(() => {
      // Run-level state remains available even when this additive progress stream is unavailable.
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, []);

  const startExploreSearch = useCallback((
    sourceRequest: SearchRequest,
    options: { displayValue: string; label?: string },
  ) => {
    const context = ensureActiveExploreContext();
    const request = cloneSearchRequest(sourceRequest);
    const token = ++searchToken.current;
    const generation = ++exploreSearchGeneration.current;
    exploreNavigationToken.current += 1;
    if (exploreRestoreFrame.current !== null) {
      window.cancelAnimationFrame(exploreRestoreFrame.current);
      exploreRestoreFrame.current = null;
    }
    context.session.clear();
    explorePageSession.current = context.session;
    context.request = request;
    context.requestKey = searchRequestKey(request);
    context.displayValue = options.displayValue.trim();
    context.languages = [...request.languages];
    context.sort = request.sort;
    if (!context.root && options.label?.trim()) context.label = options.label.trim();
    context.scrollTop = 0;
    context.keyboardFocusId = null;
    context.selectionIds = [];
    context.selectionAnchorId = null;
    const submitting: GalleryQueryState = {
      phase: "submitting",
      submitToken: token,
      queryId: null,
      page: null,
      pendingPage: null,
      error: null,
    };
    context.query = submitting;
    context.exploreIds = [];
    queryRef.current = submitting;
    exploreIdsRef.current = [];
    keyboardFocusIdRef.current = null;
    pendingExploreSearch.current = { generation, token, contextId: context.id, request };
    dispatch({ type: "selection.clear" });
    dispatchQuery({ type: "restore", state: submitting });
    setExploreIds([]);
    setKeyboardFocusId(null);
    if (uiRef.current.view === "explore" && galleryViewport.current) galleryViewport.current.scrollTop = 0;
    setSearchRefresh(generation);
  }, [ensureActiveExploreContext]);

  useEffect(() => {
    const pending = pendingExploreSearch.current;
    if (!pending || pending.generation !== searchRefresh) return;
    let cancelled = false;
    const { token, contextId, request } = pending;
    void backend.searchSubmit(request).then((result) => {
      const context = exploreContexts.current.get(contextId);
      if (cancelled || token !== searchToken.current || !context || activeExploreContextIdRef.current !== contextId) return;
      if (!result.ok) {
        const failed: GalleryQueryState = {
          ...context.query,
          phase: "error",
          error: result.error,
        };
        context.query = failed;
        queryRef.current = failed;
        dispatchQuery({ type: "restore", state: failed });
        return;
      }
      const ready: GalleryQueryState = {
        phase: "ready",
        submitToken: token,
        queryId: result.data.queryId,
        page: result.data.firstPage,
        pendingPage: null,
        error: null,
      };
      const resultIds = result.data.firstPage.items.map((item) => item.id);
      context.query = ready;
      context.exploreIds = resultIds;
      context.scrollTop = 0;
      queryRef.current = ready;
      exploreIdsRef.current = resultIds;
      dispatchQuery({ type: "restore", state: ready });
      context.session.start(result.data.queryId, result.data.firstPage);
      setExploreIds(resultIds);
      setGalleries((current) => mergeGalleryPage(current, result.data.firstPage).galleries);
      if (uiRef.current.view === "explore") {
        if (galleryViewport.current) galleryViewport.current.scrollTop = 0;
        context.session.prefetchAdjacent();
      } else {
        context.session.park();
      }
      if (request.text.trim() || request.includeTags.length || request.excludeTags.length) {
        void hydrateSearchHistory();
      }
    }).catch(() => {
      const context = exploreContexts.current.get(contextId);
      if (!cancelled && token === searchToken.current && context && activeExploreContextIdRef.current === contextId) {
        const failed: GalleryQueryState = {
          ...context.query,
          phase: "error",
          error: { code: "BACKEND_UNAVAILABLE", message: "검색 backend에 연결하지 못했습니다.", retryable: true, action: "retry" },
        };
        context.query = failed;
        queryRef.current = failed;
        dispatchQuery({ type: "restore", state: failed });
      }
    }).finally(() => {
      if (pendingExploreSearch.current?.generation === pending.generation) pendingExploreSearch.current = null;
    });
    return () => {
      cancelled = true;
    };
  }, [hydrateSearchHistory, searchRefresh]);

  useEffect(() => {
    let cancelled = false;
    const token = ++downloadHydrationToken.current;
    setDownloadsLoading(true);
    setDownloadsError(null);
    void (async () => {
      const entries: DownloadEntry[] = [];
      let page = 1;
      let totalItems = 0;
      do {
        const result = await backend.downloadEntriesList({ page, pageSize: 200 });
        if (cancelled || token !== downloadHydrationToken.current) return;
        if (!result.ok) {
          setDownloadsError(result.error.message);
          return;
        }
        if (result.data.entries.length === 0 && entries.length < result.data.totalItems) {
          throw new Error("download pagination ended before totalItems");
        }
        entries.push(...result.data.entries);
        totalItems = result.data.totalItems;
        page += 1;
      } while (entries.length < totalItems);

      setGalleries((current) => mergeDownloadEntries(current, entries));
      const galleryIds = [...new Set(entries.map((entry) => entry.galleryId))];
      setDownloadIds(galleryIds);
      setDownloadsLoading(false);

      for (let offset = 0; offset < galleryIds.length; offset += 32) {
        const detailResults = await Promise.allSettled(
          galleryIds.slice(offset, offset + 32).map((id) => backend.galleryDetailGet(id)),
        );
        if (cancelled || token !== downloadHydrationToken.current) return;
        setGalleries((current) => {
          let next: ReadonlyMap<GalleryId, Gallery> = current;
          for (const detailResult of detailResults) {
            if (detailResult.status === "fulfilled" && detailResult.value.ok) {
              hydratedDetails.current.add(detailResult.value.data.id);
              next = mergeGalleryDetail(next, detailResult.value.data);
            }
          }
          return next;
        });
      }
    })().catch(() => {
      if (!cancelled && token === downloadHydrationToken.current) {
        setDownloadsError("다운로드 목록 backend에 연결하지 못했습니다.");
      }
    }).finally(() => {
      if (!cancelled && token === downloadHydrationToken.current) setDownloadsLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [downloadsRefresh]);

  const displayGalleries = useMemo<ReadonlyMap<GalleryId, Gallery>>(() => {
    const next = new Map<GalleryId, Gallery>();
    galleries.forEach((gallery, id) => {
      const favorite = favoriteMetadata.has(`artist:${normalizeMetadataToken(gallery.artist)}`);
      next.set(id, gallery.favorite === favorite ? gallery : { ...gallery, favorite });
    });
    return next;
  }, [favoriteMetadata, galleries]);
  const favoriteMetadataForDisplay = useMemo<ReadonlySet<string>>(() => {
    const next = new Set(favoriteMetadata);
    galleries.forEach((gallery) => {
      if (gallery.group) {
        const token = `group:${gallery.group}`;
        if (favoriteMetadata.has(normalizeMetadataToken(token))) next.add(token);
      }
      gallery.tags.forEach((tag) => {
        if (favoriteMetadata.has(normalizeMetadataToken(tag))) next.add(tag);
      });
      (gallery.series ?? []).forEach((series) => {
        const token = `series:${series}`;
        if (favoriteMetadata.has(normalizeMetadataToken(token))) next.add(token);
      });
      (gallery.characters ?? []).forEach((character) => {
        const token = `character:${character}`;
        if (favoriteMetadata.has(normalizeMetadataToken(token))) next.add(token);
      });
    });
    return next;
  }, [favoriteMetadata, galleries]);

  const scopedGalleries = useMemo(() => {
    const ids = ui.view === "explore" ? exploreIds : ui.view === "downloads" ? downloadIds : autoFindIds;
    return ids.flatMap((id) => {
      const gallery = displayGalleries.get(id);
      return gallery ? [gallery] : [];
    });
  }, [autoFindIds, displayGalleries, downloadIds, exploreIds, ui.view]);
  const visible = useMemo(() => visibleGalleries(ui, scopedGalleries), [ui, scopedGalleries]);
  const actionableVisibleIds = useMemo(
    () => visible
      .filter((gallery) => gallery.download?.state !== "quarantined"
        && !(ui.view === "explore" && duplicateHiddenGalleryIds.has(gallery.id)))
      .map((gallery) => gallery.id),
    [duplicateHiddenGalleryIds, ui.view, visible],
  );
  galleriesRef.current = displayGalleries;
  visibleIdsRef.current = actionableVisibleIds;
  const allGalleries = useMemo(() => [...displayGalleries.values()], [displayGalleries]);
  const duplicateCandidateCounts = useMemo(() => {
    const counts = new Map<GalleryId, number>();
    for (const candidate of duplicateSnapshot?.candidates ?? []) {
      counts.set(candidate.parent.galleryId, (counts.get(candidate.parent.galleryId) ?? 0) + 1);
      counts.set(candidate.candidate.galleryId, (counts.get(candidate.candidate.galleryId) ?? 0) + 1);
    }
    return counts;
  }, [duplicateSnapshot?.candidates]);
  const internalDuplicateResultCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const group of internalSnapshot.groups) {
      const key = `${group.entryId}\u0000${group.galleryId}`;
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return counts;
  }, [internalSnapshot.groups]);
  const autoFindCount = useMemo(
    () => autoFindIds.filter((id) => displayGalleries.get(id)?.download?.state !== "quarantined").length,
    [autoFindIds, displayGalleries],
  );
  const attentionCount = useMemo(
    () => allGalleries.filter((gallery) => !duplicateHiddenGalleryIds.has(gallery.id)
      && ["failed", "interrupted", "review_required"].includes(gallery.download?.state ?? "")).length,
    [allGalleries, duplicateHiddenGalleryIds],
  );
  const activeDownloadCount = useMemo(
    () => allGalleries.filter((gallery) => gallery.download && activeDownloadStates.has(gallery.download.state)).length,
    [allGalleries],
  );
  const refreshExitWorkSnapshot = useCallback(async (armForceOnFailure = false): Promise<AppActiveWorkSnapshot | null> => {
    const sequence = ++exitSnapshotSequence.current;
    try {
      const result = await backend.appActiveWorkSnapshot();
      if (sequence !== exitSnapshotSequence.current || !exitConfirmOpenRef.current) return null;
      if (result.ok) {
        setExitWorkSnapshot(result.data);
        setExitStatusError(false);
        setForceQuitArmed(false);
        return result.data;
      } else {
        setExitWorkSnapshot(null);
        setExitStatusError(true);
        setForceQuitArmed(armForceOnFailure);
      }
    } catch {
      if (sequence !== exitSnapshotSequence.current || !exitConfirmOpenRef.current) return null;
      setExitWorkSnapshot(null);
      setExitStatusError(true);
      setForceQuitArmed(armForceOnFailure);
    }
    return null;
  }, []);
  const openExitConfirm = useCallback(() => {
    if (exitConfirmOpenRef.current || exitActionPendingRef.current) return;
    exitConfirmOpenRef.current = true;
    setExitWorkSnapshot(null);
    setExitStatusError(false);
    setForceQuitArmed(false);
    exitActionPendingRef.current = false;
    setExitActionPending(false);
    dispatch({ type: "overlay.exit", open: true });
  }, []);
  const closeExitConfirm = useCallback(() => {
    if (exitActionPendingRef.current) return;
    exitSnapshotSequence.current += 1;
    exitConfirmOpenRef.current = false;
    dispatch({ type: "overlay.exit", open: false });
  }, []);

  useEffect(() => {
    if (ui.overlays.exitConfirmOpen) void refreshExitWorkSnapshot();
  }, [refreshExitWorkSnapshot, ui.overlays.exitConfirmOpen]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void backend.on("app:exit-requested", () => openExitConfirm()).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    }).catch(() => {
      showToast("창 닫기 동작을 연결하지 못했습니다.");
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [openExitConfirm, showToast]);

  const selectGallery = useCallback(
    (id: GalleryId, modifiers: { ctrlKey: boolean; shiftKey: boolean }) => {
      dispatch({ type: "selection.click", id, visibleIds: visibleIdsRef.current, ctrl: modifiers.ctrlKey, shift: modifiers.shiftKey });
    },
    [],
  );

  useLayoutEffect(() => {
    dispatch({ type: "selection.retain", ids: actionableVisibleIds });
  }, [actionableVisibleIds]);

  const hydrateDetail = useCallback(async (id: GalleryId) => {
    if (hydratedDetails.current.has(id)) return;
    try {
      const result = await backend.galleryDetailGet(id);
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      hydratedDetails.current.add(id);
      setGalleries((current) => mergeGalleryDetail(current, result.data));
    } catch {
      showToast("상세 정보를 불러오지 못했습니다.");
    }
  }, [showToast]);
  const openDetail = useCallback((id: GalleryId) => {
    dispatch({ type: "detail.open", id });
    void hydrateDetail(id);
  }, [hydrateDetail]);
  const openRelatedDetail = useCallback((id: GalleryId, parentId: GalleryId) => {
    dispatch({ type: "detail.open", id, parentId });
    void hydrateDetail(id);
  }, [hydrateDetail]);
  const hydrateDuplicateReview = useCallback(async (candidateId: string) => {
    const token = ++duplicateReviewToken.current;
    setDuplicateReviewLoading(true);
    setDuplicateReviewError(null);
    try {
      const result = await backend.duplicateReviewGet(candidateId);
      if (token !== duplicateReviewToken.current) return;
      if (!result.ok) {
        setDuplicateReviewError(result.error.message);
        return;
      }
      setDuplicateReview(result.data);
    } catch {
      if (token === duplicateReviewToken.current) {
        setDuplicateReviewError("중복 검토 backend에 연결하지 못했습니다.");
      }
    } finally {
      if (token === duplicateReviewToken.current) setDuplicateReviewLoading(false);
    }
  }, []);
  const hydrateDownloadOverlapReview = useCallback(async (reviewId: string) => {
    const token = ++downloadOverlapReviewToken.current;
    setDownloadOverlapLoading(true);
    setDownloadOverlapError(null);
    try {
      const result = await backend.downloadOverlapReviewGet(reviewId);
      if (token !== downloadOverlapReviewToken.current) return;
      if (!result.ok) {
        setDownloadOverlapError(result.error.message);
        return;
      }
      setDownloadOverlapReview(result.data);
    } catch {
      if (token === downloadOverlapReviewToken.current) {
        setDownloadOverlapError("다운로드 판본 검토 backend에 연결하지 못했습니다.");
      }
    } finally {
      if (token === downloadOverlapReviewToken.current) setDownloadOverlapLoading(false);
    }
  }, []);
  const openReview = useCallback((id: GalleryId) => {
    const download = displayGalleries.get(id)?.download;
    if (download?.state === "review_required" && download.reviewKind === "gallery_duplicate") {
      if (!download.reviewId) {
        showToast("다운로드 판본 검토 ID가 없어 상태를 다시 불러옵니다.");
        setDownloadsRefresh((value) => value + 1);
        return;
      }
      setDuplicateReviewCandidateId(null);
      setDuplicateReview(null);
      setDownloadOverlapReviewId(download.reviewId);
      setDownloadOverlapReview(null);
      setDownloadOverlapError(null);
      dispatch({ type: "overlay.review", galleryId: id });
      void hydrateDownloadOverlapReview(download.reviewId);
      return;
    }
    const candidate = duplicateSnapshot?.candidates.find((item) =>
      item.parent.galleryId === id || item.candidate.galleryId === id,
    );
    if (!candidate) {
      showToast("저장된 작품 중복 후보를 찾을 수 없습니다. 중복 검사 결과를 새로 불러옵니다.");
      void hydrateDuplicateSnapshot();
      return;
    }
    setDuplicateReviewCandidateId(candidate.candidateId);
    setDuplicateReview(null);
    setDuplicateReviewError(null);
    dispatch({ type: "overlay.review", galleryId: id });
    void hydrateDuplicateReview(candidate.candidateId);
  }, [displayGalleries, duplicateSnapshot?.candidates, hydrateDownloadOverlapReview, hydrateDuplicateReview, hydrateDuplicateSnapshot, showToast]);
  const closeDuplicateReview = useCallback(() => {
    duplicateReviewToken.current += 1;
    setDuplicateReviewCandidateId(null);
    setDuplicateReview(null);
    setDuplicateReviewError(null);
    setDuplicateReviewLoading(false);
    dispatch({ type: "overlay.review", galleryId: null });
  }, []);
  const closeDownloadOverlapReview = useCallback(() => {
    downloadOverlapReviewToken.current += 1;
    setDownloadOverlapReviewId(null);
    setDownloadOverlapReview(null);
    setDownloadOverlapError(null);
    setDownloadOverlapLoading(false);
    dispatch({ type: "overlay.review", galleryId: null });
  }, []);
  const applyDuplicateDecision = useCallback(async (request: DuplicateDecisionRequest) => {
    if (duplicateDecisionPendingRef.current) return;
    duplicateDecisionPendingRef.current = true;
    setDuplicateDecisionPending(true);
    setDuplicateReviewError(null);
    try {
      const result = await backend.duplicateDecisionApply(request);
      if (!result.ok) {
        if (result.error.code === "REVISION_CONFLICT") {
          await Promise.all([
            hydrateDuplicateReview(request.candidateId),
            hydrateDuplicateSnapshot(),
          ]);
          setDuplicateReviewError("다른 창에서 판정이 변경되어 최신 근거와 이력을 다시 불러왔습니다.");
          return;
        }
        setDuplicateReviewError(result.error.message);
        return;
      }
      setDuplicateReview(result.data);
      await hydrateDuplicateSnapshot();
      setDownloadsRefresh((value) => value + 1);
      showToast("중복 판정을 저장했습니다. 파일은 자동으로 영구 삭제되지 않습니다.");
    } catch {
      setDuplicateReviewError("중복 판정 요청을 backend에 전달하지 못했습니다.");
    } finally {
      duplicateDecisionPendingRef.current = false;
      setDuplicateDecisionPending(false);
    }
  }, [hydrateDuplicateReview, hydrateDuplicateSnapshot, showToast]);
  const applyDownloadOverlapDecision = useCallback(async (request: DownloadOverlapDecisionRequest) => {
    if (downloadOverlapDecisionPendingRef.current) return;
    downloadOverlapDecisionPendingRef.current = true;
    setDownloadOverlapDecisionPending(true);
    setDownloadOverlapError(null);
    try {
      const result = await backend.downloadOverlapDecisionApply(request);
      if (!result.ok) {
        if (result.error.code === "REVISION_CONFLICT") {
          await hydrateDownloadOverlapReview(request.reviewId);
          setDownloadOverlapError("다른 창에서 검토가 변경되어 최신 내용을 다시 불러왔습니다.");
          return;
        }
        setDownloadOverlapError(result.error.message);
        return;
      }
      setDownloadOverlapReview(result.data.review);
      const excludedGalleryId = request.action === "remove_incoming"
        ? result.data.review.incoming.galleryId
        : request.action === "remove_existing_continue"
          ? result.data.review.candidates.find((candidate) => candidate.candidateId === request.candidateId)?.existing.galleryId
          : undefined;
      if (excludedGalleryId !== undefined) {
        setDuplicateHiddenGalleryIds((current) => new Set([...current, excludedGalleryId]));
      }
      setDownloadsRefresh((value) => value + 1);
      if (result.data.resumed || result.data.cancelled) {
        closeDownloadOverlapReview();
        showToast(result.data.cancelled
          ? "신규 앨범 B를 취소했습니다. 기존 앨범 A와 다른 보유 파일은 변경하지 않았습니다."
          : request.action === "remove_existing_continue"
            ? "기존 앨범 A를 제거 처리하고 신규 앨범 B 완료 절차를 다시 시작했습니다. 완료본은 격리되고 검토 중 staging은 취소됩니다."
            : "현재 후보 판정을 저장하고 신규 앨범 B 완료 절차를 다시 시작했습니다.");
      } else {
        showToast(request.action === "remove_existing_continue"
          ? "기존 앨범 A를 제거 처리했습니다. 완료본은 격리되고 검토 중 staging은 취소됩니다. 남은 후보를 검토해 주세요."
          : "현재 후보 판정을 저장했습니다. 남은 후보를 검토해 주세요.");
      }
    } catch {
      setDownloadOverlapError("다운로드 판본 판정을 backend에 전달하지 못했습니다.");
    } finally {
      downloadOverlapDecisionPendingRef.current = false;
      setDownloadOverlapDecisionPending(false);
    }
  }, [closeDownloadOverlapReview, hydrateDownloadOverlapReview, showToast]);

  const hydrateInternalReview = useCallback(async (entryId: string) => {
    const token = ++internalReviewToken.current;
    setInternalReviewLoading(true);
    setInternalReviewError(null);
    try {
      const result = await backend.internalDuplicateReviewGet(entryId);
      if (token !== internalReviewToken.current) return;
      if (!result.ok) {
        setInternalReviewError(result.error.message);
        return;
      }
      setInternalReview(result.data);
    } catch {
      if (token === internalReviewToken.current) {
        setInternalReviewError("내부 중복 검토 backend에 연결하지 못했습니다.");
      }
    } finally {
      if (token === internalReviewToken.current) setInternalReviewLoading(false);
    }
  }, []);

  const openInternalReview = useCallback((entryId: string) => {
    setInternalReviewEntryId(entryId);
    setInternalReview(null);
    setInternalPlan(null);
    setInternalReviewError(null);
    void hydrateInternalReview(entryId);
  }, [hydrateInternalReview]);

  const closeInternalReview = useCallback(() => {
    internalReviewToken.current += 1;
    setInternalReviewEntryId(null);
    setInternalReview(null);
    setInternalPlan(null);
    setInternalReviewError(null);
    setInternalReviewLoading(false);
  }, []);

  const startInternalScan = useCallback(async (requestedEntryIds: string[]) => {
    if (internalPendingRef.current || internalRun?.state === "running") return;
    const entryIds = [...new Set(requestedEntryIds)];
    if (!entryIds.length) {
      showToast("내부 페이지를 검사할 완료 앨범을 선택해 주세요.");
      return;
    }
    internalPendingRef.current = true;
    setInternalPending(true);
    setInternalError(null);
    try {
      const result = await backend.internalDuplicateScanStart({ entryIds });
      if (!result.ok) {
        setInternalError(result.error.message);
        showToast(result.error.message);
        return;
      }
      internalRunRef.current = result.data;
      setInternalRun(result.data);
      setInternalSnapshot((snapshot) => ({ ...snapshot, run: result.data }));
      await hydrateInternalSnapshot();
      await hydrateInternalArtifactProgress(result.data.runId);
      showToast(`선택한 완료 앨범 ${entryIds.length}개의 내부 중복 페이지 검사를 시작했습니다.`);
    } catch {
      const message = "내부 중복 검사를 시작하지 못했습니다.";
      setInternalError(message);
      showToast(message);
    } finally {
      internalPendingRef.current = false;
      setInternalPending(false);
    }
  }, [hydrateInternalArtifactProgress, hydrateInternalSnapshot, internalRun?.state, showToast]);

  const cancelInternalScan = useCallback(async () => {
    if (internalPendingRef.current || internalRun?.state !== "running") return;
    internalPendingRef.current = true;
    setInternalPending(true);
    try {
      const result = await backend.internalDuplicateScanCancel();
      if (!result.ok) {
        setInternalError(result.error.message);
        showToast(result.error.message);
        return;
      }
      internalRunRef.current = result.data;
      setInternalRun(result.data);
      internalArtifactProgressRef.current = null;
      setInternalArtifactProgress(null);
      setInternalSnapshot((snapshot) => ({ ...snapshot, run: result.data }));
      showToast("내부 중복 검사를 취소했습니다. 기존 검토 결과는 유지됩니다.");
    } catch {
      showToast("내부 중복 검사 취소 요청을 전달하지 못했습니다.");
    } finally {
      internalPendingRef.current = false;
      setInternalPending(false);
    }
  }, [internalRun?.state, showToast]);

  const previewInternalRemoval = useCallback(async (request: InternalRemovalPlanRequest) => {
    if (internalPendingRef.current) return;
    internalPendingRef.current = true;
    setInternalPending(true);
    setInternalReviewError(null);
    try {
      const result = await backend.internalRemovalPlan(request);
      if (!result.ok) {
        setInternalReviewError(result.error.message);
        if (result.error.code === "REVISION_CONFLICT") await hydrateInternalReview(request.entryId);
        return;
      }
      setInternalPlan(result.data);
    } catch {
      setInternalReviewError("격리 계획을 계산하지 못했습니다.");
    } finally {
      internalPendingRef.current = false;
      setInternalPending(false);
    }
  }, [hydrateInternalReview]);

  const applyInternalRemoval = useCallback(async (plan: InternalRemovalPlan) => {
    if (internalPendingRef.current) return;
    internalPendingRef.current = true;
    setInternalPending(true);
    setInternalReviewError(null);
    try {
      const result = await backend.internalRemovalApply({
        plan,
        reason: "사용자가 내부 중복 검토에서 명시적으로 격리함",
      });
      if (!result.ok) {
        setInternalReviewError(result.error.message);
        if (result.error.code === "REVISION_CONFLICT" && internalReviewEntryId) {
          await hydrateInternalReview(internalReviewEntryId);
        }
        return;
      }
      setInternalReview(result.data.review);
      setInternalPlan(null);
      await hydrateInternalSnapshot();
      setDownloadsRefresh((value) => value + 1);
      showToast(`${result.data.records.length}개 페이지를 안전 격리했습니다. 영구 삭제되지 않았습니다.`);
    } catch {
      setInternalReviewError("페이지 격리 요청을 완료하지 못했습니다. 앱 재시작 시 안전하게 조정됩니다.");
    } finally {
      internalPendingRef.current = false;
      setInternalPending(false);
    }
  }, [hydrateInternalReview, hydrateInternalSnapshot, internalReviewEntryId, showToast]);

  const undoInternalRemoval = useCallback(async (recordIds: string[]) => {
    if (internalPendingRef.current || !recordIds.length) return;
    internalPendingRef.current = true;
    setInternalPending(true);
    setInternalReviewError(null);
    try {
      const result = await backend.internalRemovalUndo({ recordIds });
      if (!result.ok) {
        setInternalReviewError(result.error.message);
        return;
      }
      setInternalReview(result.data.review);
      await hydrateInternalSnapshot();
      setDownloadsRefresh((value) => value + 1);
      showToast(`${result.data.records.length}개 페이지를 원래 위치로 복원했습니다.`);
    } catch {
      setInternalReviewError("격리 페이지 복원 요청을 완료하지 못했습니다. 앱 재시작 시 안전하게 조정됩니다.");
    } finally {
      internalPendingRef.current = false;
      setInternalPending(false);
    }
  }, [hydrateInternalSnapshot, showToast]);
  const openActivity = useCallback(() => {
    activityOpener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dispatch({ type: "overlay.activity", open: true });
  }, []);
  const closeActivity = useCallback(() => {
    dispatch({ type: "overlay.activity", open: false });
    const target = activityOpener.current;
    activityOpener.current = null;
    window.requestAnimationFrame(() => {
      if (target?.isConnected) target.focus();
      else document.querySelector<HTMLElement>("[aria-controls='activity-panel']")?.focus();
    });
  }, []);
  const openStatusDetail = useCallback((_: GalleryId) => openActivity(), [openActivity]);

  const openArtifact = useCallback(
    async (id: GalleryId) => {
      const gallery = galleriesRef.current.get(id);
      if (!gallery) return;
      if (gallery.download?.state !== "completed") {
        showToast(`${gallery.title}은 아직 실행할 수 있는 완료 파일이 없습니다.`);
        return;
      }
      try {
        const result = await backend.artifactOpenFirst(gallery.download.entryId);
        if (!result.ok) {
          showToast(result.error.message);
          setDownloadsRefresh((value) => value + 1);
        }
      } catch {
        showToast("완료 파일을 Windows 기본 뷰어로 열지 못했습니다.");
      }
    },
    [showToast],
  );

  const openDownloadFolder = useCallback(async (entryId: string) => {
    if (openingDownloadFolders.current.has(entryId)) return;
    openingDownloadFolders.current.add(entryId);
    try {
      const result = await backend.artifactOpenFolder(entryId);
      if (!result.ok) {
        showToast(result.error.code === "FILESYSTEM_MISSING"
          ? "앨범 저장 폴더가 아직 준비되지 않았거나 이동되었습니다. 잠시 후 다시 시도해 주세요."
          : result.error.message);
      }
    } catch {
      showToast("앨범 저장 폴더를 열지 못했습니다.");
    } finally {
      openingDownloadFolders.current.delete(entryId);
    }
  }, [showToast]);

  const startFreshMetadataSearch = useCallback((value: string) => {
    const target = metadataSearchToken(value);
    const kind = searchTokenKind(target.displayToken);
    const request: SearchRequest = target.includeTag
      ? {
        text: "",
        includeTags: [target.includeTag],
        excludeTags: [],
        languages: ui.search.explore.languages,
        sort: ui.exploreSort,
        pageSize: 50,
      }
      : {
        text: target.displayToken,
        includeTags: [],
        excludeTags: [],
        languages: ui.search.explore.languages,
        sort: ui.exploreSort,
        pageSize: 50,
      };
    if (!kind && !target.displayToken) return;

    const key = searchRequestKey(request);
    const existing = [...exploreContexts.current.values()].find((context) => (
      context.requestKey === key && context.query.page !== null
    ));
    if (existing) {
      dispatch({ type: "detail.minimize", minimized: true });
      activateExploreContext(existing.id);
      return;
    }

    const parent = activeExploreContextIdRef.current
      ? exploreContexts.current.get(activeExploreContextIdRef.current)
      : undefined;
    if (parent) {
      snapshotActiveExploreContext(true);
      let ids = [...exploreContextIdsRef.current];
      if (ids.length >= maximumExploreContexts) {
        const oldest = ids
          .map((id) => exploreContexts.current.get(id))
          .filter((context): context is ExploreContext => Boolean(context && !context.root && context.id !== parent.id))
          .sort((left, right) => left.lastAccessed - right.lastAccessed)[0];
        if (oldest) {
          oldest.session.clear();
          exploreContexts.current.delete(oldest.id);
          ids = ids.filter((id) => id !== oldest.id);
        }
      }
      const id = `explore-context-${++exploreContextSequence.current}`;
      const context: ExploreContext = {
        id,
        label: target.displayToken,
        root: false,
        session: createExplorePageSession(),
        request: cloneSearchRequest(request),
        requestKey: key,
        displayValue: target.displayToken,
        languages: [...request.languages],
        sort: request.sort,
        query: initialGalleryQueryState,
        exploreIds: [],
        scrollTop: 0,
        keyboardFocusId: null,
        selectionIds: [],
        selectionAnchorId: null,
        lastAccessed: ++exploreContextAccessSequence.current,
      };
      exploreContexts.current.set(id, context);
      explorePageSession.current = context.session;
      replaceExploreContextIds([...ids, id]);
      replaceActiveExploreContextId(id);
    } else {
      ensureActiveExploreContext();
    }

    dispatch({ type: "navigate", view: "explore" });
    dispatch({ type: "selection.clear" });
    dispatch({ type: "detail.minimize", minimized: true });
    dispatch({ type: "search.languages", view: "explore", languages: [...request.languages] });
    dispatch({ type: "sort.set", sort: request.sort });
    dispatch({ type: "search.commit", view: "explore", value: target.displayToken });
    startExploreSearch(request, { displayValue: target.displayToken, label: target.displayToken });
  }, [
    activateExploreContext,
    createExplorePageSession,
    ensureActiveExploreContext,
    replaceActiveExploreContextId,
    replaceExploreContextIds,
    snapshotActiveExploreContext,
    startExploreSearch,
    ui.exploreSort,
    ui.search.explore.languages,
  ]);

  const searchMetadata = startFreshMetadataSearch;

  const toggleMetadataFavorite = useCallback(async (value: string) => {
    const token = normalizeMetadataToken(value);
    if (!token || pendingFavoriteTokens.current.has(token)) return;
    const key = favoriteKeyFromToken(token);
    const enabled = !favoriteMetadata.has(token);
    pendingFavoriteTokens.current.add(token);
    try {
      const result = await backend.favoriteSet(key, enabled);
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      const normalizedToken = result.data.favorite ? favoriteToken(result.data.favorite) : token;
      setFavoriteMetadata((current) => {
        const next = new Set(current);
        if (result.data.enabled) next.add(normalizedToken);
        else next.delete(normalizedToken);
        return next;
      });
      setFavoriteRecords((current) => {
        const withoutKey = current.filter((favorite) => favoriteToken(favorite) !== normalizedToken);
        return result.data.favorite ? [...withoutKey, result.data.favorite] : withoutKey;
      });
      showToast(`${value} 즐겨찾기를 ${result.data.enabled ? "추가" : "해제"}했습니다.`);
    } catch {
      showToast("즐겨찾기 변경을 저장하지 못했습니다.");
    } finally {
      pendingFavoriteTokens.current.delete(token);
    }
  }, [favoriteMetadata, showToast]);

  const queueGalleries = useCallback(
    async (ids: GalleryId[]) => {
      const uniqueIds = [...new Set(ids)].filter((id) => !duplicateHiddenGalleryIds.has(id));
      const newGalleryIds = uniqueIds.filter((id) => !galleries.get(id)?.download);
      const retryEntryIds = uniqueIds.flatMap((id) => {
        const download = galleries.get(id)?.download;
        return download && retryableDownloadStates.has(download.state) ? [download.entryId] : [];
      });
      if (!newGalleryIds.length && !retryEntryIds.length) {
        showToast("현재 상태에서 시작할 수 있는 항목이 없습니다.");
        dispatch({ type: "selection.clear" });
        return;
      }
      let started = 0;
      try {
        if (retryEntryIds.length) {
          const retryResult = await backend.downloadRetry(retryEntryIds);
          if (!retryResult.ok) {
            showToast(retryResult.error.message);
            return;
          }
          started += retryResult.data.length;
          setDownloadsRefresh((value) => value + 1);
        }
        if (newGalleryIds.length) {
          const requestId = `frontend-queue-${Date.now()}-${++queueRequestSequence.current}`;
          const queueResult = await backend.downloadQueueAdd(newGalleryIds, requestId);
          if (!queueResult.ok) {
            showToast(queueResult.error.message);
            return;
          }
          setGalleries((current) => mergeDownloadEntries(current, queueResult.data));
          setDownloadIds((current) => [...new Set([...current, ...queueResult.data.map((entry) => entry.galleryId)])]);
          started += queueResult.data.length;
        }
        showToast(`${started}개 항목의 다운로드를 시작했습니다.`);
      } catch {
        showToast("다운로드 대기열에 연결하지 못했습니다.");
      }
      dispatch({ type: "selection.clear" });
    },
    [duplicateHiddenGalleryIds, galleries, showToast],
  );

  const retryGallery = useCallback(
    async (id: GalleryId) => {
      const download = galleriesRef.current.get(id)?.download;
      if (duplicateHiddenGalleryIds.has(id)) {
        showToast("중복 검토에서 제외 처리된 항목입니다. 설정에서 복원한 뒤 다시 다운로드할 수 있습니다.");
        return;
      }
      if (!download || !retryableDownloadStates.has(download.state)) {
        showToast("현재 상태에서는 이 항목을 재시도할 수 없습니다.");
        return;
      }
      if (!beginDownloadMutation(download.entryId)) return;
      try {
        const result = await backend.downloadRetry([download.entryId]);
        if (!result.ok) {
          showToast(result.error.message);
          return;
        }
        setDownloadsRefresh((value) => value + 1);
        showToast("다운로드를 다시 시작했습니다.");
      } catch {
        showToast("재시도 요청을 backend에 전달하지 못했습니다.");
      } finally {
        finishDownloadMutation(download.entryId);
      }
    },
    [beginDownloadMutation, duplicateHiddenGalleryIds, finishDownloadMutation, showToast],
  );

  const cancelGallery = useCallback(async (id: GalleryId) => {
    const download = galleriesRef.current.get(id)?.download;
    if (!download) return;
    if (!beginDownloadMutation(download.entryId)) return;
    try {
      const result = await backend.downloadCancel([download.entryId]);
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      setGalleries((current) => mergeDownloadEntries(current, result.data));
      showToast("다운로드를 취소했습니다.");
    } catch {
      showToast("취소 요청을 backend에 전달하지 못했습니다.");
    } finally {
      finishDownloadMutation(download.entryId);
    }
  }, [beginDownloadMutation, finishDownloadMutation, showToast]);

  const quarantineGalleries = useCallback(async (ids: GalleryId[]) => {
    const downloads = ids
      .map((id) => galleriesRef.current.get(id)?.download)
      .filter((download): download is NonNullable<Gallery["download"]> => download !== undefined);
    const restoring = downloads.length > 0 && downloads.every((download) => download.state === "quarantined");
    const eligible = downloads.filter((download) =>
      restoring ? download.state === "quarantined" : download.state === "completed",
    );
    if (!eligible.length || eligible.length !== downloads.length) {
      showToast(restoring
        ? "선택한 모든 항목이 격리 상태일 때만 함께 복원할 수 있습니다."
        : "검증이 완료된 다운로드만 격리할 수 있습니다.");
      return;
    }
    const confirmed = window.confirm(restoring
      ? `${eligible.length}개 항목을 원래 위치로 복원할까요?`
      : `${eligible.length}개 항목을 복구 가능한 격리 폴더로 옮길까요? 자동으로 영구 삭제되지 않습니다.`);
    if (!confirmed) return;
    try {
      const result = restoring
        ? await backend.downloadQuarantineUndo(eligible.map((download) => download.entryId))
        : await backend.downloadQuarantine(
            eligible.map((download) => download.entryId),
            "사용자가 Downloads 화면에서 격리를 확인함",
          );
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      setGalleries((current) => mergeDownloadEntries(current, result.data));
      dispatch({ type: "selection.clear" });
      if (restoring) {
        const restoredEntryIds = new Set(eligible.map((download) => download.entryId));
        setLastUndoAction((current) => current?.kind === "download-quarantine"
          && current.entryIds.some((entryId) => restoredEntryIds.has(entryId))
          ? null
          : current);
        showToast("격리한 파일을 원래 위치로 복원했습니다.");
      } else {
        setLastUndoAction({
          kind: "download-quarantine",
          entryIds: eligible.map((download) => download.entryId),
        });
        showToast("파일을 복구 가능한 격리 폴더로 옮겼습니다. Ctrl+Z로 실행 취소할 수 있습니다.");
      }
    } catch {
      showToast(restoring ? "격리 파일 복원 요청에 실패했습니다." : "파일 격리 요청에 실패했습니다.");
    }
  }, [showToast]);

  const reconcileArtifacts = useCallback(async () => {
    if (reconcilingArtifacts) return;
    setReconcilingArtifacts(true);
    try {
      const result = await backend.appReconcile();
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      setDownloadsRefresh((value) => value + 1);
      const summary = result.data.issues.length
        ? `${result.data.inspectedArtifacts}개 검사 · ${result.data.issues.length}개 문제를 안전 상태로 표시했습니다.`
        : `${result.data.verifiedArtifacts}개 artifact의 DB·manifest·파일 무결성을 확인했습니다.`;
      showToast(result.data.resumedJobs
        ? `${summary} ${result.data.resumedJobs}개 작업을 재개했습니다.`
        : summary);
    } catch {
      showToast("artifact 무결성 검사를 실행하지 못했습니다.");
    } finally {
      setReconcilingArtifacts(false);
    }
  }, [reconcilingArtifacts, showToast]);

  const refreshAutoFind = useCallback(async () => {
    if (autoFindPending || autoFindSnapshot.run?.state === "running") return;
    setAutoFindPending(true);
    setAutoFindError(null);
    try {
      const result = await backend.autoFindRefresh();
      if (!result.ok) {
        setAutoFindError(result.error.message);
        showToast(result.error.message);
        return;
      }
      setAutoFindSnapshot((current) => ({ ...current, run: result.data }));
      await hydrateAutoFind();
    } catch {
      const message = "자동 탐색을 시작하지 못했습니다.";
      setAutoFindError(message);
      showToast(message);
    } finally {
      setAutoFindPending(false);
    }
  }, [autoFindPending, autoFindSnapshot.run?.state, hydrateAutoFind, showToast]);

  const cancelAutoFind = useCallback(async () => {
    if (autoFindPending || autoFindSnapshot.run?.state !== "running") return;
    setAutoFindPending(true);
    try {
      const result = await backend.autoFindCancel();
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      setAutoFindSnapshot((current) => ({ ...current, run: result.data }));
      await hydrateAutoFind();
      showToast("자동 탐색을 취소했습니다. 지금까지 찾은 후보는 보존됩니다.");
    } catch {
      showToast("자동 탐색 취소 요청을 전달하지 못했습니다.");
    } finally {
      setAutoFindPending(false);
    }
  }, [autoFindPending, autoFindSnapshot.run?.state, hydrateAutoFind, showToast]);

  const excludeAutoFindCandidates = useCallback(async (ids: GalleryId[]) => {
    const candidateIds = [...new Set(ids)].filter((id) => autoFindIds.includes(id));
    if (!candidateIds.length) return;
    try {
      const result = await backend.autoFindExclude(candidateIds, "사용자가 Auto Find 후보 목록에서 제외함");
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      applyAutoFindSnapshot(result.data.snapshot);
      dispatch({ type: "selection.clear" });
      setLastUndoAction({
        kind: "auto-find-exclusion",
        galleryIds: result.data.excludedGalleryIds,
      });
      showToast(`${result.data.excludedGalleryIds.length}개 후보를 다음 탐색에서도 제외합니다. Ctrl+Z로 실행 취소할 수 있습니다.`);
    } catch {
      showToast("자동 탐색 후보 제외 요청을 저장하지 못했습니다.");
    }
  }, [applyAutoFindSnapshot, autoFindIds, showToast]);

  const undoLastGalleryAction = useCallback(async () => {
    const action = lastUndoAction;
    if (!action) {
      showToast("실행 취소할 최근 제외 또는 격리 작업이 없습니다.");
      return;
    }
    if (undoPendingRef.current) return;
    undoPendingRef.current = true;
    try {
      if (action.kind === "auto-find-exclusion") {
        const result = await backend.explorationExclusionsRestore(action.galleryIds);
        if (!result.ok) {
          showToast(result.error.message);
          return;
        }
        applyAutoFindSnapshot(result.data.snapshot);
        showToast(`${result.data.restoredGalleryIds.length}개 Auto Find 후보 제외를 취소했습니다.`);
      } else {
        const result = await backend.downloadQuarantineUndo(action.entryIds);
        if (!result.ok) {
          showToast(result.error.message);
          return;
        }
        setGalleries((current) => mergeDownloadEntries(current, result.data));
        setDownloadsRefresh((current) => current + 1);
        showToast(`${result.data.length}개 격리 항목을 원래 위치로 복원했습니다.`);
      }
      setLastUndoAction((current) => current === action ? null : current);
    } catch {
      showToast(action.kind === "auto-find-exclusion"
        ? "Auto Find 후보 제외를 취소하지 못했습니다."
        : "격리 항목을 원래 위치로 복원하지 못했습니다.");
    } finally {
      undoPendingRef.current = false;
    }
  }, [applyAutoFindSnapshot, lastUndoAction, showToast]);

  const startDuplicateScan = useCallback(async () => {
    if (duplicatePendingRef.current || duplicateRun?.state === "running") return;
    duplicatePendingRef.current = true;
    setDuplicatePending(true);
    setDuplicateError(null);
    try {
      const result = await backend.duplicateScanStart();
      if (!result.ok) {
        setDuplicateError(result.error.message);
        showToast(result.error.message);
        return;
      }
      if (duplicateRunRef.current?.runId !== result.data.runId) duplicateHydrationToken.current += 1;
      duplicateRunRef.current = result.data;
      setDuplicateRun(result.data);
      if (duplicateSnapshotRef.current) {
        const next = { ...duplicateSnapshotRef.current, run: result.data };
        duplicateSnapshotRef.current = next;
        setDuplicateSnapshot(next);
      }
      await hydrateDuplicateSnapshot();
      showToast("검증된 로컬 아티팩트를 기준으로 작품 중복 검사를 시작했습니다.");
    } catch {
      const message = "작품 중복 검사를 시작하지 못했습니다.";
      setDuplicateError(message);
      showToast(message);
    } finally {
      duplicatePendingRef.current = false;
      setDuplicatePending(false);
    }
  }, [duplicateRun?.state, hydrateDuplicateSnapshot, showToast]);

  const cancelDuplicateScan = useCallback(async () => {
    if (duplicatePendingRef.current || duplicateRun?.state !== "running") return;
    duplicatePendingRef.current = true;
    setDuplicatePending(true);
    try {
      const result = await backend.duplicateScanCancel();
      if (!result.ok) {
        setDuplicateError(result.error.message);
        showToast(result.error.message);
        return;
      }
      duplicateRunRef.current = result.data;
      setDuplicateRun(result.data);
      if (duplicateSnapshotRef.current) {
        const next = { ...duplicateSnapshotRef.current, run: result.data };
        duplicateSnapshotRef.current = next;
        setDuplicateSnapshot(next);
      }
      await hydrateDuplicateSnapshot();
      showToast("작품 중복 검사를 취소했습니다. 저장된 후보와 판정 이력은 유지됩니다.");
    } catch {
      showToast("작품 중복 검사 취소 요청을 전달하지 못했습니다.");
    } finally {
      duplicatePendingRef.current = false;
      setDuplicatePending(false);
    }
  }, [duplicateRun?.state, hydrateDuplicateSnapshot, showToast]);

  const loadExplorePage = useCallback(async (page: number) => {
    const contextId = activeExploreContextIdRef.current;
    const context = contextId ? exploreContexts.current.get(contextId) : undefined;
    const currentQuery = queryRef.current;
    if (!context || !currentQuery.queryId || page < 1) return;
    const navigationToken = ++exploreNavigationToken.current;
    if (exploreRestoreFrame.current !== null) {
      window.cancelAnimationFrame(exploreRestoreFrame.current);
      exploreRestoreFrame.current = null;
    }
    if (currentQuery.page && galleryViewport.current) {
      context.session.recordScroll(currentQuery.page.page, galleryViewport.current.scrollTop);
      context.scrollTop = galleryViewport.current.scrollTop;
    }
    const loading: GalleryQueryState = {
      ...currentQuery,
      phase: "loading-page",
      pendingPage: page,
      error: null,
    };
    context.query = loading;
    queryRef.current = loading;
    dispatchQuery({ type: "restore", state: loading });
    const result = await context.session.open(page);
    if (
      result.status === "stale"
      || navigationToken !== exploreNavigationToken.current
      || activeExploreContextIdRef.current !== contextId
    ) return;
    if (result.status === "failed") {
      const failed: GalleryQueryState = { ...loading, phase: "error", pendingPage: null, error: result.error };
      context.query = failed;
      queryRef.current = failed;
      dispatchQuery({ type: "restore", state: failed });
      return;
    }
    const ready: GalleryQueryState = {
      ...loading,
      phase: "ready",
      page: result.page,
      pendingPage: null,
      error: null,
    };
    const resultIds = result.page.items.map((item) => item.id);
    context.query = ready;
    context.exploreIds = resultIds;
    context.scrollTop = result.scrollTop;
    queryRef.current = ready;
    exploreIdsRef.current = resultIds;
    dispatchQuery({ type: "restore", state: ready });
    setExploreIds(resultIds);
    setGalleries((current) => mergeGalleryPage(current, result.page).galleries);
    exploreRestoreFrame.current = window.requestAnimationFrame(() => {
      if (navigationToken === exploreNavigationToken.current && galleryViewport.current) {
        galleryViewport.current.scrollTop = result.scrollTop;
      }
      exploreRestoreFrame.current = null;
    });
  }, []);

  const selectedIds = useMemo(() => [...ui.selection.ids], [ui.selection.ids]);
  const multiSelectionMode = ui.selection.ids.size >= 2;
  const selectedCompletedEntryIds = useMemo(() => [...new Set(selectedIds.flatMap((id) => {
    const download = displayGalleries.get(id)?.download;
    return download?.state === "completed" ? [download.entryId] : [];
  }))], [displayGalleries, selectedIds]);
  const selectedCanInternalScan = selectedIds.length > 0
    && selectedCompletedEntryIds.length === selectedIds.length;

  const saveSettingsPatch = useCallback(
    async (patch: SettingsPatch) => {
      const result = await saveSettings(patch);
      showToast(result.ok ? "설정을 저장했습니다." : result.error.message);
      return result.ok;
    },
    [saveSettings, showToast],
  );

  const togglePrivacyMode = useCallback(async () => {
    if (privacyModePending) return;
    setPrivacyModePending(true);
    const result = await saveSettings({ privacyMode: !settings.privacyMode });
    setPrivacyModePending(false);
    if (result.ok) {
      showToast(result.data.privacyMode ? "프라이버시 모드를 켰습니다." : "프라이버시 모드를 껐습니다.");
    } else {
      showToast(result.error.message);
    }
  }, [privacyModePending, saveSettings, settings.privacyMode, showToast]);

  const requestTagSuggestions = useCallback((query: string, namespace?: TagNamespace) => {
    const sequence = ++tagSuggestionSequence.current;
    if (!query) { setTagSuggestions([]); return; }
    void backend.tagSuggestionsSearch({ query, namespace, limit: 8 }).then((result) => {
      if (sequence !== tagSuggestionSequence.current) return;
      setTagSuggestions(result.ok ? result.data : []);
    }).catch(() => { if (sequence === tagSuggestionSequence.current) setTagSuggestions([]); });
  }, []);

  const searchSuggestions = useMemo<SearchSuggestion[]>(() => {
    return ui.search.explore.draft.trim() ? tagSuggestions.map(catalogSuggestion) : buildSearchSuggestionCatalog(searchHistory);
  }, [searchHistory, tagSuggestions, ui.search.explore.draft]);

  const autoFindDiscoveryDates = useMemo(() => new Map(
    autoFindSnapshot.candidates.map((candidate) => [candidate.id, candidate.discoveredAt]),
  ), [autoFindSnapshot.candidates]);
  const groupedVisible = useMemo(() => {
    if (ui.view !== "auto-find" && ui.view !== "downloads") return [];
    const grouping = ui.grouping[ui.view] as GalleryGrouping;
    if (grouping === "all") return [];
    return groupGalleries(visible, grouping, (gallery) => ui.view === "auto-find"
      ? autoFindDiscoveryDates.get(gallery.id) ?? gallery.publishedAt
      : gallery.download?.updatedAt ?? gallery.download?.createdAt ?? gallery.publishedAt);
  }, [autoFindDiscoveryDates, ui.grouping, ui.view, visible]);
  const groupedStorageKeys = useMemo(() => groupedVisible.map((group) => galleryGroupStorageKey(
    ui.view === "auto-find" ? "auto-find" : "downloads",
    group,
  )), [groupedVisible, ui.view]);
  const keyboardNavigableIds = useMemo(() => {
    if (ui.view === "explore" || ui.grouping[ui.view] === "all") return actionableVisibleIds;
    const groupedView = ui.view;
    return groupedVisible.flatMap((group) => {
      const key = galleryGroupStorageKey(groupedView, group);
      return collapsedGroupKeys.has(key) ? [] : group.items.map((gallery) => gallery.id);
    });
  }, [actionableVisibleIds, collapsedGroupKeys, groupedVisible, ui.grouping, ui.view]);
  const effectiveKeyboardFocusId = useMemo(() => {
    if (keyboardFocusId !== null && keyboardNavigableIds.includes(keyboardFocusId)) return keyboardFocusId;
    return selectedIds.find((id) => keyboardNavigableIds.includes(id))
      ?? keyboardNavigableIds.at(0)
      ?? null;
  }, [keyboardFocusId, keyboardNavigableIds, selectedIds]);
  const focusGalleryCard = useCallback((id: GalleryId) => {
    setKeyboardFocusId(id);
    window.requestAnimationFrame(() => {
      const card = galleryViewport.current?.querySelector<HTMLElement>(`[data-gallery-id="${Number(id)}"]`);
      card?.focus({ preventScroll: true });
      card?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    });
  }, []);
  const refreshCurrentView = useCallback(() => {
    if (ui.view === "explore") {
      const activeId = activeExploreContextIdRef.current;
      const context = activeId ? exploreContexts.current.get(activeId) : undefined;
      if (!context?.request) {
        showToast("새로고침할 검색 결과가 없습니다. 먼저 검색해 주세요.");
        return;
      }
      startExploreSearch(context.request, { displayValue: context.displayValue, label: context.label });
      return;
    }
    if (ui.view === "auto-find") {
      void hydrateAutoFind(true);
      return;
    }
    setDownloadsRefresh((current) => current + 1);
  }, [hydrateAutoFind, showToast, startExploreSearch, ui.view]);
  const allVisibleGroupsCollapsed = groupedStorageKeys.length > 0
    && groupedStorageKeys.every((key) => collapsedGroupKeys.has(key));
  const toggleGroupCollapsed = useCallback((key: string) => {
    const next = new Set(collapsedGroupKeys);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    persistCollapsedGroupKeys(next);
  }, [collapsedGroupKeys, persistCollapsedGroupKeys]);
  const setAllVisibleGroupsCollapsed = useCallback((collapsed: boolean) => {
    const next = new Set(collapsedGroupKeys);
    for (const key of groupedStorageKeys) {
      if (collapsed) next.add(key);
      else next.delete(key);
    }
    persistCollapsedGroupKeys(next);
  }, [collapsedGroupKeys, groupedStorageKeys, persistCollapsedGroupKeys]);

  useEffect(() => {
    const keyDown = (event: KeyboardEvent) => {
      const target = event.target instanceof HTMLElement
        ? event.target
        : document.activeElement instanceof HTMLElement
          ? document.activeElement
          : document.body;
      const primaryModifier = event.ctrlKey || event.metaKey;
      const modalOpen = Boolean(document.querySelector("dialog[open]"));
      const textEditing = Boolean(target.closest('input, textarea, select, [contenteditable="true"]'));

      if (event.key === "Escape") {
        if (event.defaultPrevented || event.repeat || event.isComposing || target.closest("dialog")) return;
        if (ui.overlays.activityOpen) {
          event.preventDefault();
          closeActivity();
          return;
        }
        if (ui.overlays.settingsOpen || ui.overlays.reviewGalleryId !== null || ui.overlays.exitConfirmOpen) return;
        if (ui.search[ui.view].suggestionsOpen) {
          dispatch({ type: "search.suggestions", view: ui.view, open: false });
          return;
        }
        if (ui.detail.activeId !== null) dispatch({ type: "detail.close", id: ui.detail.activeId });
        else if (selectedIds.length) dispatch({ type: "selection.clear" });
        else openExitConfirm();
        event.preventDefault();
        return;
      }

      if (event.defaultPrevented || event.isComposing || modalOpen) return;

      if (primaryModifier && event.key === "Tab" && !event.altKey && !event.repeat) {
        event.preventDefault();
        const currentIndex = viewOrder.indexOf(ui.view);
        const offset = event.shiftKey ? -1 : 1;
        const nextView = viewOrder[(currentIndex + offset + viewOrder.length) % viewOrder.length]!;
        setKeyboardFocusId(null);
        navigateView(nextView);
        window.requestAnimationFrame(() => {
          document.querySelector<HTMLInputElement>('.view-header input[aria-label="검색"]')?.focus();
        });
        return;
      }

      if (primaryModifier && event.key.toLocaleLowerCase() === "f" && !event.altKey && !event.repeat) {
        event.preventDefault();
        const input = document.querySelector<HTMLInputElement>('.view-header input[aria-label="검색"]');
        input?.focus();
        input?.select();
        return;
      }

      if (event.key === "F5" && !primaryModifier && !event.altKey && !event.repeat) {
        event.preventDefault();
        refreshCurrentView();
        return;
      }

      if (textEditing) return;

      if ((event.key === "?" || event.key === "/" || event.code === "Slash") && !primaryModifier && !event.altKey && !event.repeat) {
        event.preventDefault();
        setKeyboardShortcutsOpen(true);
        return;
      }

      if (primaryModifier && event.key.toLocaleLowerCase() === "z" && !event.shiftKey && !event.altKey && !event.repeat) {
        event.preventDefault();
        void undoLastGalleryAction();
        return;
      }

      const galleryContext = Boolean(target.closest(".gallery-viewport, .selection-toolbar"));
      if (!galleryContext) return;

      if (primaryModifier && event.key.toLocaleLowerCase() === "a" && !event.altKey && !event.repeat) {
        event.preventDefault();
        if (event.shiftKey) dispatch({ type: "selection.clear" });
        else dispatch({ type: "selection.all", ids: actionableVisibleIds });
        return;
      }

      const card = target.closest<HTMLElement>(".gallery-card");
      const cardIsDirectTarget = card === target;
      const focusedId = card?.dataset.galleryId ? galleryId(Number(card.dataset.galleryId)) : effectiveKeyboardFocusId;

      if (primaryModifier && event.key === "Enter" && !event.shiftKey && !event.altKey && !event.repeat) {
        const actionIds = selectedIds.length ? selectedIds : focusedId === null ? [] : [focusedId];
        if (!actionIds.length) return;
        event.preventDefault();
        void queueGalleries(actionIds);
        return;
      }

      if (event.key === "Delete" && !primaryModifier && !event.altKey && !event.repeat && !target.closest("button")) {
        const actionIds = selectedIds.length ? selectedIds : focusedId === null ? [] : [focusedId];
        if (!actionIds.length) return;
        event.preventDefault();
        if (ui.view === "downloads") void quarantineGalleries(actionIds);
        else if (ui.view === "auto-find") void excludeAutoFindCandidates(actionIds);
        else showToast("후보 제외는 Auto Find 화면에서 사용할 수 있습니다.");
        return;
      }

      if (!cardIsDirectTarget || focusedId === null) return;
      const horizontal = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : 0;
      const vertical = event.key === "ArrowUp" ? -galleryColumns : event.key === "ArrowDown" ? galleryColumns : 0;
      const delta = horizontal || vertical;
      if (!delta) return;
      const currentIndex = keyboardNavigableIds.indexOf(focusedId);
      if (currentIndex < 0) return;
      const nextIndex = Math.max(0, Math.min(keyboardNavigableIds.length - 1, currentIndex + delta));
      const nextId = keyboardNavigableIds[nextIndex];
      if (nextId === undefined) return;
      event.preventDefault();
      if (event.shiftKey) {
        const anchorId = ui.selection.anchorId !== null && keyboardNavigableIds.includes(ui.selection.anchorId)
          ? ui.selection.anchorId
          : focusedId;
        dispatch({ type: "selection.range", anchorId, id: nextId, visibleIds: keyboardNavigableIds });
      }
      focusGalleryCard(nextId);
    };
    window.addEventListener("keydown", keyDown);
    return () => window.removeEventListener("keydown", keyDown);
  }, [actionableVisibleIds, closeActivity, effectiveKeyboardFocusId, excludeAutoFindCandidates, focusGalleryCard, galleryColumns, keyboardNavigableIds, navigateView, openExitConfirm, quarantineGalleries, queueGalleries, refreshCurrentView, selectedIds, showToast, ui.detail.activeId, ui.overlays, ui.search, ui.selection.anchorId, ui.view, undoLastGalleryAction]);

  const config = viewConfig[ui.view];
  const resultSourceLabel = backend.runtime === "tauri" ? "Hitomi 실데이터" : "브라우저 fixture";
  const currentAutoFindStatus = autoFindStatusLabel(autoFindLoading, autoFindError, autoFindSnapshot.run);
  const currentDuplicateStatus = duplicateStatusLabel(duplicateLoading, duplicateError, duplicateRun);
  const currentInternalStatus = internalStatusLabel(internalLoading, internalError, internalRun);
  const exploreContextTabs = useMemo<ExploreContextTab[]>(() => exploreContextIds.flatMap((id) => {
    const context = exploreContexts.current.get(id);
    if (!context) return [];
    const contextQuery = id === activeExploreContextId ? query : context.query;
    return [{
      id,
      label: context.label,
      ...(contextQuery.page ? {
        page: contextQuery.page.page,
        totalPages: contextQuery.page.totalPages,
      } : {}),
      root: context.root,
      busy: contextQuery.phase === "submitting" || contextQuery.phase === "loading-page",
    }];
  }), [activeExploreContextId, exploreContextIds, query]);
  const returnToPreviousExploreContext = useCallback(() => {
    const activeIndex = exploreContextIdsRef.current.indexOf(activeExploreContextIdRef.current ?? "");
    const previousId = activeIndex > 0 ? exploreContextIdsRef.current[activeIndex - 1] : undefined;
    if (previousId) activateExploreContext(previousId);
  }, [activateExploreContext]);
  const renderGalleryGrid = (items: Gallery[], ariaLabel: string) => (
    <GalleryGrid
      columns={galleryColumns}
      previewWidth={previewWidth}
      selectionContext={multiSelectionMode}
      ariaLabel={ariaLabel}
    >
      {items.map((gallery, index) => (
        <GalleryCard
          key={gallery.id}
          gallery={gallery}
          thumbnailPriority={index < galleryColumns ? "visible" : "prefetch"}
          view={ui.view}
          explorationExcluded={duplicateHiddenGalleryIds.has(gallery.id)}
          selected={ui.selection.ids.has(gallery.id)}
          selectionContext={multiSelectionMode}
          favoriteMetadata={favoriteMetadataForDisplay}
          duplicateCandidateCount={duplicateCandidateCounts.get(gallery.id) ?? 0}
          internalDuplicateResultCount={gallery.download
            ? internalDuplicateResultCounts.get(`${gallery.download.entryId}\u0000${gallery.id}`) ?? 0
            : 0}
          internalDuplicateProgress={internalArtifactProgress
            && ui.view === "downloads"
            && gallery.download?.entryId === internalArtifactProgress.entryId
            && gallery.id === internalArtifactProgress.galleryId
            ? internalArtifactProgress
            : undefined}
          keyboardFocusable={gallery.download?.state !== "quarantined"
            && !duplicateHiddenGalleryIds.has(gallery.id)
            && gallery.id === effectiveKeyboardFocusId}
          onKeyboardFocus={setKeyboardFocusId}
          onSelect={selectGallery}
          onOpenDetail={openDetail}
          onOpenArtifact={openArtifact}
          onOpenDownloadFolder={openDownloadFolder}
          onOpenReview={openReview}
          onOpenInternalReview={openInternalReview}
          onStatusDetail={openStatusDetail}
          onMetadataSearch={searchMetadata}
          onMetadataFavorite={toggleMetadataFavorite}
        />
      ))}
    </GalleryGrid>
  );

  return (
    <>
      <div className={`app-shell${ui.railCollapsed ? " sidebar-collapsed" : ""}`}>
        <SideRail
          view={ui.view}
          collapsed={ui.railCollapsed}
          autoFindCount={autoFindCount}
          attentionCount={attentionCount}
          sourceLabel={backend.runtime === "tauri" ? "Hitomi live" : "Browser fixture"}
          onNavigate={navigateView}
          onToggle={() => dispatch({ type: "rail.toggle" })}
        />
        <main className="workspace">
          <ViewHeader
            view={ui.view}
            search={ui.search[ui.view]}
            suggestions={ui.view === "explore" ? searchSuggestions : []}
            activityCount={activeDownloadCount}
            activityOpen={ui.overlays.activityOpen}
            onDraft={(value) => dispatch({ type: "search.draft", view: ui.view, value })}
            onSuggestions={(open, active) => dispatch({ type: "search.suggestions", view: ui.view, open, active })}
            onCommit={(value) => {
              if (ui.view === "explore") {
                const displayValue = (value ?? ui.search.explore.draft).trim();
                startExploreSearch({
                  text: displayValue,
                  includeTags: [],
                  excludeTags: [],
                  languages: [...ui.search.explore.languages],
                  sort: ui.exploreSort,
                  pageSize: 50,
                }, { displayValue, label: displayValue || "새 탐색" });
              }
              dispatch({ type: "search.commit", view: ui.view, value });
              if (ui.view !== "explore") showToast("현재 결과를 필터했습니다.");
            }}
            onSelectSuggestion={(suggestion, value) => {
              if (ui.view === "explore" && suggestion.request) {
                dispatch({ type: "search.languages", view: "explore", languages: suggestion.request.languages });
                dispatch({ type: "sort.set", sort: suggestion.request.sort });
                startExploreSearch(suggestion.request, { displayValue: value, label: value || "새 탐색" });
              } else if (ui.view === "explore") {
                startExploreSearch({
                  text: value.trim(),
                  includeTags: [],
                  excludeTags: [],
                  languages: [...ui.search.explore.languages],
                  sort: ui.exploreSort,
                  pageSize: 50,
                }, { displayValue: value, label: value || "새 탐색" });
              }
              dispatch({ type: "search.commit", view: ui.view, value });
            }}
            onCompleteSuggestion={(value) => {
              dispatch({ type: "search.draft", view: ui.view, value });
              dispatch({ type: "search.suggestions", view: ui.view, open: false });
            }}
            onLanguages={(languages) => {
              dispatch({ type: "search.languages", view: ui.view, languages });
            }}
            onTagSuggestionQuery={requestTagSuggestions}
            onTagCatalogRefresh={() => void refreshTagCatalog()}
            tagCatalogStatus={tagCatalogStatus}
            tagCatalogRefreshing={tagCatalogRefreshing}
            tagCatalogRevision={tagCatalogStatus?.revision}
            privacyMode={settings.privacyMode}
            privacyModePending={privacyModePending || settingsLoading}
            onPrivacyModeToggle={() => void togglePrivacyMode()}
            onActivity={() => ui.overlays.activityOpen ? closeActivity() : openActivity()}
            onSettings={() => dispatch({ type: "overlay.settings", open: true })}
          />
          <section className="page-heading">
            <div><span className="eyebrow">{config.eyebrow}</span><h1>{config.title}</h1></div>
            <div className="heading-actions">
              {ui.view === "auto-find" ? (
                <>
                  <button type="button" className="text-button" disabled={autoFindPending || autoFindSnapshot.run?.state === "running"} onClick={() => void refreshAutoFind()}><FluentIcon glyph="\uE72C" /> {autoFindSnapshot.run?.state === "failed" ? "다시 탐색" : "즐겨찾기 작가 갱신"}</button>
                  {autoFindSnapshot.run?.state === "running" ? <button type="button" className="text-button danger-button" disabled={autoFindPending} onClick={() => void cancelAutoFind()}><FluentIcon glyph="\uE711" /> 탐색 취소</button> : null}
                </>
              ) : ui.view === "downloads" ? (
                <>
                  <p className="sr-only" id="duplicate-scan-explanation">작품 간 검사는 작가가 같은 서로 다른 앨범끼리 비교하고, 내부 페이지 검사는 각 앨범 안에서 반복되거나 유사한 페이지를 찾습니다.</p>
                  <button type="button" className="text-button" disabled={reconcilingArtifacts} onClick={() => void reconcileArtifacts()}><FluentIcon glyph="\uE9D9" /> {reconcilingArtifacts ? "무결성 검사 중" : "무결성 검사"}</button>
                  <button type="button" className="text-button" aria-describedby="duplicate-scan-explanation" title="완료된 앨범 중 작가 정보가 하나라도 같은 작품끼리만 비교합니다." disabled={duplicateLoading || duplicatePending || duplicateRun?.state === "running"} onClick={() => void startDuplicateScan()}><FluentIcon glyph="\uE9D9" /> 같은 작가 작품 중복 검사</button>
                  {duplicateRun?.state === "running" ? <button type="button" className="text-button danger-button" disabled={duplicatePending} onClick={() => void cancelDuplicateScan()}><FluentIcon glyph="\uE711" /> 중복 검사 취소</button> : null}
                  <button
                    type="button"
                    className="text-button"
                    aria-describedby="duplicate-scan-explanation"
                    title={selectedIds.length === 0
                      ? "완료된 앨범을 하나 이상 선택하세요."
                      : !selectedCanInternalScan
                        ? "선택한 항목이 모두 다운로드 완료 상태여야 합니다."
                        : `선택한 완료 앨범 ${selectedCompletedEntryIds.length}개만 내부 검사합니다.`}
                    disabled={internalLoading || internalPending || internalRun?.state === "running" || !selectedCanInternalScan}
                    onClick={() => void startInternalScan(selectedCompletedEntryIds)}
                  ><FluentIcon glyph="\uE9D9" /> 선택 앨범 내부 페이지 검사{selectedCanInternalScan ? ` (${selectedCompletedEntryIds.length})` : ""}</button>
                  {internalRun?.state === "running" ? <button type="button" className="text-button danger-button" disabled={internalPending} onClick={() => void cancelInternalScan()}><FluentIcon glyph="\uE711" /> 내부 검사 취소</button> : null}
                  <button type="button" className="text-button primary" onClick={() => void queueGalleries(actionableVisibleIds)}><FluentIcon glyph="\uE896" /> 전체 다운로드</button>
                </>
              ) : null}
            </div>
          </section>
          {ui.view === "explore" && activeExploreContextId ? (
            <ExploreContextBar
              tabs={exploreContextTabs}
              activeId={activeExploreContextId}
              onActivate={activateExploreContext}
              onBack={returnToPreviousExploreContext}
              onClose={closeExploreContext}
            />
          ) : null}
          <section className="context-row">
            <div className="context-left">
              {(ui.view === "auto-find" || ui.view === "downloads") ? (
                <div className="gallery-grouping-toolbar" role="group" aria-label="목록 표시 도구">
                  <GroupingControl
                    value={ui.grouping[ui.view]}
                    onChange={(grouping) => persistGalleryGrouping(
                      ui.view === "auto-find" ? "auto-find" : "downloads",
                      grouping,
                    )}
                  />
                  <button
                    type="button"
                    className="text-button dark gallery-groups-toggle-all"
                    disabled={ui.grouping[ui.view] === "all" || !groupedVisible.length}
                    title={ui.grouping[ui.view] === "all" ? "기간별 또는 작가별에서 사용할 수 있습니다." : undefined}
                    onClick={() => setAllVisibleGroupsCollapsed(!allVisibleGroupsCollapsed)}
                  ><FluentIcon glyph="\uE70D" /> {allVisibleGroupsCollapsed ? "전부 펼치기" : "전부 접기"}</button>
                </div>
              ) : null}
              {ui.view === "explore" ? (
                <div className="select-control explore-sort-control"><label htmlFor="sort-select">정렬</label><select id="sort-select" value={ui.exploreSort} onChange={(event) => dispatch({ type: "sort.set", sort: event.target.value as SearchSort })}>{sortOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}</select></div>
              ) : ui.view === "auto-find" ? (
                <div className="auto-find-evidence">
                  <span className={`context-summary auto-find-status is-${autoFindSnapshot.run?.state ?? "idle"}`} role="status">{currentAutoFindStatus}</span>
                  {((autoFindSnapshot.run?.historyMode === "newer_than_oldest_downloaded" && autoFindSnapshot.cutoffEvidence.length)
                    || autoFindSnapshot.truncations.length) ? (
                    <details className="auto-find-evidence-details">
                      <summary>검증 근거 {autoFindSnapshot.cutoffEvidence.length + autoFindSnapshot.truncations.length}개</summary>
                      <div className="auto-find-evidence-popover">
                        {autoFindSnapshot.run?.historyMode === "newer_than_oldest_downloaded" && autoFindSnapshot.cutoffEvidence.length ? (
                          <ul aria-label="Auto Find 기록 cutoff 근거">
                            {autoFindSnapshot.cutoffEvidence.map((evidence) => (
                              <li key={evidence.artist}>
                                {evidence.artist}: {evidence.oldestOwnedGalleryId === undefined
                                  ? "검증 완료·격리 소유 작품 없음"
                                  : `가장 오래된 소유 gallery ID #${evidence.oldestOwnedGalleryId} 이후, ${evidence.qualifiedOwnedCount}개 확인`}
                              </li>
                            ))}
                          </ul>
                        ) : null}
                        {autoFindSnapshot.truncations.length ? (
                          <ul aria-label="Auto Find 결과 제한 경고">
                            {autoFindSnapshot.truncations.map((truncation) => (
                              <li key={`${truncation.artist}-${truncation.limit}`}>
                                {truncation.artist}: cutoff 이후 후보 {truncation.eligibleCount}개 중 {truncation.limit}개만 표시했습니다.
                              </li>
                            ))}
                          </ul>
                        ) : null}
                      </div>
                    </details>
                  ) : null}
                </div>
              ) : (
                <>
                  <div className="select-control download-status-filter-control">
                    <label className="sr-only" htmlFor="download-status-filter">다운로드 상태</label>
                    <select
                      id="download-status-filter"
                      aria-label="다운로드 상태 필터"
                      value={ui.downloadsFilter}
                      onChange={(event) => dispatch({
                        type: "downloads.filter",
                        filter: event.target.value as DownloadFilter,
                      })}
                    >
                      {(["all", "active", "review", "failed", "complete"] as const).map((filter) => (
                        <option key={filter} value={filter}>
                          {{ all: "전체 상태", active: "작업 중", review: "검토 필요", failed: "실패", complete: "완료" }[filter]}
                        </option>
                      ))}
                    </select>
                  </div>
                  <span className={`context-summary duplicate-scan-status is-${duplicateRun?.state ?? "idle"}`} role="status">{currentDuplicateStatus}</span>
                  <span className={`context-summary duplicate-scan-status is-${internalRun?.state ?? "idle"}`} role="status">{currentInternalStatus}</span>
                  {internalSnapshot.skips.length ? (
                    <details className="internal-scan-skips">
                      <summary>내부 검사 제외 항목 {internalSnapshot.skips.length}개</summary>
                      <p>500페이지 이상 앨범은 성능 상한 때문에 내부 페이지 검사에서만 제외됩니다. 다운로드와 전체 페이지 탐색에는 제한이 없습니다.</p>
                      <ul>{internalSnapshot.skips.map((skip) => <li key={skip.entryId}>#{skip.galleryId} · {skip.pageCount}p · 페이지 제한으로 제외</li>)}</ul>
                    </details>
                  ) : null}
                  {duplicateError ? <button type="button" className="text-button compact" onClick={() => void hydrateDuplicateSnapshot(true)}>결과 다시 불러오기</button> : null}
                  {internalError ? <button type="button" className="text-button compact" onClick={() => void hydrateInternalSnapshot(true)}>내부 결과 다시 불러오기</button> : null}
                </>
              )}
            </div>
            <div className="context-summary">{visible.length}개 결과 · {resultSourceLabel}</div>
          </section>
          <SelectionToolbar
            active={multiSelectionMode}
            count={ui.selection.ids.size}
            downloadsView={ui.view === "downloads"}
            restoreMode={selectedIds.length > 0 && selectedIds.every((id) => displayGalleries.get(id)?.download?.state === "quarantined")}
            onAll={() => dispatch({ type: "selection.all", ids: actionableVisibleIds })}
            onClear={() => dispatch({ type: "selection.clear" })}
            onPrimary={() => void queueGalleries(selectedIds)}
            onDelete={() => ui.view === "downloads"
              ? void quarantineGalleries(selectedIds)
              : ui.view === "auto-find"
                ? void excludeAutoFindCandidates(selectedIds)
                : showToast("후보 제외는 Auto Find 화면에서 사용할 수 있습니다.")}
          />
          <section id="gallery-viewport" ref={galleryViewport} className="gallery-viewport">
            {settingsLoading ? (
              <div className="loading-state" role="status"><span className="spinner" /> 저장된 화면 설정을 불러오는 중</div>
            ) : ((ui.view === "explore" && query.phase === "submitting" && !visible.length)
              || (ui.view === "downloads" && downloadsLoading && !visible.length)
              || (ui.view === "auto-find" && autoFindLoading && !visible.length)) ? (
              <GalleryGridSkeleton columns={galleryColumns} previewWidth={previewWidth} />
            ) : ui.view === "explore" && query.phase === "idle" ? (
              <div className="empty-state"><FluentIcon glyph="\uE721" /><h2>검색을 시작해 주세요</h2><p>검색어와 언어·정렬 필터를 정한 뒤 검색 버튼을 눌러 주세요.</p></div>
            ) : ui.view === "explore" && query.error && !query.page ? (
              <div className="empty-state" role="alert"><FluentIcon glyph="\uE7BA" /><h2>검색 결과를 불러오지 못했습니다</h2><p>{query.error.message}</p><button type="button" className="text-button" onClick={() => {
                const activeId = activeExploreContextIdRef.current;
                const context = activeId ? exploreContexts.current.get(activeId) : undefined;
                if (context?.request) startExploreSearch(context.request, { displayValue: context.displayValue, label: context.label });
              }}>다시 시도</button></div>
            ) : ui.view === "downloads" && downloadsError ? (
              <div className="empty-state" role="alert"><FluentIcon glyph="\uE7BA" /><h2>다운로드 목록을 불러오지 못했습니다</h2><p>{downloadsError}</p><button type="button" className="text-button" onClick={() => setDownloadsRefresh((value) => value + 1)}>다시 시도</button></div>
            ) : ui.view === "auto-find" && autoFindError && !autoFindSnapshot.candidates.length ? (
              <div className="empty-state" role="alert"><FluentIcon glyph="\uE7BA" /><h2>자동 탐색 결과를 불러오지 못했습니다</h2><p>{autoFindError}</p><button type="button" className="text-button" onClick={() => void hydrateAutoFind(true)}>다시 시도</button></div>
            ) : visible.length ? (
              (ui.view === "auto-find" || ui.view === "downloads") ? (
                ui.grouping[ui.view] === "all"
                  ? renderGalleryGrid(visible, `${config.title} 전체 목록`)
                  : <GalleryAccordionGroups
                      groups={groupedVisible}
                      view={ui.view}
                      previewWidth={previewWidth}
                      collapsedGroupKeys={collapsedGroupKeys}
                      onToggle={toggleGroupCollapsed}
                      renderGrid={renderGalleryGrid}
                    />
              ) : renderGalleryGrid(visible, config.title)
            ) : (
              <div className="empty-state"><FluentIcon glyph="\uE11A" /><h2>표시할 갤러리가 없습니다</h2><p>{ui.view === "auto-find" ? "즐겨찾기 작가를 추가한 뒤 명시적으로 갱신하거나 현재 검색·언어 필터를 바꿔 보세요." : "검색어나 언어·상태 필터를 바꿔 보세요."}</p></div>
            )}
            {ui.view === "explore" && query.page ? <div className="pager"><button type="button" className="text-button" disabled={query.phase === "loading-page" || query.page.page <= 1} onClick={() => void loadExplorePage(query.page!.page - 1)}>이전</button><span>{query.page.page} / {Math.max(1, query.page.totalPages)}{query.phase === "loading-page" ? " · 불러오는 중" : ""}</span><button type="button" className="text-button" disabled={query.phase === "loading-page" || query.page.page >= query.page.totalPages} onClick={() => void loadExplorePage(query.page!.page + 1)}>다음</button></div> : null}
          </section>
        </main>
        <ActivityDrawer
          open={ui.overlays.activityOpen}
          galleries={allGalleries}
          duplicateExcludedGalleryIds={duplicateHiddenGalleryIds}
          onClose={closeActivity}
          onReview={openReview}
          onRetry={(id) => void retryGallery(id)}
          onCancel={(id) => void cancelGallery(id)}
          pendingEntryIds={pendingDownloadEntries}
        />
      </div>

      <DetailWorkspace
        tabs={ui.detail.tabs}
        activeId={ui.detail.activeId}
        minimized={ui.detail.minimized}
        galleries={displayGalleries}
        favoriteMetadata={favoriteMetadataForDisplay}
        previewWidth={previewWidth}
        relatedPreviewWidth={settings.relatedPreviewWidth}
        backend={backend}
        onActivate={(id) => dispatch({ type: "detail.activate", id })}
        onClose={(id) => dispatch({ type: "detail.close", id })}
        onCloseAll={() => dispatch({ type: "detail.closeAll" })}
        onMinimize={() => dispatch({ type: "detail.minimize", minimized: true })}
        onRestore={() => dispatch({ type: "detail.minimize", minimized: false })}
        onOpenRelated={openRelatedDetail}
        onQueue={(id) => void queueGalleries([id])}
        onOpenDownloadFolder={(entryId) => void openDownloadFolder(entryId)}
        onMetadataSearch={searchMetadata}
        onMetadataFavorite={toggleMetadataFavorite}
      />

      <SettingsDialog
        open={ui.overlays.settingsOpen}
        settings={settings}
        loading={settingsLoading}
        error={settingsError}
        onClose={() => dispatch({ type: "overlay.settings", open: false })}
        onSave={saveSettingsPatch}
        onPreviewLayout={setSettingsPreview}
        onPreviewFolderName={previewFolderNameTemplate}
        onMaintenance={runMaintenance}
        onCheckForUpdates={() => appUpdater.checkForUpdates("manual")}
        onLoadExplorationExclusions={loadExplorationExclusionsAndSync}
        onRestoreExplorationExclusions={restoreExplorationExclusionsAndSync}
      />

      <UpdateDialog
        open={!tutorialOpen && appUpdater.state.info !== null && ["available", "downloading", "installing", "error"].includes(appUpdater.state.phase)}
        state={appUpdater.state}
        onLater={appUpdater.dismissUpdate}
        onInstall={() => void appUpdater.installUpdate()}
      />

      <TutorialDialog open={tutorialOpen} onClose={closeTutorial} />

      <KeyboardShortcutsDialog
        open={keyboardShortcutsOpen}
        onClose={() => setKeyboardShortcutsOpen(false)}
      />

      <DuplicateReviewDialog
        open={ui.overlays.reviewGalleryId !== null && duplicateReviewCandidateId !== null}
        review={duplicateReview ?? undefined}
        galleries={displayGalleries}
        loading={duplicateReviewLoading}
        error={duplicateReviewError}
        decisionPending={duplicateDecisionPending}
        browserFixture={backend.runtime === "browser-mock"}
        onClose={closeDuplicateReview}
        onRetry={() => duplicateReviewCandidateId && void hydrateDuplicateReview(duplicateReviewCandidateId)}
        onRescan={() => void startDuplicateScan()}
        onDecision={(request) => void applyDuplicateDecision(request)}
      />

      <DownloadOverlapReviewDialog
        open={ui.overlays.reviewGalleryId !== null && downloadOverlapReviewId !== null}
        review={downloadOverlapReview ?? undefined}
        loading={downloadOverlapLoading}
        error={downloadOverlapError}
        decisionPending={downloadOverlapDecisionPending}
        browserFixture={backend.runtime === "browser-mock"}
        previewWidth={previewWidth}
        thumbnailClient={thumbnailClient}
        onClose={closeDownloadOverlapReview}
        onRetry={() => downloadOverlapReviewId && void hydrateDownloadOverlapReview(downloadOverlapReviewId)}
        onDecision={(request) => void applyDownloadOverlapDecision(request)}
      />

      <InternalDuplicateDialog
        open={internalReviewEntryId !== null}
        review={internalReview ?? undefined}
        plan={internalPlan ?? undefined}
        loading={internalReviewLoading}
        busy={internalPending}
        error={internalReviewError}
        onClose={closeInternalReview}
        onRetry={() => internalReviewEntryId && void hydrateInternalReview(internalReviewEntryId)}
        onRescan={() => internalReviewEntryId && void startInternalScan([internalReviewEntryId])}
        onPlan={(request) => void previewInternalRemoval(request)}
        onApply={(plan) => void applyInternalRemoval(plan)}
        onUndo={(recordIds) => void undoInternalRemoval(recordIds)}
      />

      <ExitConfirmDialog
        open={ui.overlays.exitConfirmOpen}
        snapshot={exitWorkSnapshot}
        statusError={exitStatusError}
        actionPending={exitActionPending}
        forceQuitArmed={forceQuitArmed}
        onClose={closeExitConfirm}
        onMinimizeToTray={() => {
          if (exitActionPendingRef.current) return;
          exitActionPendingRef.current = true;
          setExitActionPending(true);
          void backend.appMinimizeToTray().then((result) => {
            if (!result.ok) {
              if (result.error.code === "APP_ACTIVE_WORK_STATUS_UNAVAILABLE") {
                setExitWorkSnapshot(null);
                setExitStatusError(true);
                setForceQuitArmed(false);
              }
              exitActionPendingRef.current = false;
              setExitActionPending(false);
              showToast(result.error.message);
            } else {
              exitActionPendingRef.current = false;
              setExitActionPending(false);
              exitSnapshotSequence.current += 1;
              exitConfirmOpenRef.current = false;
              dispatch({ type: "overlay.exit", open: false });
            }
          }).catch(() => {
            exitActionPendingRef.current = false;
            setExitActionPending(false);
            showToast("트레이로 최소화하지 못했습니다.");
          });
        }}
        onQuit={() => {
          if (exitActionPendingRef.current) return;
          if (exitWorkSnapshot === null && !exitStatusError) return;
          exitActionPendingRef.current = true;
          setExitActionPending(true);
          void (async () => {
            if (exitWorkSnapshot === null) {
              if (!forceQuitArmed) {
                const refreshed = await refreshExitWorkSnapshot(true);
                exitActionPendingRef.current = false;
                setExitActionPending(false);
                if (!refreshed) showToast("작업 상태를 다시 확인하지 못했습니다. 트레이로 보내거나 상태 확인 없이 종료할 수 있습니다.");
                return;
              }
            }

            const result = await backend.appQuit(exitWorkSnapshot
              ? {
                expectedWorkSetFingerprint: exitWorkSnapshot.workSetFingerprint,
                confirmActiveWork: hasActiveWork(exitWorkSnapshot),
              }
              : {
                expectedWorkSetFingerprint: "",
                confirmActiveWork: true,
                forceWhenStatusUnknown: true,
              });
            if (!result.ok) {
              if (result.error.code === "APP_ACTIVE_WORK_STATUS_UNAVAILABLE") {
                setExitWorkSnapshot(null);
                setExitStatusError(true);
                setForceQuitArmed(false);
              }
              exitActionPendingRef.current = false;
              setExitActionPending(false);
              showToast(result.error.message);
              return;
            }
            if (!result.data.accepted) {
              if (result.data.snapshot) {
                setExitWorkSnapshot(result.data.snapshot);
                setExitStatusError(false);
                setForceQuitArmed(false);
              }
              exitActionPendingRef.current = false;
              setExitActionPending(false);
              showToast(result.data.reason === "active_work_changed"
                ? "진행 작업이 변경되었습니다. 내용을 확인하고 다시 선택해 주세요."
                : "진행 중인 작업을 확인한 뒤 종료를 다시 선택해 주세요.");
            }
          })().catch(() => {
            exitActionPendingRef.current = false;
            setExitActionPending(false);
            showToast("프로그램을 종료하지 못했습니다.");
          });
        }}
      />

      {toast ? <div key={toast.id} className="toast" role="status">{toast.message}</div> : null}
    </>
  );
}

function GroupingControl({ value, onChange }: { value: GalleryGrouping; onChange: (value: GalleryGrouping) => void }) {
  return (
    <div className="segmented gallery-grouping-control" role="group" aria-label="표시 방식">
      <button type="button" aria-pressed={value === "all"} className={value === "all" ? "is-active" : ""} onClick={() => onChange("all")}>전체</button>
      <button type="button" aria-pressed={value === "day"} className={value === "day" ? "is-active" : ""} onClick={() => onChange("day")}>기간별</button>
      <button type="button" aria-pressed={value === "artist"} className={value === "artist" ? "is-active" : ""} onClick={() => onChange("artist")}>작가별</button>
    </div>
  );
}

type GalleryAccordionGroupsProps = {
  groups: readonly GalleryGroup[];
  view: "auto-find" | "downloads";
  previewWidth: number;
  collapsedGroupKeys: ReadonlySet<string>;
  onToggle: (key: string) => void;
  renderGrid: (items: Gallery[], ariaLabel: string) => ReactNode;
};

function GalleryAccordionGroups({
  groups,
  view,
  previewWidth,
  collapsedGroupKeys,
  onToggle,
  renderGrid,
}: GalleryAccordionGroupsProps) {
  const titleSize = Math.round(Math.max(14, Math.min(17, previewWidth / 18)));
  return (
    <div className="gallery-groups" data-group-view={view}>
      {groups.map((group) => {
        const storageKey = galleryGroupStorageKey(view, group);
        const collapsed = collapsedGroupKeys.has(storageKey);
        const label = view === "auto-find" && group.key.startsWith("artist\u001f")
          ? `즐겨찾기 작가 · ${group.label}`
          : group.label;
        return (
          <section className={`gallery-group${collapsed ? " is-collapsed" : ""}`} key={group.key}>
            <h2>
              <button
                type="button"
                className="gallery-group-toggle"
                aria-expanded={!collapsed}
                onClick={() => onToggle(storageKey)}
              >
                <span className="gallery-group-title" style={{ fontSize: `${titleSize}px` }}>{label}</span>
                <small className="gallery-group-count">{group.items.length}개 {view === "auto-find" ? "후보" : "작품"}</small>
                <span className="gallery-group-toggle-icon" aria-hidden="true">▾</span>
              </button>
            </h2>
            {!collapsed ? <div className="gallery-group-content">{renderGrid(group.items, `${label} 갤러리`)}</div> : null}
          </section>
        );
      })}
    </div>
  );
}
