import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type ReactNode } from "react";
import { markUi } from "../../diagnostics/uiDiagnostics";
import { readNavigationCheckpoint, writeNavigationCheckpoint } from "../../state/navigationCheckpoint";
import type { BackgroundOpenOptions } from "../../state/downloadStatus";
import type { NavigationOrigin } from "../../state/navigationCheckpoint";
import { readDetailPositions, saveDetailPosition } from "../../state/detailPositions";
import { persistNativeCheckpoint } from "../../api/workConsole";
import { useWorkQueue } from "../../hooks/useWorkQueue";
import { WorkQueuePanel, QueueSummary, queueProgress } from "../../components/WorkQueuePanel";
import { backend } from "../../api/backend";
import { PersonalLibraryProvider } from "../personalLibrary/PersonalLibraryProvider";
import { PersonalLibraryWorkspace, type SavedItemOpenOptions } from "../personalLibrary/PersonalLibraryWorkspace";
import { galleryFor, type Bookmark } from "../personalLibrary/api";
import { useAppShell } from "../../app/AppShell";
import { DownloadProgressContext, DownloadProgressStore } from "../../state/downloadProgress";
import type {
  AutoFindRun,
  AutoFindSnapshot,
  DownloadChangedEvent,
  DownloadEntry,
  DownloadLibraryPage,
  DownloadPage,
  DownloadOverlapAutomationHistoryItem,
  DownloadOverlapDecisionRequest,
  DownloadOverlapMergeRequest,
  DownloadOverlapReview,
  DuplicateReview,
  DuplicateScanRun,
  DuplicateSnapshot,
  ExplorationExclusionContext,
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
} from "../../api/contracts";
import {
  ActivityDrawer,
  type AutomaticOverlapActivity,
  type DanbooruSessionActivity,
  type SessionDownloadActivity,
} from "../../components/ActivityDrawer";
import { AutoFindPager } from "../../components/AutoFindPager";
import { DetailWorkspace } from "../../components/DetailWorkspace";
import { DownloadArtistFolderGrid } from "../../components/DownloadArtistFolderGrid";
import { DuplicateReviewDialog } from "../../components/DuplicateReviewDialog";
import { DownloadOverlapReviewDialog } from "../../components/DownloadOverlapReviewDialog";
import { ExploreContextBar, type ExploreContextTab } from "../../components/ExploreContextBar";
import { InternalDuplicateDialog } from "../../components/InternalDuplicateDialog";
import { FluentIcon } from "../../components/FluentIcon";
import { GalleryCard } from "../../components/GalleryCard";
import { ExcludedAlbumDialog } from "../../components/ExcludedAlbumDialog";
import { GalleryGrid } from "../../components/GalleryGrid";
import { GalleryGridSkeleton } from "../../components/GalleryGridSkeleton";
import { GalleryDisplayModeControl } from "../../components/GalleryDisplayModeControl";
import { GalleryScrollPositionHint } from "../../components/GalleryScrollPositionHint";
import { ProgressiveGallerySlot } from "../../components/ProgressiveGallerySlot";
import { KeyboardShortcutsDialog } from "../../components/KeyboardShortcutsDialog";
import { SelectionToolbar } from "../../components/SelectionToolbar";
import { ResultDetails } from "../../components/ResultDetails";
import { SettingsDialog } from "../../components/SettingsDialog";
import { TutorialDialog } from "../../components/TutorialDialog";
import { beginTutorialAction } from "../../tutorial/tourActions";
import { tutorialStepsBySource } from "../../tutorial/tourSteps";
import { workspaceRegistry } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { ViewHeader, type SearchSuggestion } from "../../components/ViewHeader";
import { galleryId, retryableDownloadStates, type DownloadFilter, type DownloadState, type Gallery, type GalleryDisplayMode, type GalleryId, type Language, type SearchSort, type ViewId } from "../../core/types";
import { useSavedGalleryPreviews } from "../../hooks/useSavedGalleryPreviews";
import { useProgressiveGalleryWindow } from "../../hooks/useProgressiveGalleryWindow";
import { resolveCompactGalleryColumns, resolveGalleryColumns } from "../../layout/galleryColumns";
import { alignPageSizeToColumns } from "../../layout/pageSizeAlignment";
import { buildSearchSuggestionCatalog, catalogSuggestion } from "../../search/searchSuggestions";
import { activeSearchToken, metadataSearchToken, searchTokenKind } from "../../search/searchTokens";
import { matchesGlobalSearchRules } from "../../search/globalSearchRules";
import { applyDownloadChanged } from "../../state/downloadProjection";
import { cancelDownloads, canCancelDownload, runningDownloadStates } from "../../state/downloadCancellation";
import { pickArtistBalancedCompletedDownload } from "../../state/downloadRandom";
import { paginateAutoFindItems, paginateGalleryItems } from "../../state/autoFindPagination";
import { autoFindFavoriteMatches, groupAutoFindGalleries, uniqueAutoFindGalleries } from "../../state/autoFindFavorites";
import { autoFindCutoffDescription, autoFindHistoryModeLabel } from "../../state/autoFindHistory";
import {
  duplicateEventNeedsSnapshot,
  duplicateRunIsNewer,
  mergeHydratedDuplicateSnapshot,
  validDuplicateRun,
} from "../../state/duplicateProjection";
import { mergeDownloadEntries, mergeDownloadLibraryPage, mergeGalleryDetail, mergeGalleryPage } from "../../state/galleryProjection";
import { galleryQueryReducer, initialGalleryQueryState, type GalleryQueryState } from "../../state/galleryQuery";
import { ExplorePageSession } from "../../state/explorePageSession";
import { isPendingAutoFindCandidate, visibleGalleries } from "../../state/selectors";
import {
  galleryGroupStorageKey,
  groupGalleries,
  type GalleryGroup,
  type GalleryGrouping,
} from "../../state/galleryGrouping";
import {
  buildStrictOverlapPlan,
  automaticUncensoredMerge,
  DOWNLOAD_OVERLAP_AUTO_REASON_CODE,
  DOWNLOAD_OVERLAP_AUTO_RULE_VERSION,
} from "../../state/downloadOverlapAuto";
import { buildDownloadOverlapContainmentGroup, prioritizeDownloadOverlapReviews, type DownloadOverlapContainmentGroup } from "../../state/downloadOverlapContainment";
import { collectUnacknowledgedAutomationHistory } from "../../state/downloadOverlapAutomationSequence";
import { initialUiState, uiReducer } from "../../state/uiState";
import { useDownloadPopularity } from "../../hooks/useDownloadPopularity";
import type { DownloadSort } from "../../api/downloadPopularity";
import {
  GalleryCoverSessionRetainer,
  galleryCoverPageSignature,
  prefetchNextGalleryPageAfterCurrent,
  useThumbnailClient,
} from "../../thumbnail";

const viewConfig: Record<ViewId, { eyebrow: string; title: string }> = {
  explore: { eyebrow: "EXPLORE", title: "갤러리 탐색" },
  "auto-find": { eyebrow: "AUTO FIND", title: "즐겨찾기 작가·그룹 자동 탐색" },
  downloads: { eyebrow: "DOWNLOADS", title: "다운로드 목록" },
};

const viewOrder: ViewId[] = ["explore", "auto-find", "downloads"];

const useStableGalleryCoverPage = (items: readonly Gallery[]): readonly Gallery[] => {
  const signature = galleryCoverPageSignature(items);
  const retained = useRef<{ signature: string; items: readonly Gallery[] } | null>(null);
  if (!retained.current || retained.current.signature !== signature) {
    retained.current = { signature, items };
  }
  return retained.current.items;
};

const sortOptions: Array<{ value: SearchSort; label: string }> = [
  { value: "recent", label: "최신순" },
  { value: "popular_today", label: "인기순 · 오늘" },
  { value: "popular_week", label: "인기순 · 이번 주" },
  { value: "popular_month", label: "인기순 · 이번 달" },
  { value: "popular_year", label: "인기순 · 올해" },
  { value: "random", label: "무작위" },
];

const previewFolderNameTemplate = (template: string) => backend.folderNameTemplatePreview(template);
const loadStorageUsage = () => backend.storageUsageGet();
const loadExplorationExclusions = () => backend.explorationExclusionsList();
const restoreExplorationExclusions = (galleryIds: GalleryId[]) =>
  backend.explorationExclusionsRestore(galleryIds);

const activityNotificationStates: ReadonlySet<DownloadState> = new Set([
  "completed",
  "failed",
  "interrupted",
  "review_required",
  "cancelled",
  "quarantined",
]);
const DOWNLOAD_OVERLAP_AUTOMATION_HISTORY_PAGE_SIZE = 50;

type AutomationReviewSession = {
  items: DownloadOverlapAutomationHistoryItem[];
  index: number;
};

type UndoAction = { at: number } & (
  | { kind: "auto-find-exclusion" | "explore-exclusion"; galleryIds: GalleryId[] }
  | { kind: "download-quarantine"; entryIds: string[] });

type ClosedNavigation = { at: number } & (
  | { kind: "explore"; context: ExploreContext; index: number }
  | { kind: "detail"; ids: GalleryId[]; activeId: GalleryId | null; before: GalleryId[] });

type ExploreContext = {
  origin?: NavigationOrigin;
  resumePage?: { queryId: string | null; page: number };
  id: string;
  label: string;
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

const maximumExploreContexts = 64;

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

const autoFindStatusLabel = (
  loading: boolean,
  error: string | null,
  pendingCandidateCount: number,
  run?: AutoFindRun,
): string => {
  if (loading) return "저장된 자동 탐색 결과를 불러오는 중";
  if (error) return `자동 탐색 오류 · ${error}`;
  if (!run) return "아직 실행한 자동 탐색이 없습니다.";
  if (run.state === "running") {
    return `탐색 중 · 작가·그룹 ${run.completedFavorites}/${run.totalFavorites} · 확인된 항목 ${run.candidatesFound}개 · 다운로드 전 ${pendingCandidateCount}개`;
  }
  if (run.state === "failed") return `탐색 실패 · ${run.errorMessage ?? run.errorCode ?? "원인을 확인해 주세요."}`;
  if (run.state === "cancelled") return `탐색 취소됨 · 확인된 항목 ${run.candidatesFound}개 · 다운로드 전 ${pendingCandidateCount}개 보존`;
  return `탐색 완료 · 작가·그룹 ${run.completedFavorites}/${run.totalFavorites} · 확인된 항목 ${run.candidatesFound}개 · 다운로드 전 ${pendingCandidateCount}개`;
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

export type GalleryWorkspaceBridge = {
  workspace: ReactNode;
  favoriteMetadata: ReadonlySet<string>;
  activityCount: number;
  onActivity: () => void;
  onActivityRecord: (activity: DanbooruSessionActivity) => void;
  onMetadataFavorite: (token: string) => void;
  onOpenPersonalLibrary: () => void;
};

type HitomiFeatureProps = {
  active: boolean;
  navigationRequest?: import("../../app/CommonNavigation").NavigationRequest | null;
  children: (bridge: GalleryWorkspaceBridge) => ReactNode;
};

/**
 * Persistent gallery controller. Only its workspace DOM is switched off.
 * The callback is the explicit legacy gallery activity/favorites integration seam;
 * this module does not import or create another platform's workspace.
 */
export function HitomiFeature(props: HitomiFeatureProps) {
  const shell = useAppShell();
  return <PersonalLibraryProvider notify={shell.showToast}><HitomiFeatureContent {...props} /></PersonalLibraryProvider>;
}

function HitomiFeatureContent({ active, children, navigationRequest }: HitomiFeatureProps) {
  const shell = useAppShell();
  const [personalLibraryOpen, setPersonalLibraryOpen] = useState(false);
  const [personalLibraryVisited, setPersonalLibraryVisited] = useState(false);
  const [savedPageRequest, setSavedPageRequest] = useState<{ galleryId: GalleryId; page: number; sequence: number } | null>(null);
  const {
    showToast, privacyModePending, togglePrivacyMode,
    openExitConfirm, setActivityOpen, setSettingsOpen, toggleRail, selectSource,
  } = shell;
  const shellRef = useRef(shell);
  shellRef.current = shell;
  const thumbnailClient = useThumbnailClient();
  const [downloadProgress] = useState(() => new DownloadProgressStore());
  useEffect(() => () => downloadProgress.clear(), [downloadProgress]);
  const sessionCoverRetainer = useRef<GalleryCoverSessionRetainer | null>(null);
  if (!sessionCoverRetainer.current) {
    sessionCoverRetainer.current = new GalleryCoverSessionRetainer(thumbnailClient);
  }
  const [navigationCheckpoint] = useState(() => backend.runtime === "tauri" ? readNavigationCheckpoint() : null);
  const checkpointRestored = useRef(false);
  const [ui, dispatch] = useReducer(uiReducer, initialUiState, (initial) => navigationCheckpoint
    ? { ...initial, view:navigationCheckpoint.view, downloadsFilter:navigationCheckpoint.downloadsFilter } : initial);
  const [query, dispatchQuery] = useReducer(galleryQueryReducer, initialGalleryQueryState);
  const [galleries, setGalleries] = useState<ReadonlyMap<GalleryId, Gallery>>(() => new Map());
  const [exploreIds, setExploreIds] = useState<GalleryId[]>([]);
  const [downloadIds, setDownloadIds] = useState<GalleryId[]>([]);
  const popularity = useDownloadPopularity(shell.backgroundReady, downloadIds);
  const [duplicateHiddenGalleryIds, setDuplicateHiddenGalleryIds] = useState<ReadonlySet<GalleryId>>(() => new Set());
  const [explorationExcludedGalleryIds, setExplorationExcludedGalleryIds] = useState<ReadonlySet<GalleryId>>(() => new Set());
  const [explorationExclusionsReady, setExplorationExclusionsReady] = useState(false);
  const [exclusionContext, setExclusionContext] = useState<ExplorationExclusionContext | null>(null);
  const restoringExclusionIds = useRef(new Set<GalleryId>());
  const [restoringExclusions, setRestoringExclusions] = useState<ReadonlySet<GalleryId>>(() => new Set());
  const [downloadsLoading, setDownloadsLoading] = useState(true);
  const [downloadsError, setDownloadsError] = useState<string | null>(null);
  const [exploreContextIds, setExploreContextIds] = useState<string[]>([]);
  const [activeExploreContextId, setActiveExploreContextId] = useState<string | null>(null);
  const [downloadsRefresh, setDownloadsRefresh] = useState(0);
  const workQueue = useWorkQueue(shell.backgroundReady);
  const [downloadsPage, setDownloadsPage] = useState(1);
  const downloadsPageContext = useRef<string | null>(null);
  const [favoriteMetadata, setFavoriteMetadata] = useState<ReadonlySet<string>>(() => new Set());
  const [favoriteRecords, setFavoriteRecords] = useState<FavoriteRecord[]>([]);
  const [searchHistory, setSearchHistory] = useState<SearchHistoryEntry[]>([]);
  const searchHistoryRequest = useRef(0);
  const historyMutationBusy = useRef(false);
  const [historyPending, setHistoryPending] = useState(false);
  const [tagCatalogStatus, setTagCatalogStatus] = useState<TagCatalogStatus | undefined>(undefined);
  const [tagCatalogRefreshing, setTagCatalogRefreshing] = useState(false);
  const [randomOpenPending, setRandomOpenPending] = useState(false);
  const [tagSuggestions, setTagSuggestions] = useState<TagSuggestion[]>([]);
  const tagSuggestionSequence = useRef(0);
  const [autoFindSnapshot, setAutoFindSnapshot] = useState<AutoFindSnapshot>({ candidates: [], cutoffEvidence: [], truncations: [] });
  const [autoFindIds, setAutoFindIds] = useState<GalleryId[]>([]);
  const [autoFindPage, setAutoFindPage] = useState(1);
  const autoFindPageContext = useRef<string | null>(null);
  const [autoFindLoading, setAutoFindLoading] = useState(true);
  const [autoFindError, setAutoFindError] = useState<string | null>(null);
  const [autoFindPending, setAutoFindPending] = useState(false);
  const [duplicateSnapshot, setDuplicateSnapshot] = useState<DuplicateSnapshot | null>(null);
  const [duplicateRun, setDuplicateRun] = useState<DuplicateScanRun | undefined>(undefined);
  const [duplicateLoading, setDuplicateLoading] = useState(true);
  const [duplicateError, setDuplicateError] = useState<string | null>(null);
  const [duplicatePending, setDuplicatePending] = useState(false);
  const [pairCompareOpen, setPairCompareOpen] = useState(false);
  const [pairCompareIds, setPairCompareIds] = useState<[string, string]>(["", ""]);
  const [pairCompareRun, setPairCompareRun] = useState<{ runId: string; ids: GalleryId[] } | null>(null);
  const [duplicateReviewCandidateId, setDuplicateReviewCandidateId] = useState<string | null>(null);
  const [duplicateReview, setDuplicateReview] = useState<DuplicateReview | null>(null);
  const [duplicateReviewLoading, setDuplicateReviewLoading] = useState(false);
  const [duplicateReviewError, setDuplicateReviewError] = useState<string | null>(null);
  const [duplicateDecisionPending, setDuplicateDecisionPending] = useState(false);
  const [downloadOverlapReviewId, setDownloadOverlapReviewId] = useState<string | null>(null);
  const [downloadOverlapReview, setDownloadOverlapReview] = useState<DownloadOverlapReview | null>(null);
  const [overlapInventory, setOverlapInventory] = useState<DownloadOverlapReview[]>([]);
  const [overlapInventoryLoading, setOverlapInventoryLoading] = useState(false);
  const [overlapInventoryRefresh, setOverlapInventoryRefresh] = useState(0);
  const [containmentBatchProgress, setContainmentBatchProgress] = useState<{ completed: number; total: number }>();
  const [containmentKeeperId, setContainmentKeeperId] = useState<GalleryId | null>(null);
  const [containmentBatchGroup, setContainmentBatchGroup] = useState<DownloadOverlapContainmentGroup>();
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
  const [keyboardFocusId, setKeyboardFocusId] = useState<GalleryId | null>(null);
  const [keyboardShortcutsOpen, setKeyboardShortcutsOpen] = useState(false);
  const [lastUndoAction, setLastUndoAction] = useState<UndoAction | null>(null);
  const closedNavigation = useRef<ClosedNavigation[]>([]);
  const detailOrigins = useRef(new Map<GalleryId, NavigationOrigin>());
  const restoredContextIds = useRef(new Map<string, string>());
  const [reconcilingArtifacts, setReconcilingArtifacts] = useState(false);
  const [settingsPreview, setSettingsPreview] = useState<{ maxColumns: number; previewWidth: number } | null>(null);
  const [pendingDownloadEntries, setPendingDownloadEntries] = useState<ReadonlySet<string>>(() => new Set());
  const [bulkRetryPending, setBulkRetryPending] = useState(false);
  const bulkRetryBusy = useRef(false);
  const [cancellingDownloadEntries, setCancellingDownloadEntries] = useState<ReadonlySet<string>>(() => new Set());
  const [sessionDownloadActivities, setSessionDownloadActivities] = useState<SessionDownloadActivity[]>([]);
  const [automaticOverlapActivities, setAutomaticOverlapActivities] = useState<AutomaticOverlapActivity[]>([]);
  const [downloadOverlapAutomationHistory, setDownloadOverlapAutomationHistory] = useState<DownloadOverlapAutomationHistoryItem[]>([]);
  const [downloadOverlapAutomationHistoryPage, setDownloadOverlapAutomationHistoryPage] = useState(0);
  const [downloadOverlapAutomationHistoryTotalItems, setDownloadOverlapAutomationHistoryTotalItems] = useState(0);
  const [downloadOverlapAutomationHistoryUnacknowledgedItems, setDownloadOverlapAutomationHistoryUnacknowledgedItems] = useState(0);
  const [downloadOverlapAutomationHistoryLoading, setDownloadOverlapAutomationHistoryLoading] = useState(false);
  const [downloadOverlapAutomationHistoryError, setDownloadOverlapAutomationHistoryError] = useState<string | null>(null);
  const [downloadOverlapAutomationHistoryPendingReviewIds, setDownloadOverlapAutomationHistoryPendingReviewIds] = useState<ReadonlySet<string>>(() => new Set());
  const [automationReviewSession, setAutomationReviewSession] = useState<AutomationReviewSession | null>(null);
  const [automationSequenceLoading, setAutomationSequenceLoading] = useState(false);
  const [unreadAutomaticOverlapReviewIds, setUnreadAutomaticOverlapReviewIds] = useState<ReadonlySet<string>>(() => new Set());
  const [automaticOverlapSweepRevision, setAutomaticOverlapSweepRevision] = useState(0);
  const [danbooruSessionActivities, setDanbooruSessionActivities] = useState<DanbooruSessionActivity[]>([]);
  const [unreadActivityCount, setUnreadActivityCount] = useState(0);
  const searchToken = useRef(0);
  const autoFindHydrationToken = useRef(0);
  const autoFindHydrationTask = useRef<Promise<void> | null>(null);
  const autoFindHydrationAgain = useRef(false);
  const duplicateHydrationToken = useRef(0);
  const duplicateReviewToken = useRef(0);
  const duplicateRunRef = useRef<DuplicateScanRun | undefined>(undefined);
  const duplicateSnapshotRef = useRef<DuplicateSnapshot | null>(null);
  const duplicatePendingRef = useRef(false);
  const duplicateDecisionPendingRef = useRef(false);
  const downloadOverlapReviewToken = useRef(0);
  const exploreExclusionPendingRef = useRef(false);
  const downloadOverlapAutomationHistoryBodyToken = useRef(0);
  const downloadOverlapAutomationHistoryCountToken = useRef(0);
  const downloadOverlapAutomationHistoryPendingRef = useRef(new Set<string>());
  const automationReviewSessionRef = useRef<AutomationReviewSession | null>(null);
  const automationSequenceToken = useRef(0);
  const automationSequenceLoadingRef = useRef(false);
  const downloadOverlapDecisionPendingRef = useRef(false);
  const automaticOverlapInFlightRef = useRef(false);
  const automaticOverlapRescanRequestedRef = useRef(false);
  const automaticOverlapAttemptedRef = useRef(new Set<string>());
  const internalHydrationToken = useRef(0);
  const internalReviewToken = useRef(0);
  const internalRunRef = useRef<InternalScanRun | undefined>(undefined);
  const internalArtifactProgressRef = useRef<InternalArtifactScanProgress | null>(null);
  const internalPendingRef = useRef(false);
  const downloadHydrationToken = useRef(0);
  const explorationExclusionsHydrationToken = useRef(0);
  const queueRequestSequence = useRef(0);
  const pendingDownloadEntriesRef = useRef(new Set<string>());
  const sessionDownloadStatesRef = useRef(new Map<GalleryId, DownloadState | undefined>());
  const automaticOverlapActivityIdsRef = useRef(new Set<string>());
  const undoPendingRef = useRef(false);
  const pendingFavoriteTokens = useRef(new Set<string>());
  const pendingFavoriteIntents = useRef(new Map<string, boolean>());
  const openingDownloadFolders = useRef(new Set<string>());
  const hydratedDetails = useRef(new Set<GalleryId>());
  const hydratingDetails = useRef(new Set<GalleryId>());
  const hydratedCardDetails = useRef(new Set<GalleryId>());
  const hydratingCardDetails = useRef(new Set<GalleryId>());
  const nearbyDetailQueue = useRef<GalleryId[]>([]);
  const nearbyDetailQueued = useRef(new Set<GalleryId>());
  const artistDetailQueue = useRef<GalleryId[]>([]);
  const artistDetailWanted = useRef(new Set<GalleryId>());
  const artistDetailAttempted = useRef(new Set<GalleryId>());
  const nearbyDetailWorkers = useRef(0);
  const nearbyDetailPump = useRef<() => void>(() => undefined);
  const galleriesRef = useRef(galleries);
  const gallerySummaryFromCurrent = useCallback((gallery: Gallery | undefined, id: GalleryId) => {
    if (!gallery) return { id };
    const publishedRank = Number(gallery.publishedAt.replaceAll("-", ""));
    return {
      id: gallery.id,
      title: gallery.title,
      artist: gallery.artist,
      ...(gallery.group ? { group: gallery.group } : {}),
      pages: gallery.pages,
      language: gallery.language,
      ...(Number.isFinite(publishedRank) ? { publishedRank } : {}),
    };
  }, []);
  const downloadLibraryPageFromEntries = useCallback((page: DownloadPage): DownloadLibraryPage => ({
    page: page.page,
    totalItems: page.totalItems,
    items: page.entries.map((download) => ({
      gallery: gallerySummaryFromCurrent(galleriesRef.current.get(download.galleryId), download.galleryId),
      download,
    })),
  }), [gallerySummaryFromCurrent]);
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
  const exploreLifetime = useRef(0);
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
  const { settings, loading: settingsLoading, error: settingsError } = shell.settingsStore;
  const currentGalleryDisplayMode = ui.displayMode[ui.view];
  const [collapsedGroupKeys, setCollapsedGroupKeys] = useState<ReadonlySet<string>>(() => new Set());
  const [expandedDownloadGroupKeys, setExpandedDownloadGroupKeys] = useState<ReadonlySet<string>>(() => new Set());
  const { enqueue: persistListPreferences, isPending: preferencesPending } = shell.preferenceQueue;

  const maximumColumns = settingsPreview?.maxColumns ?? settings.maxColumns;
  const previewWidth = settingsPreview?.previewWidth ?? settings.previewWidth;
  const [galleryColumns, setGalleryColumns] = useState(1);
  const [artistFolderColumns, setArtistFolderColumns] = useState(1);
  const hitomiPageSize = alignPageSizeToColumns(settings.explorePageSize, galleryColumns, 200);


  useEffect(() => () => {
    checkpointRestored.current = false;
    autoFindHydrationTask.current = null;
    autoFindHydrationAgain.current = false;
    ++autoFindHydrationToken.current;
    ++exploreLifetime.current;
    ++automationSequenceToken.current;
    ++downloadOverlapReviewToken.current;
    sessionCoverRetainer.current?.clear();
    for (const context of exploreContexts.current.values()) context.session.clear();
    if (exploreContexts.current.size === 0) explorePageSession.current?.clear();
    exploreContexts.current.clear();
    if (exploreRestoreFrame.current !== null) window.cancelAnimationFrame(exploreRestoreFrame.current);
  }, []);

  const cancelAutomationReviewSequence = useCallback(() => {
    ++automationSequenceToken.current;
    automationSequenceLoadingRef.current = false;
    setAutomationSequenceLoading(false);
    automationReviewSessionRef.current = null;
    setAutomationReviewSession(null);
  }, []);

  const applyDownloadOverlapAutomationHistoryCount = useCallback((
    totalItems: number,
    unacknowledgedItems: number,
  ) => {
    setDownloadOverlapAutomationHistoryTotalItems(totalItems);
    setDownloadOverlapAutomationHistoryUnacknowledgedItems(unacknowledgedItems);
  }, []);

  const hydrateDownloadOverlapAutomationHistoryMeta = useCallback(async () => {
    const token = ++downloadOverlapAutomationHistoryCountToken.current;
    try {
      const result = await backend.downloadOverlapAutomationHistoryList({ page: 1, pageSize: 1 });
      if (token !== downloadOverlapAutomationHistoryCountToken.current || !result.ok) return;
      applyDownloadOverlapAutomationHistoryCount(result.data.totalItems, result.data.unacknowledgedItems);
      const persistedReviewIds = new Set(result.data.items.map((item) => item.reviewId));
      setUnreadAutomaticOverlapReviewIds((current) =>
        new Set([...current].filter((reviewId) => !persistedReviewIds.has(reviewId))));
    } catch {
      // The lightweight count is best-effort. Opening Activity performs a visible retry.
    }
  }, [applyDownloadOverlapAutomationHistoryCount]);

  const hydrateDownloadOverlapAutomationHistoryPage = useCallback(async (
    page: number,
    replace: boolean,
  ) => {
    const bodyToken = ++downloadOverlapAutomationHistoryBodyToken.current;
    const countToken = ++downloadOverlapAutomationHistoryCountToken.current;
    setDownloadOverlapAutomationHistoryLoading(true);
    setDownloadOverlapAutomationHistoryError(null);
    try {
      const result = await backend.downloadOverlapAutomationHistoryList({
        page,
        pageSize: DOWNLOAD_OVERLAP_AUTOMATION_HISTORY_PAGE_SIZE,
      });
      if (bodyToken !== downloadOverlapAutomationHistoryBodyToken.current) return;
      if (!result.ok) {
        setDownloadOverlapAutomationHistoryError(result.error.message);
        return;
      }
      setDownloadOverlapAutomationHistory((current) => {
        const merged = new Map((replace ? [] : current).map((item) => [item.reviewId, item]));
        result.data.items.forEach((item) => merged.set(item.reviewId, item));
        return [...merged.values()].sort((left, right) => right.occurredAt.localeCompare(left.occurredAt));
      });
      setDownloadOverlapAutomationHistoryPage(result.data.page);
      if (countToken === downloadOverlapAutomationHistoryCountToken.current) {
        applyDownloadOverlapAutomationHistoryCount(result.data.totalItems, result.data.unacknowledgedItems);
      }
      const persistedReviewIds = new Set(result.data.items.map((item) => item.reviewId));
      setUnreadAutomaticOverlapReviewIds((current) =>
        new Set([...current].filter((reviewId) => !persistedReviewIds.has(reviewId))));
    } catch {
      if (bodyToken === downloadOverlapAutomationHistoryBodyToken.current) {
        setDownloadOverlapAutomationHistoryError("자동 판본 분류 기록 backend에 연결하지 못했습니다.");
      }
    } finally {
      if (bodyToken === downloadOverlapAutomationHistoryBodyToken.current) {
        setDownloadOverlapAutomationHistoryLoading(false);
      }
    }
  }, [applyDownloadOverlapAutomationHistoryCount]);

  useEffect(() => {
    void hydrateDownloadOverlapAutomationHistoryMeta();
  }, [hydrateDownloadOverlapAutomationHistoryMeta]);

  const refreshDownloadOverlapAutomationHistory = useCallback(() => {
    if (shellRef.current.activityOpen) {
      void hydrateDownloadOverlapAutomationHistoryPage(1, true);
    } else {
      void hydrateDownloadOverlapAutomationHistoryMeta();
    }
  }, [hydrateDownloadOverlapAutomationHistoryMeta, hydrateDownloadOverlapAutomationHistoryPage]);

  const recordSessionDownloadActivity = useCallback((id: GalleryId, state?: DownloadState) => {
    const known = sessionDownloadStatesRef.current.has(id);
    const previousState = sessionDownloadStatesRef.current.get(id);
    sessionDownloadStatesRef.current.set(id, state);
    const attentionTransition = known
      && state !== undefined
      && state !== previousState
      && activityNotificationStates.has(state);
    if (known && !attentionTransition) return;

    const occurredAt = Date.now();
    setSessionDownloadActivities((current) => known
      ? current.map((activity) => activity.galleryId === id
        ? { galleryId: id, occurredAt, state }
        : activity)
      : [{ galleryId: id, occurredAt, state }, ...current]);
    if (!shellRef.current.activityOpen) setUnreadActivityCount((current) => current + 1);
  }, []);

  const recordAutomaticOverlapActivity = useCallback((activity: AutomaticOverlapActivity) => {
    if (automaticOverlapActivityIdsRef.current.has(activity.id)) return;
    automaticOverlapActivityIdsRef.current.add(activity.id);
    setAutomaticOverlapActivities((current) => [activity, ...current]);
    if (!shellRef.current.activityOpen) {
      setUnreadAutomaticOverlapReviewIds((current) => new Set([...current, activity.reviewId]));
    }
  }, []);

  const recordDanbooruActivity = useCallback((activity: DanbooruSessionActivity) => {
    setDanbooruSessionActivities((current) => [activity, ...current].slice(0, 200));
    if (!shellRef.current.activityOpen) setUnreadActivityCount((current) => current + 1);
  }, []);

  const replaceExploreContextIds = useCallback((ids: string[]) => {
    exploreContextIdsRef.current = ids;
    setExploreContextIds(ids);
  }, []);

  const replaceActiveExploreContextId = useCallback((id: string | null) => {
    activeExploreContextIdRef.current = id;
    setActiveExploreContextId(id);
  }, []);

  const createExploreContext = useCallback((activate = true): ExploreContext => {
    const id = `explore-context-${++exploreContextSequence.current}`;
    const context: ExploreContext = {
      id,
      label: "검색",
      session: createExplorePageSession(),
      request: null,
      requestKey: null,
      displayValue: uiRef.current.search.explore.committed,
      languages: [...uiRef.current.search.explore.languages],
      sort: uiRef.current.exploreSort,
      query: initialGalleryQueryState,
      exploreIds: [],
      scrollTop: 0,
      keyboardFocusId: null,
      selectionIds: [],
      selectionAnchorId: null,
      lastAccessed: ++exploreContextAccessSequence.current,
    };
    if (activate) explorePageSession.current = context.session;
    exploreContexts.current.set(id, context);
    replaceExploreContextIds([...exploreContextIdsRef.current, id]);
    if (activate) replaceActiveExploreContextId(id);
    return context;
  }, [createExplorePageSession, replaceActiveExploreContextId, replaceExploreContextIds]);

  const snapshotActiveExploreContext = useCallback((park = true): ExploreContext | null => {
    const activeId = activeExploreContextIdRef.current;
    const context = activeId ? exploreContexts.current.get(activeId) : undefined;
    if (!context) return null;

    const currentQuery = queryRef.current;
    const viewportScroll = uiRef.current.view === "explore"
      ? galleryViewport.current?.scrollTop ?? context.scrollTop
      : context.scrollTop;
    if (currentQuery.page && uiRef.current.view === "explore") {
      context.session.recordScroll(currentQuery.page.page, viewportScroll);
    }
    context.query = currentQuery;
    context.exploreIds = [...exploreIdsRef.current];
    // Controls may contain the next search's draft, not the conditions that
    // own these cached results. A submitted context keeps its original search.
    if (!context.request) {
      context.displayValue = uiRef.current.search.explore.committed;
      context.languages = [...uiRef.current.search.explore.languages];
      context.sort = uiRef.current.exploreSort;
    }
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

  const captureNavigationOrigin = useCallback((): NavigationOrigin => ({
    view: uiRef.current.view,
    contextId: activeExploreContextIdRef.current,
    detailId: uiRef.current.detail.minimized ? null : uiRef.current.detail.activeId,
  }), []);
  const rememberClosed = useCallback((item: ClosedNavigation) => {
    closedNavigation.current = [...closedNavigation.current.slice(-23), item];
  }, []);

  const hydrateRestoredContext = useCallback(async (context: ExploreContext) => {
    const resume=context.resumePage;
    if (!resume || !context.request) return;
    delete context.resumePage;
    const lifetime=exploreLifetime.current;
    const publish=() => {
      replaceExploreContextIds([...exploreContextIdsRef.current]);
      if (activeExploreContextIdRef.current!==context.id) return;
      queryRef.current=context.query; exploreIdsRef.current=context.exploreIds;
      dispatchQuery({ type:"restore",state:context.query }); setExploreIds(context.exploreIds);
    };
    context.query={ ...initialGalleryQueryState,phase:"loading-page",queryId:resume.queryId,pendingPage:resume.page };
    const loading=context.query;
    publish();
    try {
      let result=resume.queryId ? await backend.searchPageGet(resume.queryId,resume.page,`restore-${context.id}`) : null;
      let queryId=resume.queryId;
      if (!result || (!result.ok && result.error.code==="QUERY_NOT_FOUND")) {
        const submitted=await backend.searchSubmit(context.request);
        if (!submitted.ok) result=submitted;
        else {
          queryId=submitted.data.queryId;
          result=resume.page > 1 ? await backend.searchPageGet(queryId, resume.page, `restore-${context.id}`) : { ok:true,data:submitted.data.firstPage };
        }
      }
      if (lifetime!==exploreLifetime.current || exploreContexts.current.get(context.id)!==context || context.query!==loading) return;
      if (!result.ok) { context.query={ ...context.query,phase:"error",error:result.error,pendingPage:null }; publish(); return; }
      context.query={ ...initialGalleryQueryState,phase:"ready",queryId,page:result.data };
      context.exploreIds=result.data.items.map((item)=>item.id);
      context.session.start(queryId!,result.data); context.session.recordScroll(result.data.page,context.scrollTop);
      if (activeExploreContextIdRef.current!==context.id) context.session.park();
      setGalleries((current)=>mergeGalleryPage(current,result.data).galleries);
      publish();
      window.requestAnimationFrame(()=>{
        if (activeExploreContextIdRef.current===context.id && galleryViewport.current) galleryViewport.current.scrollTop=context.scrollTop;
      });
    } catch {
      if (lifetime!==exploreLifetime.current || exploreContexts.current.get(context.id)!==context || context.query!==loading) return;
      context.query={ ...context.query,phase:"error",pendingPage:null,error:{ code:"BACKEND_UNAVAILABLE",message:"검색 탭을 복구하지 못했습니다. 새로고침으로 다시 시도할 수 있습니다.",retryable:true,action:"retry" } }; publish();
    }
  }, [replaceExploreContextIds]);

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
    dispatch({ type: "search.languages", view: "explore", languages: [...(context.request?.languages ?? context.languages)] });
    dispatch({ type: "sort.set", sort: context.request?.sort ?? context.sort });
    dispatch({ type: "search.commit", view: "explore", value: context.displayValue });
    dispatch({
      type: "selection.restore",
      ids: [...context.selectionIds],
      anchorId: context.selectionAnchorId,
    });
    dispatchQuery({ type: "restore", state: context.query });
    setExploreIds([...context.exploreIds]);
    setKeyboardFocusId(context.keyboardFocusId);
    if (context.resumePage) void hydrateRestoredContext(context);
    exploreRestoreFrame.current = window.requestAnimationFrame(() => {
      if (activeExploreContextIdRef.current === context.id && galleryViewport.current) {
        galleryViewport.current.scrollTop = context.scrollTop;
      }
      context.session.releaseRetainedPage();
      exploreRestoreFrame.current = null;
    });
  }, [hydrateRestoredContext, replaceActiveExploreContextId]);

  const returnToOrigin = useCallback((origin?: NavigationOrigin) => {
    if (!origin) return false;
    const context = origin.contextId ? exploreContexts.current.get(origin.contextId) : undefined;
    if (origin.view === "explore" && context) restoreExploreContext(context);
    else dispatch({ type: "navigate", view: origin.view });
    if (origin.detailId !== null && uiRef.current.detail.tabs.includes(origin.detailId as GalleryId)) {
      dispatch({ type: "detail.activate", id: origin.detailId as GalleryId });
    } else dispatch({ type: "detail.minimize", minimized: true });
    return true;
  }, [restoreExploreContext]);

  useEffect(() => {
    if (!shell.backgroundReady || checkpointRestored.current) return;
    checkpointRestored.current=true;
    if (!navigationCheckpoint || exploreContexts.current.size) return;
    let activeContext:ExploreContext | undefined;
    for (const tab of navigationCheckpoint.tabs) {
      const context=createExploreContext(false);
      restoredContextIds.current.set(tab.id, context.id);
      context.label=tab.label; context.displayValue=tab.displayValue; context.request=tab.request;
      context.requestKey=searchRequestKey(tab.request)+JSON.stringify([settings.searchIncludeTags,settings.searchExcludeTags]);
      context.languages=tab.request.languages; context.sort=tab.request.sort; context.scrollTop=tab.scrollTop;
      context.resumePage={ queryId:tab.queryId,page:tab.page };
      if (tab.id===navigationCheckpoint.activeTab) activeContext=context;
    }
    const remap = (origin: NavigationOrigin): NavigationOrigin => ({ ...origin,
      contextId: origin.contextId ? restoredContextIds.current.get(origin.contextId) ?? null : null });
    for (const tab of navigationCheckpoint.tabs) {
      const context = exploreContexts.current.get(restoredContextIds.current.get(tab.id)!);
      if (context && tab.origin) context.origin = remap(tab.origin);
    }
    for (const [id, origin] of navigationCheckpoint.detailOrigins ?? []) detailOrigins.current.set(id as GalleryId, remap(origin));
    for (const [id, position] of navigationCheckpoint.detailPositions ?? []) saveDetailPosition(id, position);
    if (activeContext) {
      replaceActiveExploreContextId(activeContext.id);
      if (navigationCheckpoint.view==="explore") restoreExploreContext(activeContext);
    }
  }, [shell.backgroundReady,navigationCheckpoint,createExploreContext,replaceActiveExploreContextId,restoreExploreContext,settings.searchIncludeTags,settings.searchExcludeTags]);

  useEffect(() => {
    if (backend.runtime!=="tauri") return;
    let previous="";
    const save=() => {
      if (!checkpointRestored.current) return;
      if (uiRef.current.view==="explore") snapshotActiveExploreContext(false);
      const value={ version:1 as const,savedAt:0,view:uiRef.current.view,downloadsFilter:uiRef.current.downloadsFilter,
        detail: uiRef.current.detail, detailOrigins: [...detailOrigins.current], detailPositions: [...readDetailPositions()],
        activeTab:activeExploreContextIdRef.current,tabs:[...exploreContexts.current.values()].flatMap((context)=>context.request ? [{
          id:context.id,label:context.label,displayValue:context.displayValue,request:context.request,
          origin: context.origin,
          queryId:context.resumePage?.queryId ?? context.query.queryId,page:context.resumePage?.page ?? context.query.page?.page ?? 1,scrollTop:context.scrollTop,
        }] : []) };
      const key=JSON.stringify(value); if (key===previous) return;
      previous=key;
      const checkpoint={ ...value,savedAt:Date.now() };
      writeNavigationCheckpoint(checkpoint);
      const bounded = readNavigationCheckpoint();
      if (bounded) persistNativeCheckpoint(bounded);
    };
    const timer=window.setInterval(save,2000);
    window.addEventListener("beforeunload",save);
    return () => { window.clearInterval(timer); window.removeEventListener("beforeunload",save); };
  }, [snapshotActiveExploreContext]);

  const activateExploreContext = useCallback((id: string) => {
    const target = exploreContexts.current.get(id);
    if (!target) return;
    if (activeExploreContextIdRef.current === id) {
      if (uiRef.current.view === "explore") snapshotActiveExploreContext(false);
      restoreExploreContext(target);
      return;
    }
    exploreNavigationToken.current += 1;
    snapshotActiveExploreContext(true);
    restoreExploreContext(target);
  }, [restoreExploreContext, snapshotActiveExploreContext]);

  const closeExploreContext = useCallback((id: string) => {
    const context = exploreContexts.current.get(id);
    if (!context) return;
    if (activeExploreContextIdRef.current === id) snapshotActiveExploreContext(false);
    const ids = exploreContextIdsRef.current;
    const closingIndex = ids.indexOf(id);
    rememberClosed({ kind: "explore", context, index: closingIndex, at: performance.now() });
    if (context.request) context.resumePage = { queryId: context.query.queryId, page: context.query.page?.page ?? 1 };
    const nextIds = ids.filter((contextId) => contextId !== id);
    context.session.clear();
    exploreContexts.current.delete(id);
    replaceExploreContextIds(nextIds);
    if (activeExploreContextIdRef.current !== id) return;
    exploreNavigationToken.current += 1;
    const fallbackId = nextIds[Math.max(0, closingIndex - 1)] ?? nextIds[0];
    const fallback = fallbackId ? exploreContexts.current.get(fallbackId) : undefined;
    if (fallback) restoreExploreContext(fallback);
    else {
      replaceActiveExploreContextId(null);
      queryRef.current = initialGalleryQueryState;
      exploreIdsRef.current = [];
      dispatchQuery({ type: "restore", state: initialGalleryQueryState });
      setExploreIds([]);
      setKeyboardFocusId(null);
      dispatch({ type: "selection.clear" });
      dispatch({ type: "search.commit", view: "explore", value: "" });
    }
    returnToOrigin(context.origin);
  }, [replaceActiveExploreContextId, replaceExploreContextIds, restoreExploreContext, returnToOrigin, rememberClosed, snapshotActiveExploreContext]);

  const navigateView = useCallback((view: ViewId) => {
    setPersonalLibraryOpen(false);
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

  const appliedNavigation = useRef<unknown>(null);
  useEffect(() => {
    if (!navigationRequest || navigationRequest === appliedNavigation.current || navigationRequest.source !== "hitomi") return;
    const { view } = navigationRequest;
    if (view !== "explore" && view !== "downloads" && view !== "auto-find") return;
    appliedNavigation.current = navigationRequest;
    navigateView(view);
  }, [navigationRequest, navigateView]);

  const loadExplorationExclusionsAndSync = useCallback(async () => {
    const token = ++explorationExclusionsHydrationToken.current;
    setExplorationExclusionsReady(false);
    const result = await loadExplorationExclusions();
    if (token !== explorationExclusionsHydrationToken.current) return result;
    if (result.ok) {
      setExplorationExcludedGalleryIds(new Set(result.data.map((item) => item.galleryId)));
      setDuplicateHiddenGalleryIds(new Set(result.data
        .filter((item) => item.reasons.some((reason) => reason.kind === "duplicate_hidden"))
        .map((item) => item.galleryId)));
      setExplorationExclusionsReady(true);
    }
    return result;
  }, []);

  const restoreExplorationExclusionsAndSync = useCallback(async (galleryIds: GalleryId[]) => {
    const result = await restoreExplorationExclusions(galleryIds);
    if (result.ok) {
      ++explorationExclusionsHydrationToken.current;
      setExplorationExclusionsReady(false);
      const restored = new Set(result.data.restoredGalleryIds);
      setExplorationExcludedGalleryIds((current) => new Set([...current].filter((id) => !restored.has(id))));
      setDuplicateHiddenGalleryIds((current) => new Set([...current].filter((id) => !restored.has(id))));
      void loadExplorationExclusionsAndSync().catch(() => undefined);
    }
    return result;
  }, [loadExplorationExclusionsAndSync]);

  const beginDownloadOverlapAutomationHistoryMutation = useCallback((reviewId: string): boolean => {
    if (downloadOverlapDecisionPendingRef.current
      || downloadOverlapAutomationHistoryPendingRef.current.has(reviewId)) return false;
    downloadOverlapAutomationHistoryPendingRef.current.add(reviewId);
    setDownloadOverlapAutomationHistoryPendingReviewIds(
      new Set(downloadOverlapAutomationHistoryPendingRef.current),
    );
    return true;
  }, []);

  const finishDownloadOverlapAutomationHistoryMutation = useCallback((reviewId: string) => {
    downloadOverlapAutomationHistoryPendingRef.current.delete(reviewId);
    setDownloadOverlapAutomationHistoryPendingReviewIds(
      new Set(downloadOverlapAutomationHistoryPendingRef.current),
    );
  }, []);

  const applyAcknowledgedDownloadOverlapAutomationHistory = useCallback((
    item: DownloadOverlapAutomationHistoryItem,
  ) => {
    ++downloadOverlapAutomationHistoryBodyToken.current;
    setDownloadOverlapAutomationHistoryLoading(false);
    setDownloadOverlapAutomationHistory((current) => current.map((known) =>
      known.reviewId === item.reviewId ? item : known));
    const session = automationReviewSessionRef.current;
    if (session) {
      const updated = {
        ...session,
        items: session.items.map((known) => known.reviewId === item.reviewId ? item : known),
      };
      automationReviewSessionRef.current = updated;
      setAutomationReviewSession(updated);
    }
    setUnreadAutomaticOverlapReviewIds((current) => {
      const next = new Set(current);
      next.delete(item.reviewId);
      return next;
    });
  }, []);

  const acknowledgeDownloadOverlapAutomationHistory = useCallback(async (reviewId: string) => {
    if (!beginDownloadOverlapAutomationHistoryMutation(reviewId)) return false;
    try {
      const result = await backend.downloadOverlapAutomationHistoryAcknowledge(reviewId);
      if (!result.ok) {
        showToast(result.error.message);
        return false;
      }
      applyAcknowledgedDownloadOverlapAutomationHistory(result.data);
      setDownloadOverlapAutomationHistoryUnacknowledgedItems((current) => Math.max(0, current - 1));
      void hydrateDownloadOverlapAutomationHistoryMeta();
      return true;
    } catch {
      showToast("자동 판본 분류 기록을 확인 처리하지 못했습니다.");
      return false;
    } finally {
      finishDownloadOverlapAutomationHistoryMutation(reviewId);
    }
  }, [applyAcknowledgedDownloadOverlapAutomationHistory, beginDownloadOverlapAutomationHistoryMutation, finishDownloadOverlapAutomationHistoryMutation, hydrateDownloadOverlapAutomationHistoryMeta, showToast]);

  const restoreDownloadOverlapAutomationExclusions = useCallback(async (
    reviewId: string,
    galleryIds: GalleryId[],
  ) => {
    if (!galleryIds.length || !beginDownloadOverlapAutomationHistoryMutation(reviewId)) return false;
    try {
      const restored = await restoreExplorationExclusionsAndSync(galleryIds);
      if (!restored.ok) {
        showToast(restored.error.message);
        return false;
      }
      setDownloadsRefresh((current) => current + 1);
      const acknowledged = await backend.downloadOverlapAutomationHistoryAcknowledge(reviewId);
      if (!acknowledged.ok) {
        showToast(`목록 제외 ${restored.data.restoredGalleryIds.length}개는 해제했지만 기록 확인 처리는 실패했습니다. ${acknowledged.error.message}`);
        void hydrateDownloadOverlapAutomationHistoryMeta();
        return false;
      }
      applyAcknowledgedDownloadOverlapAutomationHistory(acknowledged.data);
      setDownloadOverlapAutomationHistoryUnacknowledgedItems((current) => Math.max(0, current - 1));
      void hydrateDownloadOverlapAutomationHistoryMeta();
      showToast(`${restored.data.restoredGalleryIds.length}개 앨범의 탐색·목록 제외를 해제했습니다. 격리된 실제 파일은 복원하지 않았습니다.`);
      return true;
    } catch {
      showToast("자동 분류 제외 앨범을 목록에 복원하지 못했습니다.");
      return false;
    } finally {
      finishDownloadOverlapAutomationHistoryMutation(reviewId);
    }
  }, [applyAcknowledgedDownloadOverlapAutomationHistory, beginDownloadOverlapAutomationHistoryMutation, finishDownloadOverlapAutomationHistoryMutation, hydrateDownloadOverlapAutomationHistoryMeta, restoreExplorationExclusionsAndSync, showToast]);

  useEffect(() => {
    void loadExplorationExclusionsAndSync().catch(() => undefined);
  }, [downloadsRefresh, loadExplorationExclusionsAndSync]);

  useEffect(() => {
    if (preferencesPending()) return;
    dispatch({ type: "grouping.set", view: "auto-find", grouping: settings.autoFindGrouping });
    dispatch({ type: "grouping.set", view: "downloads", grouping: settings.downloadsGrouping === "artist" ? "artist" : "all" });
    dispatch({ type: "displayMode.set", view: "explore", mode: settings.exploreDisplayMode });
    dispatch({ type: "displayMode.set", view: "auto-find", mode: settings.autoFindDisplayMode });
    dispatch({ type: "displayMode.set", view: "downloads", mode: settings.downloadsDisplayMode });
  }, [preferencesPending, settings.autoFindDisplayMode, settings.autoFindGrouping, settings.downloadsDisplayMode, settings.downloadsGrouping, settings.exploreDisplayMode]);

  const persistGalleryGrouping = useCallback((view: "auto-find" | "downloads", grouping: GalleryGrouping) => {
    dispatch({ type: "grouping.set", view, grouping });
    const patch: SettingsPatch = view === "auto-find"
      ? { autoFindGrouping: grouping }
      : { downloadsGrouping: grouping };
    persistListPreferences(patch);
  }, [persistListPreferences]);

  const persistGalleryDisplayMode = useCallback((view: ViewId, mode: GalleryDisplayMode) => {
    dispatch({ type: "displayMode.set", view, mode });
    const patch: SettingsPatch = view === "explore"
      ? { exploreDisplayMode: mode }
      : view === "auto-find"
        ? { autoFindDisplayMode: mode }
        : { downloadsDisplayMode: mode };
    persistListPreferences(patch);
  }, [persistListPreferences]);

  useEffect(() => {
    if (preferencesPending()) return;
    setCollapsedGroupKeys(new Set(settings.collapsedGroupKeys));
  }, [preferencesPending, settings.collapsedGroupKeys]);

  useEffect(() => {
    // Download groups are intentionally a session-local folder view. Entering a
    // different grouping starts closed so thousands of cards are never mounted
    // merely by opening Downloads.
    setExpandedDownloadGroupKeys(new Set());
  }, [ui.grouping.downloads]);

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
        sessionCoverRetainer.current?.clear();
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
    const request = ++searchHistoryRequest.current;
    try {
      const result = await backend.searchHistoryList(100);
      if (request === searchHistoryRequest.current && result.ok) setSearchHistory(result.data);
    } catch {
      // Search history is an enhancement; a transient failure must not block searching.
    }
  }, []);

  const removeSearchHistory = useCallback(async (historyId?: number) => {
    if (historyMutationBusy.current) return;
    historyMutationBusy.current = true; setHistoryPending(true); searchHistoryRequest.current++;
    try {
      const result = historyId === undefined ? await backend.searchHistoryClear() : await backend.searchHistoryRemove(historyId);
      if (!result.ok) { showToast(result.error.message); return; }
      if (historyId === undefined) setSearchHistory([]);
      else setSearchHistory((current) => {
        const selected = current.find((item) => item.historyId === historyId);
        const key = (item: SearchHistoryEntry) => JSON.stringify([item.text, item.includeTags, item.excludeTags, item.languages, item.sort]);
        return selected ? current.filter((item) => key(item) !== key(selected)) : current;
      });
      await hydrateSearchHistory();
    } catch { showToast("검색 기록을 지우지 못했습니다. 다시 시도해 주세요."); }
    finally { historyMutationBusy.current = false; setHistoryPending(false); }
  }, [hydrateSearchHistory, showToast]);

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
    if (showLoading) setAutoFindLoading(true);
    autoFindHydrationAgain.current=true;
    if (autoFindHydrationTask.current) return autoFindHydrationTask.current;
    const lifetime=exploreLifetime.current;
    const task=(async()=>{
      do {
        autoFindHydrationAgain.current=false;
        const token=++autoFindHydrationToken.current;
        try {
          const result=await backend.autoFindSnapshot();
          if (lifetime!==exploreLifetime.current || token!==autoFindHydrationToken.current) return;
          if (!result.ok) setAutoFindError(result.error.message);
          else { setAutoFindError(null); applyAutoFindSnapshot(result.data); }
        } catch {
          if (lifetime===exploreLifetime.current) setAutoFindError("자동 탐색 backend에 연결하지 못했습니다.");
        }
      } while (autoFindHydrationAgain.current && lifetime===exploreLifetime.current);
    })();
    autoFindHydrationTask.current=task;
    try { await task; } finally {
      if (autoFindHydrationTask.current===task) { autoFindHydrationTask.current=null; setAutoFindLoading(false); }
    }
  }, [applyAutoFindSnapshot]);

  const autoFindRulesKey = JSON.stringify([settings.searchIncludeTags, settings.searchExcludeTags]);
  const appliedAutoFindRules = useRef(autoFindRulesKey);
  useEffect(() => {
    if (!shell.backgroundReady || settingsLoading || appliedAutoFindRules.current === autoFindRulesKey) return;
    appliedAutoFindRules.current = autoFindRulesKey;
    // Reproject saved discoveries, not another network scan. Relaxed rules must
    // also bring previously hidden candidates back without losing checkpoints.
    void hydrateAutoFind();
  }, [autoFindRulesKey, hydrateAutoFind, settingsLoading, shell.backgroundReady]);

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


  useEffect(() => {
    if (!shell.backgroundReady) return;
    void hydrateFavorites();
    void hydrateSearchHistory();
    void hydrateTagCatalogStatus();
    void hydrateAutoFind(true);
    void hydrateDuplicateSnapshot(true);
    void hydrateInternalSnapshot(true);
  }, [shell.backgroundReady, hydrateAutoFind, hydrateDuplicateSnapshot, hydrateFavorites, hydrateInternalSnapshot, hydrateSearchHistory, hydrateTagCatalogStatus]);

  useLayoutEffect(() => {
    const viewport = galleryViewport.current;
    if (!viewport) return;
    let lastColumns = "";
    const update = () => {
      const detailColumns = resolveGalleryColumns(viewport.clientWidth, maximumColumns, previewWidth);
      const next = currentGalleryDisplayMode === "compact"
        ? resolveCompactGalleryColumns(viewport.clientWidth, previewWidth)
        : detailColumns;
      const columnsKey = `${next}:${detailColumns}`;
      if (columnsKey === lastColumns) return;
      lastColumns = columnsKey;
      setGalleryColumns((current) => current === next ? current : next);
      setArtistFolderColumns((current) => current === detailColumns ? current : detailColumns);
    };
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [active, currentGalleryDisplayMode, maximumColumns, previewWidth, settingsLoading]);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void backend.on("download:changed", (event: DownloadChangedEvent) => {
      const change = downloadProgress.apply(event, galleriesRef.current.get(galleryId(event.galleryId))?.download);
      if (!change.applied || !change.structural) return;
      workQueue.changed();
      recordSessionDownloadActivity(galleryId(event.galleryId), event.state);
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
  }, [downloadProgress, recordSessionDownloadActivity, showToast, workQueue.changed]);

  useEffect(() => {
    let disposed=false; let unsubscribe:(() => void) | undefined;
    void backend.on("download:resync", () => setDownloadsRefresh((revision) => revision + 1))
      .then((cleanup) => { if (disposed) cleanup(); else unsubscribe=cleanup; }).catch(() => undefined);
    return () => { disposed=true; unsubscribe?.(); };
  }, []);

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

  const publishExploreContext = useCallback((context: ExploreContext) => {
    // Background jobs own their state; only the selected tab owns the viewport.
    replaceExploreContextIds([...exploreContextIdsRef.current]);
    if (activeExploreContextIdRef.current !== context.id) return;
    queryRef.current = context.query;
    exploreIdsRef.current = context.exploreIds;
    dispatchQuery({ type: "restore", state: context.query });
    setExploreIds(context.exploreIds);
  }, [replaceExploreContextIds]);

  const startExploreSearch = useCallback((
    sourceRequest: SearchRequest,
    options: { displayValue: string; label?: string; replace?: boolean; background?: boolean },
  ) => {
    const request = cloneSearchRequest(sourceRequest);
    const key = searchRequestKey(request) + JSON.stringify([settings.searchIncludeTags, settings.searchExcludeTags]);
    const activeId = activeExploreContextIdRef.current;
    let context = options.replace && activeId ? exploreContexts.current.get(activeId) : undefined;
    if (!options.replace) {
      const existing = [...exploreContexts.current.values()].find((item) => item.requestKey === key);
      if (existing) {
        if (!options.background) {
          // Revisiting an existing search still needs a return edge to the invoking album.
          if (uiRef.current.detail.activeId !== null && !uiRef.current.detail.minimized) existing.origin = captureNavigationOrigin();
          activateExploreContext(existing.id);
        }
        return true;
      }
      if (exploreContexts.current.size >= maximumExploreContexts) {
        showToast(`검색 탭은 최대 ${maximumExploreContexts}개까지 열 수 있습니다. 사용하지 않는 탭을 닫아 주세요.`);
        return false;
      }
      snapshotActiveExploreContext(!options.background);
    }
    const origin = captureNavigationOrigin();
    context ??= createExploreContext(!options.background);
    if (!options.replace) context.origin = origin;
    const target = context;
    const token = ++searchToken.current;
    const lifetime = exploreLifetime.current;
    if (!options.background) exploreNavigationToken.current += 1;
    if (!options.background && exploreRestoreFrame.current !== null) {
      window.cancelAnimationFrame(exploreRestoreFrame.current);
      exploreRestoreFrame.current = null;
    }
    context.session.clear();
    if (!options.background) explorePageSession.current = context.session;
    context.request = request;
    context.requestKey = key;
    context.displayValue = options.displayValue.trim();
    context.languages = [...request.languages];
    context.sort = request.sort;
    context.label = options.label?.trim() || options.displayValue.trim() || "검색";
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
    if (!options.background) {
    queryRef.current = submitting;
    exploreIdsRef.current = [];
    keyboardFocusIdRef.current = null;
    dispatch({ type: "navigate", view: "explore" });
    dispatch({ type: "search.languages", view: "explore", languages: [...request.languages] });
    dispatch({ type: "sort.set", sort: request.sort });
    dispatch({ type: "search.commit", view: "explore", value: context.displayValue });
    dispatch({ type: "selection.clear" });
    dispatchQuery({ type: "restore", state: submitting });
    setExploreIds([]);
    setKeyboardFocusId(null);
    if (uiRef.current.view === "explore" && galleryViewport.current) galleryViewport.current.scrollTop = 0;
    }
    publishExploreContext(context);
    const isCurrent = () => lifetime === exploreLifetime.current
      && exploreContexts.current.get(target.id) === target
      && target.query.submitToken === token;
    const fail = (error: NonNullable<GalleryQueryState["error"]>) => {
      if (!isCurrent()) return;
      target.query = { ...target.query, phase: "error", error };
      publishExploreContext(target);
    };
    void backend.searchSubmit(request).then((result) => {
      if (!isCurrent()) return;
      if (!result.ok) {
        fail(result.error);
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
      target.query = ready;
      target.exploreIds = resultIds;
      target.scrollTop = 0;
      target.session.start(result.data.queryId, result.data.firstPage);
      publishExploreContext(target);
      setGalleries((current) => mergeGalleryPage(current, result.data.firstPage).galleries);
      if (activeExploreContextIdRef.current === target.id && uiRef.current.view === "explore") {
        if (galleryViewport.current) galleryViewport.current.scrollTop = 0;
      } else {
        target.session.park();
      }
      target.session.prefetchAdjacent();
      if (request.text.trim() || request.includeTags.length || request.excludeTags.length) {
        void hydrateSearchHistory();
      }
    }).catch(() => {
      fail({ code: "BACKEND_UNAVAILABLE", message: "검색 backend에 연결하지 못했습니다.", retryable: true, action: "retry" });
    });
    return true;
  }, [activateExploreContext, createExploreContext, hydrateSearchHistory, publishExploreContext,
    settings.searchIncludeTags, settings.searchExcludeTags, showToast, snapshotActiveExploreContext, captureNavigationOrigin]);

  useEffect(() => {
    if (!shell.backgroundReady) return;
    let cancelled = false;
    const token = ++downloadHydrationToken.current;
    setDownloadsLoading(true);
    setDownloadsError(null);
    void (async () => {
      const nextIds: GalleryId[] = [];
      const nextIdSet = new Set<GalleryId>();
      let page = 1;
      let totalItems = 0;
      let loadedItems = 0;
      do {
        let result: DownloadLibraryPage | null = null;
        try {
          const loaded = await backend.downloadLibraryPageList({ page, pageSize: 200 });
          if (loaded.ok) result = loaded.data;
        } catch {
          result = null;
        }
        if (!result) {
          const fallback = await backend.downloadEntriesList({ page, pageSize: 200 });
          if (cancelled || token !== downloadHydrationToken.current) return;
          if (!fallback.ok) {
            setDownloadsError(fallback.error.message);
            return;
          }
          result = downloadLibraryPageFromEntries(fallback.data);
        }
        if (cancelled || token !== downloadHydrationToken.current) return;
        if (result.items.length === 0) break;
        loadedItems += result.items.length;
        setGalleries((current) => mergeDownloadLibraryPage(current, result).galleries);
        for (const item of result.items) {
          const id = item.gallery.id;
          if (nextIdSet.has(id)) continue;
          nextIdSet.add(id);
          nextIds.push(id);
        }
        setDownloadIds([...nextIds]);
        totalItems = result.totalItems;
        page += 1;
        // Retain every loaded summary, but yield between batches so native input
        // and the first usable frame are not starved by a large saved library.
        if (loadedItems < totalItems) await new Promise<void>((resolve) => window.setTimeout(resolve, 16));
      } while (loadedItems < totalItems);
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
  }, [downloadsRefresh, shell.backgroundReady]);

  const autoFindMatchedFavoriteTokens = useMemo(() => new Map(
    autoFindSnapshot.candidates.map((candidate) => [candidate.id, (candidate.matchedFavorites?.length ? candidate.matchedFavorites : [candidate.matchedFavorite]).map(favoriteToken)]),
  ), [autoFindSnapshot.candidates]);
  const savedPreviews = useSavedGalleryPreviews(backend, galleries);
  const setRepresentativePreview = useCallback(async (id: GalleryId, sourcePage: number | null): Promise<boolean> => {
    try {
      await savedPreviews.save(id, sourcePage);
      showToast(sourcePage === null ? "대표 미리보기를 자동 선택으로 되돌렸습니다." : `${sourcePage}페이지를 대표 미리보기로 저장했습니다.`);
      return true;
    } catch (error) {
      showToast(error instanceof Error ? error.message : "대표 미리보기를 저장하지 못했습니다.");
      return false;
    }
  }, [savedPreviews.save, showToast]);
  const displayGalleries = useMemo<ReadonlyMap<GalleryId, Gallery>>(() => {
    const next = new Map<GalleryId, Gallery>();
    galleries.forEach((gallery, id) => {
      const matchedFavoriteToken = autoFindMatchedFavoriteTokens.get(id);
      const favorite = autoFindFavoriteMatches(gallery, favoriteMetadata, matchedFavoriteToken).length > 0;
      const representativePreview = savedPreviews.previews.get(id);
      const validPreview = representativePreview?.sourcePage && representativePreview.entryId === gallery.download?.entryId
        && gallery.download?.state === "completed";
      next.set(id, !representativePreview && gallery.favorite === favorite ? gallery : {
        ...gallery, favorite, representativePreview,
        ...(validPreview && representativePreview.width && representativePreview.height
          ? { thumbnailWidth: representativePreview.width, thumbnailHeight: representativePreview.height } : {}),
      });
    });
    return next;
  }, [autoFindMatchedFavoriteTokens, favoriteMetadata, galleries, savedPreviews.previews]);
  const favoriteMetadataForDisplay = useMemo<ReadonlySet<string>>(() => {
    const next = new Set(favoriteMetadata);
    galleries.forEach((gallery) => {
      const artistToken = `artist:${gallery.artist}`;
      if (favoriteMetadata.has(normalizeMetadataToken(artistToken))) next.add(artistToken);
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

  const pendingAutoFindIds = useMemo(
    () => autoFindIds.filter((id) => {
      const gallery = displayGalleries.get(id);
      return gallery !== undefined && isPendingAutoFindCandidate(gallery)
        && matchesGlobalSearchRules(gallery, settings.searchIncludeTags, settings.searchExcludeTags);
    }),
    [autoFindIds, displayGalleries, settings.searchIncludeTags, settings.searchExcludeTags],
  );
  const autoFindSearchState = ui.search["auto-find"];
  const filteredPendingAutoFindGalleries = useMemo(() => visibleGalleries(
    {
      ...initialUiState,
      view: "auto-find",
      search: { ...initialUiState.search, "auto-find": autoFindSearchState },
    },
    pendingAutoFindIds.flatMap((id) => {
      const gallery = displayGalleries.get(id);
      return gallery ? [gallery] : [];
    }),
  ), [autoFindSearchState, displayGalleries, pendingAutoFindIds]);

  const scopedGalleries = useMemo(() => {
    const ids = ui.view === "explore" ? exploreIds : ui.view === "downloads" ? downloadIds : pendingAutoFindIds;
    return ids.flatMap((id) => {
      const gallery = displayGalleries.get(id);
      return gallery ? [gallery] : [];
    });
  }, [displayGalleries, downloadIds, exploreIds, pendingAutoFindIds, ui.view]);
  // Explore is a submitted server query. Draft language/sort controls must not
  // filter or reorder the existing response before the next search is submitted.
  const visible = useMemo(() => ui.view === "explore" ? scopedGalleries : visibleGalleries(ui, scopedGalleries, popularity.ranks), [ui, scopedGalleries, popularity.ranks]);
  const actionableVisibleIds = useMemo(
    () => visible
      .filter((gallery) => gallery.download?.state !== "quarantined"
        && !(ui.view === "explore" && (duplicateHiddenGalleryIds.has(gallery.id) || explorationExcludedGalleryIds.has(gallery.id))))
      .map((gallery) => gallery.id),
    [duplicateHiddenGalleryIds, explorationExcludedGalleryIds, ui.view, visible],
  );
  galleriesRef.current = displayGalleries;
  const allGalleries = useMemo(() => [...displayGalleries.values()], [displayGalleries]);
  const pendingOverlapIdsKey = JSON.stringify([...new Set(allGalleries.flatMap((gallery) =>
    gallery.download?.state === "review_required" && gallery.download.reviewKind === "gallery_duplicate"
      && gallery.download.reviewId ? [gallery.download.reviewId] : []))].sort());
  const overlapInventoryEnabled = shell.activityOpen || downloadOverlapReviewId !== null;
  useEffect(() => {
    if (!overlapInventoryEnabled) return;
    let cancelled = false;
    const ids = JSON.parse(pendingOverlapIdsKey) as string[];
    setOverlapInventoryLoading(true);
    void (async () => {
      const reviews: DownloadOverlapReview[] = [];
      let next = 0;
      await Promise.all(Array.from({ length: Math.min(6, ids.length) }, async () => {
        while (!cancelled && next < ids.length) {
          const id = ids[next++]!;
          try {
            const result = await backend.downloadOverlapReviewGet(id);
            if (result.ok && result.data.state === "pending") reviews.push(result.data);
          } catch { /* The selected review keeps its own explicit retry/error UI. */ }
        }
      }));
      if (!cancelled) {
        setOverlapInventory(prioritizeDownloadOverlapReviews(reviews));
        setOverlapInventoryLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [overlapInventoryEnabled, overlapInventoryRefresh, pendingOverlapIdsKey]);
  const currentOverlapInventory = useMemo(() => downloadOverlapReview
    ? [downloadOverlapReview, ...overlapInventory.filter((review) => review.reviewId !== downloadOverlapReview.reviewId)]
    : overlapInventory, [downloadOverlapReview, overlapInventory]);
  const containmentGroup = useMemo(() => downloadOverlapReview
    ? buildDownloadOverlapContainmentGroup(containmentKeeperId ?? downloadOverlapReview.incoming.galleryId, currentOverlapInventory)
    : null, [containmentKeeperId, currentOverlapInventory, downloadOverlapReview]);
  const priorityContainmentGroups = useMemo(() => {
    const keepers = new Set(currentOverlapInventory.flatMap((review) => [review.incoming.galleryId,
      ...review.candidates.map((candidate) => candidate.existing.galleryId)]));
    return [...keepers].flatMap((id) => {
      const group = buildDownloadOverlapContainmentGroup(id, currentOverlapInventory);
      return group && group.items.length >= 2 ? [group] : [];
    }).sort((a, b) => b.items.length - a.items.length || b.keeper.pageCount - a.keeper.pageCount);
  }, [currentOverlapInventory]);
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
  const autoFindCount = filteredPendingAutoFindGalleries.length;
  const attentionCount = useMemo(
    () => allGalleries.filter((gallery) => !duplicateHiddenGalleryIds.has(gallery.id)
      && ["failed", "interrupted", "review_required"].includes(gallery.download?.state ?? "")).length,
    [allGalleries, duplicateHiddenGalleryIds],
  );
  const selectGallery = useCallback(
    (id: GalleryId, modifiers: { ctrlKey: boolean; shiftKey: boolean }) => {
      dispatch({ type: "selection.click", id, visibleIds: visibleIdsRef.current, ctrl: modifiers.ctrlKey, shift: modifiers.shiftKey });
    },
    [],
  );

  useLayoutEffect(() => {
    dispatch({ type: "selection.retain", ids: actionableVisibleIds });
  }, [actionableVisibleIds]);

  const hydrateDetail = useCallback(async (id: GalleryId, options?: { silent?: boolean }) => {
    if (hydratedDetails.current.has(id) || hydratingDetails.current.has(id)) return;
    hydratingDetails.current.add(id);
    try {
      const result = await backend.galleryDetailGet(id);
      if (!result.ok) {
        if (!options?.silent) showToast(result.error.message);
        return;
      }
      hydratedDetails.current.add(id);
      setGalleries((current) => mergeGalleryDetail(current, result.data));
    } catch {
      if (!options?.silent) showToast("상세 정보를 불러오지 못했습니다.");
    } finally {
      hydratingDetails.current.delete(id);
    }
  }, [showToast]);
  const refreshMergedGallery = useCallback((id: GalleryId) => {
    hydratedDetails.current.delete(id);
    hydratedCardDetails.current.delete(id);
    void hydrateDetail(id, { silent: true });
  }, [hydrateDetail]);
  const hydrateDownloadCardDetail = useCallback(async (id: GalleryId) => {
    if (hydratedDetails.current.has(id)
      || hydratingDetails.current.has(id)
      || hydratedCardDetails.current.has(id)
      || hydratingCardDetails.current.has(id)) return;
    hydratingCardDetails.current.add(id);
    try {
      const result = await backend.gallerySummaryGet(id);
      if (!result.ok) return;
      hydratedCardDetails.current.add(id);
      setGalleries((current) => mergeGalleryPage(current, {
        page: 1,
        totalPages: 1,
        items: [result.data],
      }).galleries);
    } catch {
      // The durable local summary remains usable; a later viewport entry can retry.
    } finally {
      hydratingCardDetails.current.delete(id);
    }
  }, []);
  const pumpNearbyDownloadDetails = useCallback(() => {
    while (nearbyDetailWorkers.current < 6 && (nearbyDetailQueue.current.length || artistDetailQueue.current.length)) {
      const nearby = nearbyDetailQueue.current.length > 0;
      const id = nearby ? nearbyDetailQueue.current.shift() : artistDetailQueue.current.shift();
      if (id === undefined) break;
      if (nearby) {
        if (!nearbyDetailQueued.current.delete(id)) continue;
      } else {
        if (!artistDetailWanted.current.has(id) || artistDetailAttempted.current.has(id)) continue;
        artistDetailAttempted.current.add(id);
      }
      if (hydratedDetails.current.has(id)
        || hydratingDetails.current.has(id)
        || hydratedCardDetails.current.has(id)
        || hydratingCardDetails.current.has(id)) continue;
      nearbyDetailWorkers.current += 1;
      void hydrateDownloadCardDetail(id).finally(() => {
        nearbyDetailWorkers.current = Math.max(0, nearbyDetailWorkers.current - 1);
        nearbyDetailPump.current();
      });
    }
  }, [hydrateDownloadCardDetail]);
  nearbyDetailPump.current = pumpNearbyDownloadDetails;
  const hydrateNearbyDownloadDetails = useCallback((ids: readonly GalleryId[]) => {
    for (const id of [...ids].reverse()) {
      if (hydratedDetails.current.has(id)
        || hydratingDetails.current.has(id)
        || hydratedCardDetails.current.has(id)
        || hydratingCardDetails.current.has(id)
        || nearbyDetailQueued.current.has(id)) continue;
      nearbyDetailQueued.current.add(id);
      nearbyDetailQueue.current.unshift(id);
    }
    nearbyDetailPump.current();
  }, []);
  const discardNearbyDownloadDetails = useCallback((ids: readonly GalleryId[]) => {
    for (const id of ids) nearbyDetailQueued.current.delete(id);
  }, []);
  useEffect(() => {
    if (ui.view === "downloads" && active) return;
    nearbyDetailQueued.current.clear();
    nearbyDetailQueue.current = [];
  }, [active, ui.view]);
  useEffect(() => () => {
    nearbyDetailQueued.current.clear();
    nearbyDetailQueue.current = [];
    artistDetailWanted.current.clear();
    artistDetailQueue.current = [];
  }, []);
  const openDetail = useCallback((id: GalleryId, options?: BackgroundOpenOptions) => {
    if (!uiRef.current.detail.tabs.includes(id)) detailOrigins.current.set(id, captureNavigationOrigin());
    dispatch({ type: "detail.open", id, activate: !options?.background });
    void hydrateDetail(id);
  }, [hydrateDetail, captureNavigationOrigin]);
  const openSavedItem = useCallback((item: Bookmark, options?: SavedItemOpenOptions) => {
    const saved = galleryFor(item);
    setGalleries((current) => {
      const next = new Map(current);
      const previous = current.get(saved.id);
      next.set(saved.id, { ...saved, ...previous, download: saved.download ?? previous?.download });
      return next;
    });
    openDetail(saved.id);
    setSavedPageRequest((current) => ({ galleryId: saved.id, page: options?.detailOnly ? 0 : item.page, sequence: (current?.sequence ?? 0) + 1 }));
  }, [openDetail]);
  const openSelectedDetails = useCallback((ids: GalleryId[]) => {
    const origin = captureNavigationOrigin();
    const pending = [...new Set(ids)];
    pending.forEach((id, index) => {
      if (!uiRef.current.detail.tabs.includes(id)) detailOrigins.current.set(id, origin);
      dispatch({ type: "detail.open", id, activate: index === 0 });
    });
    // Tabs appear immediately; bound metadata I/O even for a large selection.
    const hydrateNext = async () => {
      while (pending.length) {
        const id = pending.shift();
        if (id !== undefined) await hydrateDetail(id, { silent: true });
      }
    };
    for (let worker = 0; worker < Math.min(3, ids.length); worker += 1) void hydrateNext();
  }, [captureNavigationOrigin, hydrateDetail]);
  const detailCheckpointRestored = useRef(false);
  useEffect(() => {
    if (!shell.backgroundReady || !checkpointRestored.current || detailCheckpointRestored.current) return;
    detailCheckpointRestored.current = true;
    const detail = navigationCheckpoint?.detail;
    if (!detail) return;
    for (const id of detail.tabs) {
      dispatch({ type: "detail.open", id, activate: false });
    }
    // Restore the foreground first, with bounded metadata I/O for large saved sessions.
    const pending = [...detail.tabs].sort((a, b) => Number(b === detail.activeId) - Number(a === detail.activeId));
    const hydrateNext = async () => {
      while (pending.length) {
        const id = pending.shift();
        if (id !== undefined) await hydrateDetail(id, { silent: true });
      }
    };
    const workerCount = Math.min(3, pending.length);
    for (let worker = 0; worker < workerCount; worker += 1) void hydrateNext();
    if (detail.activeId !== null && detail.tabs.includes(detail.activeId)) dispatch({ type: "detail.activate", id: detail.activeId });
    dispatch({ type: "detail.minimize", minimized: detail.minimized });
  }, [hydrateDetail, navigationCheckpoint, shell.backgroundReady]);
  const closeDetail = useCallback((id: GalleryId) => {
    const detail = uiRef.current.detail;
    if (!detail.tabs.includes(id)) return;
    rememberClosed({ kind: "detail", ids: [id], activeId: id, before: [...detail.tabs], at: performance.now() });
    dispatch({ type: "detail.close", id });
    if (detail.activeId === id && !detail.minimized) returnToOrigin(detailOrigins.current.get(id));
  }, [rememberClosed, returnToOrigin]);
  const closeAllDetails = useCallback(() => {
    const detail = uiRef.current.detail;
    rememberClosed({ kind: "detail", ids: [...detail.tabs], activeId: detail.activeId, before: [...detail.tabs], at: performance.now() });
    dispatch({ type: "detail.closeAll" });
  }, [rememberClosed]);
  const reopenClosedNavigation = useCallback(() => {
    const item = closedNavigation.current.at(-1);
    if (!item) return false;
    if (item.kind === "explore" && exploreContexts.current.size >= maximumExploreContexts) {
      showToast("검색 탭 한도에 도달했습니다. 탭 하나를 닫은 뒤 복원해 주세요.");
      return true;
    }
    closedNavigation.current.pop();
    if (item.kind === "explore") {
      snapshotActiveExploreContext(true);
      exploreContexts.current.set(item.context.id, item.context);
      const ids = [...exploreContextIdsRef.current];
      ids.splice(Math.min(item.index, ids.length), 0, item.context.id);
      replaceExploreContextIds(ids);
      dispatch({ type: "detail.minimize", minimized: true });
      restoreExploreContext(item.context);
    } else {
      for (const id of item.ids) {
        const index = item.before.indexOf(id);
        dispatch({ type: "detail.open", id, parentId: item.before[index - 1], activate: false });
        void hydrateDetail(id);
      }
      if (item.activeId !== null) dispatch({ type: "detail.activate", id: item.activeId });
    }
    return true;
  }, [hydrateDetail, replaceExploreContextIds, restoreExploreContext, showToast, snapshotActiveExploreContext]);
  const randomOpenAvailable = useMemo(() => {
    if (ui.view === "explore") return true;
    if (ui.view === "auto-find") {
      return filteredPendingAutoFindGalleries.some((gallery) => !duplicateHiddenGalleryIds.has(gallery.id));
    }
    if (downloadsLoading || downloadsError !== null || !explorationExclusionsReady) return false;
    return downloadIds.some((id) => displayGalleries.get(id)?.download?.state === "completed"
      && !explorationExcludedGalleryIds.has(id)
      && !duplicateHiddenGalleryIds.has(id));
  }, [displayGalleries, downloadIds, downloadsError, downloadsLoading, duplicateHiddenGalleryIds, explorationExcludedGalleryIds, explorationExclusionsReady, filteredPendingAutoFindGalleries, ui.view]);
  const openRandomGallery = useCallback(async () => {
    if (randomOpenPending) return;

    if (ui.view === "downloads" && (downloadsLoading || downloadsError !== null || !explorationExclusionsReady)) {
      showToast(downloadsError
        ? "다운로드 목록을 완전히 불러오지 못해 랜덤 열기를 사용할 수 없습니다. 목록을 새로고침해 주세요."
        : "다운로드 및 제외 목록을 모두 확인한 뒤 랜덤 열기를 사용할 수 있습니다.");
      return;
    }

    let candidates: Gallery[] = [];
    if (ui.view === "auto-find") {
      candidates = filteredPendingAutoFindGalleries.filter(
        (gallery) => !duplicateHiddenGalleryIds.has(gallery.id),
      );
    } else if (ui.view === "downloads") {
      const excludedGalleryIds = new Set([
        ...explorationExcludedGalleryIds,
        ...duplicateHiddenGalleryIds,
      ]);
      const gallery = pickArtistBalancedCompletedDownload(
        downloadIds.flatMap((id) => {
          const candidate = displayGalleries.get(id);
          return candidate ? [candidate] : [];
        }),
        excludedGalleryIds,
      );
      candidates = gallery ? [gallery] : [];
    }

    if (ui.view !== "explore") {
      const gallery = ui.view === "downloads"
        ? candidates[0]
        : candidates[Math.floor(Math.random() * candidates.length)];
      if (!gallery) {
        showToast(ui.view === "auto-find"
          ? "현재 로드된 Auto Find 후보가 없습니다."
          : "다운로드가 완료된 앨범이 없습니다.");
        return;
      }
      openDetail(gallery.id);
      return;
    }

    setRandomOpenPending(true);
    try {
      const result = await backend.searchSubmit({
        text: "",
        includeTags: [],
        excludeTags: [],
        languages: ["korean", "japanese", "chinese", "english"],
        sort: "random",
        pageSize: 1,
      });
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      const summary = result.data.firstPage.items[0];
      if (!summary) {
        showToast("현재 검색 규칙에 맞는 랜덤 갤러리를 찾지 못했습니다.");
        return;
      }
      setGalleries((current) => mergeGalleryPage(current, result.data.firstPage).galleries);
      openDetail(summary.id);
    } catch {
      showToast("랜덤 갤러리를 불러오지 못했습니다.");
    } finally {
      setRandomOpenPending(false);
    }
  }, [displayGalleries, downloadIds, downloadsError, downloadsLoading, duplicateHiddenGalleryIds, explorationExcludedGalleryIds, explorationExclusionsReady, filteredPendingAutoFindGalleries, openDetail, randomOpenPending, showToast, ui.view]);
  const openRelatedDetail = useCallback((id: GalleryId, parentId: GalleryId, options?: { activate?: boolean }) => {
    if (!uiRef.current.detail.tabs.includes(id)) detailOrigins.current.set(id, { ...captureNavigationOrigin(), detailId: parentId });
    dispatch({ type: "detail.open", id, parentId, activate: options?.activate });
    void hydrateDetail(id);
  }, [hydrateDetail, captureNavigationOrigin]);
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
      if (result.data.state === "stale") setDownloadsRefresh((value) => value + 1);
    } catch {
      if (token === downloadOverlapReviewToken.current) {
        setDownloadOverlapError("다운로드 판본 검토 backend에 연결하지 못했습니다.");
      }
    } finally {
      if (token === downloadOverlapReviewToken.current) setDownloadOverlapLoading(false);
    }
  }, []);
  const openReview = useCallback((id: GalleryId) => {
    cancelAutomationReviewSequence();
    setContainmentKeeperId(null);
    setContainmentBatchGroup(undefined);
    setContainmentBatchProgress(undefined);
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
  }, [cancelAutomationReviewSequence, displayGalleries, duplicateSnapshot?.candidates, hydrateDownloadOverlapReview, hydrateDuplicateReview, hydrateDuplicateSnapshot, showToast]);
  const closeDuplicateReview = useCallback(() => {
    if (duplicateDecisionPendingRef.current) return;
    duplicateReviewToken.current += 1;
    setDuplicateReviewCandidateId(null);
    setDuplicateReview(null);
    setDuplicateReviewError(null);
    setDuplicateReviewLoading(false);
    dispatch({ type: "overlay.review", galleryId: null });
  }, []);
  const closeDownloadOverlapReview = useCallback(() => {
    if (downloadOverlapDecisionPendingRef.current
      || (downloadOverlapReviewId !== null
        && downloadOverlapAutomationHistoryPendingRef.current.has(downloadOverlapReviewId))) return;
    cancelAutomationReviewSequence();
    downloadOverlapReviewToken.current += 1;
    setDownloadOverlapReviewId(null);
    setDownloadOverlapReview(null);
    setDownloadOverlapError(null);
    setDownloadOverlapLoading(false);
    setContainmentBatchProgress(undefined);
    setContainmentKeeperId(null);
    setContainmentBatchGroup(undefined);
    dispatch({ type: "overlay.review", galleryId: null });
  }, [cancelAutomationReviewSequence, downloadOverlapReviewId]);
  const showAutomaticOverlapReview = useCallback((reviewId: string, id: GalleryId) => {
    setContainmentKeeperId(null);
    setContainmentBatchGroup(undefined);
    setContainmentBatchProgress(undefined);
    setDuplicateReviewCandidateId(null);
    setDuplicateReview(null);
    setDownloadOverlapReviewId(reviewId);
    setDownloadOverlapReview(null);
    setDownloadOverlapError(null);
    dispatch({ type: "overlay.review", galleryId: id });
    void hydrateDownloadOverlapReview(reviewId);
  }, [hydrateDownloadOverlapReview]);
  const navigateAutomationReviewSequence = useCallback((index: number) => {
    const session = automationReviewSessionRef.current;
    const current = session?.items[session.index];
    const next = session?.items[index];
    if (!session || !current || !next || index === session.index
      || downloadOverlapDecisionPendingRef.current
      || downloadOverlapAutomationHistoryPendingRef.current.has(current.reviewId)) return;
    const updated = { ...session, index };
    automationReviewSessionRef.current = updated;
    setAutomationReviewSession(updated);
    showAutomaticOverlapReview(next.reviewId, next.incomingGalleryId);
  }, [showAutomaticOverlapReview]);
  const completeAutomationReviewAction = useCallback((reviewId: string, reviewToken: number) => {
    if (reviewToken !== downloadOverlapReviewToken.current) return;
    const session = automationReviewSessionRef.current;
    if (!session) {
      closeDownloadOverlapReview();
      return;
    }
    if (session.items[session.index]?.reviewId !== reviewId) return;
    for (let offset = 1; offset <= session.items.length; offset += 1) {
      const index = (session.index + offset) % session.items.length;
      if (!session.items[index]!.acknowledgedAt) {
        navigateAutomationReviewSequence(index);
        return;
      }
    }
    closeDownloadOverlapReview();
    showToast(`자동분류 ${session.items.length}건의 순차 검토를 완료했습니다.`);
  }, [closeDownloadOverlapReview, navigateAutomationReviewSequence, showToast]);
  const downloadOverlapAutomationHistoryItem = downloadOverlapReview?.reviewId === downloadOverlapReviewId
    ? (automationReviewSession?.items ?? downloadOverlapAutomationHistory).find((item) => item.reviewId === downloadOverlapReviewId
      && item.incomingGalleryId === downloadOverlapReview.incoming.galleryId)
    : undefined;
  const acknowledgeDownloadOverlapAutomationReview = useCallback(async (reviewId: string) => {
    if (reviewId !== downloadOverlapAutomationHistoryItem?.reviewId) return;
    const reviewToken = downloadOverlapReviewToken.current;
    const acknowledged = await acknowledgeDownloadOverlapAutomationHistory(reviewId);
    if (acknowledged) completeAutomationReviewAction(reviewId, reviewToken);
  }, [acknowledgeDownloadOverlapAutomationHistory, completeAutomationReviewAction, downloadOverlapAutomationHistoryItem]);
  const restoreDownloadOverlapAutomationReview = useCallback(async (reviewId: string, galleryIds: GalleryId[]) => {
    if (reviewId !== downloadOverlapAutomationHistoryItem?.reviewId
      || galleryIds.some((id) => !downloadOverlapAutomationHistoryItem.removedGalleryIds.includes(id))) return;
    const reviewToken = downloadOverlapReviewToken.current;
    const restored = await restoreDownloadOverlapAutomationExclusions(reviewId, galleryIds);
    if (restored) completeAutomationReviewAction(reviewId, reviewToken);
  }, [completeAutomationReviewAction, downloadOverlapAutomationHistoryItem, restoreDownloadOverlapAutomationExclusions]);
  const applyDuplicateDecision = useCallback(async (request: DownloadOverlapDecisionRequest) => {
    if (duplicateDecisionPendingRef.current) return;
    duplicateDecisionPendingRef.current = true;
    setDuplicateDecisionPending(true);
    setDuplicateReviewError(null);
    try {
      const result = await backend.downloadOverlapDecisionApply(request);
      if (!result.ok) {
        if (result.error.code === "REVISION_CONFLICT" && request.candidateId) {
          await Promise.all([hydrateDuplicateReview(request.candidateId), hydrateDuplicateSnapshot()]);
          setDuplicateReviewError("다른 창에서 판정이 변경되어 최신 근거를 다시 불러왔습니다.");
        } else setDuplicateReviewError(result.error.message);
        return;
      }
      const hiddenId = request.action === "remove_incoming" ? result.data.review.incoming.galleryId
        : request.action === "remove_existing_continue" ? result.data.review.candidates[0]?.existing.galleryId : undefined;
      if (hiddenId !== undefined) {
        setDuplicateHiddenGalleryIds((known) => new Set([...known, hiddenId]));
        recordSessionDownloadActivity(hiddenId, "cancelled");
      }
      setDownloadsRefresh((value) => value + 1);
      setOverlapInventoryRefresh((value) => value + 1);
      await hydrateDuplicateSnapshot();
      duplicateDecisionPendingRef.current = false;
      closeDuplicateReview();
      showToast(hiddenId !== undefined ? `#${hiddenId} 제외 처리 · 파일은 영구 삭제하지 않습니다.` : "판정을 저장하고 두 완료 앨범을 보존했습니다. 다시 다운로드하지 않습니다.");
    } catch {
      setDuplicateReviewError("판정 결과를 확인하지 못했습니다. 다시 불러오기로 현재 상태를 확인해 주세요.");
    } finally {
      duplicateDecisionPendingRef.current = false;
      setDuplicateDecisionPending(false);
    }
  }, [closeDuplicateReview, hydrateDuplicateReview, hydrateDuplicateSnapshot, recordSessionDownloadActivity, showToast]);
  const applyCompletedPairMerge = useCallback(async (request: DownloadOverlapMergeRequest) => {
    if (duplicateDecisionPendingRef.current || !duplicateReview) return;
    const pair = duplicateReview.candidate;
    if (request.reviewId !== `duplicate:${pair.candidateId}` || request.candidateId !== pair.candidateId) return;
    const targetEntryId = request.sourceSide === "existing" ? pair.candidate.entryId : pair.parent.entryId;
    duplicateDecisionPendingRef.current = true;
    setDuplicateDecisionPending(true);
    setDuplicateReviewError(null);
    try {
      const result = await backend.downloadOverlapMerge({ ...request, excludeSource: true });
      if (!result.ok) {
        if (result.error.code === "REVISION_CONFLICT") await hydrateDuplicateReview(pair.candidateId);
        setDuplicateReviewError(result.error.message);
        return;
      }
      const merged = result.data;
      refreshMergedGallery(merged.targetGalleryId);
      thumbnailClient.invalidate((key) => key.kind === "overlap-review-page" ? merged.affectedReviewIds.includes(key.reviewId) : key.kind === "artifact-page"
        ? key.entryId === targetEntryId : key.galleryId === merged.targetGalleryId);
      if (merged.sourceExcluded) {
        setDuplicateHiddenGalleryIds((known) => new Set([...known, merged.sourceGalleryId]));
        recordSessionDownloadActivity(merged.sourceGalleryId, "cancelled");
      }
      recordSessionDownloadActivity(merged.targetGalleryId);
      setDownloadsRefresh((value) => value + 1);
      setOverlapInventoryRefresh((value) => value + 1);
      setOverlapInventory((reviews) => reviews.filter((review) => !merged.affectedReviewIds.includes(review.reviewId)));
      await hydrateDuplicateSnapshot();
      duplicateDecisionPendingRef.current = false;
      closeDuplicateReview();
      showToast(`${merged.replacedPages}장 교체${merged.addedPages ? ` · ${merged.addedPages}장 추가` : ""} · #${merged.targetGalleryId}에 병합 완료 · 원본 #${merged.sourceGalleryId} 제외 · 교체 전 파일은 백업에 보존했습니다.`);
    } catch {
      setDuplicateReviewError("병합 결과를 확인하지 못했습니다. 다시 불러오기로 현재 상태를 확인해 주세요. 복구용 원본은 보존됩니다.");
    } finally {
      duplicateDecisionPendingRef.current = false;
      setDuplicateDecisionPending(false);
    }
  }, [closeDuplicateReview, duplicateReview, hydrateDuplicateReview, hydrateDuplicateSnapshot, recordSessionDownloadActivity, refreshMergedGallery, showToast, thumbnailClient]);
  const applyDownloadOverlapMerge = useCallback(async (request: DownloadOverlapMergeRequest) => {
    if (downloadOverlapDecisionPendingRef.current
      || downloadOverlapAutomationHistoryPendingRef.current.has(request.reviewId)) return;
    if (automaticOverlapInFlightRef.current) { showToast("진행 중인 자동 판정이 끝난 뒤 다시 시도해 주세요."); return; }
    const candidate = downloadOverlapReview?.candidates.find((item) => item.candidateId === request.candidateId);
    if (!downloadOverlapReview || !candidate || downloadOverlapReview.reviewId !== request.reviewId) return;
    const targetEntryId = request.sourceSide === "existing" ? downloadOverlapReview.entryId : candidate.existing.entryId;
    downloadOverlapDecisionPendingRef.current = true;
    setDownloadOverlapDecisionPending(true);
    setDownloadOverlapError(null);
    try {
      const result = await backend.downloadOverlapMerge({ ...request, excludeSource: true });
      if (!result.ok) {
        if (result.error.code === "REVISION_CONFLICT") {
          await hydrateDownloadOverlapReview(request.reviewId);
          setDownloadOverlapError("검토가 변경되었습니다. 최신 대응 페이지를 확인하고 다시 선택해 주세요.");
        } else setDownloadOverlapError(result.error.message);
        return;
      }
      const merged = result.data;
      refreshMergedGallery(merged.targetGalleryId);
      thumbnailClient.invalidate((key) => key.kind === "overlap-review-page" ? merged.affectedReviewIds.includes(key.reviewId) : key.kind === "artifact-page"
        ? key.entryId === targetEntryId : key.galleryId === merged.targetGalleryId);
      if (merged.sourceExcluded) {
        setDuplicateHiddenGalleryIds((known) => new Set([...known, merged.sourceGalleryId]));
        recordSessionDownloadActivity(merged.sourceGalleryId, "cancelled");
      }
      recordSessionDownloadActivity(merged.targetGalleryId);
      setDownloadsRefresh((value) => value + 1);
      setOverlapInventoryRefresh((value) => value + 1);
      setOverlapInventory((reviews) => reviews.filter((review) => !merged.affectedReviewIds.includes(review.reviewId)));
      downloadOverlapDecisionPendingRef.current = false;
      closeDownloadOverlapReview();
      showToast(`${merged.replacedPages}장 교체${merged.addedPages ? ` · ${merged.addedPages}장 추가` : ""} · #${merged.targetGalleryId}에 병합 완료${merged.sourceExcluded ? ` · 원본 #${merged.sourceGalleryId} 제외` : ""} · 교체 전 파일은 백업에 보존했습니다.`);
    } catch {
      setDownloadOverlapError("병합 요청의 결과를 확인하지 못했습니다. 새로 고침으로 현재 상태를 확인해 주세요. 복구용 원본은 보존됩니다.");
    } finally {
      downloadOverlapDecisionPendingRef.current = false;
      setDownloadOverlapDecisionPending(false);
    }
  }, [closeDownloadOverlapReview, downloadOverlapReview, hydrateDownloadOverlapReview, recordSessionDownloadActivity, refreshMergedGallery, showToast, thumbnailClient]);
  const applyDownloadOverlapDecision = useCallback(async (request: DownloadOverlapDecisionRequest) => {
    if (downloadOverlapDecisionPendingRef.current
      || downloadOverlapAutomationHistoryPendingRef.current.has(request.reviewId)) return;
    if (automaticOverlapInFlightRef.current) { showToast("진행 중인 자동 판정이 끝난 뒤 다시 시도해 주세요."); return; }
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
        recordSessionDownloadActivity(
          excludedGalleryId,
          galleriesRef.current.get(excludedGalleryId)?.download?.state,
        );
      }
      setDownloadsRefresh((value) => value + 1);
      setOverlapInventoryRefresh((value) => value + 1);
      if (result.data.resumed || result.data.cancelled) {
        downloadOverlapDecisionPendingRef.current = false;
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
  }, [closeDownloadOverlapReview, hydrateDownloadOverlapReview, recordSessionDownloadActivity, showToast]);

  const applyContainmentBatch = useCallback(async (itemKeys: string[]) => {
    if (downloadOverlapDecisionPendingRef.current || !containmentGroup
      || (downloadOverlapReviewId !== null
        && downloadOverlapAutomationHistoryPendingRef.current.has(downloadOverlapReviewId))) return;
    if (automaticOverlapInFlightRef.current) { showToast("진행 중인 자동 판정이 끝난 뒤 다시 시도해 주세요."); return; }
    const requested = new Set(itemKeys);
    const items = containmentGroup.items.filter((item) => requested.has(item.key));
    if (!items.length) return;
    downloadOverlapDecisionPendingRef.current = true;
    setDownloadOverlapDecisionPending(true);
    setDownloadOverlapError(null);
    setContainmentBatchProgress({ completed: 0, total: items.length });
    setContainmentBatchGroup(containmentGroup);
    let completed = 0;
    let batchError: string | undefined;
    try {
      for (const original of items) {
        // Every decision is separately durable. Never reuse another review's
        // revision, or continue a batch after a failed/stale containment proof.
        const loaded = await backend.downloadOverlapReviewGet(original.review.reviewId);
        if (!loaded.ok) throw new Error(loaded.error.message);
        const fresh = buildDownloadOverlapContainmentGroup(containmentGroup.keeper.galleryId, [loaded.data])
          ?.items.find((item) => item.key === original.key && item.excluded.entryId === original.excluded.entryId);
        if (!fresh) throw new Error("포함관계 또는 처리 상태가 변경되었습니다. 최신 목록에서 다시 확인해 주세요.");
        const result = await backend.downloadOverlapDecisionApply({
          reviewId: fresh.review.reviewId, expectedRevision: fresh.review.revision,
          candidateId: fresh.candidate.candidateId, action: fresh.action, actor: "human",
          reasonCode: "manual_containment_batch_v1", ruleVersion: 1,
          featureSnapshotJson: JSON.stringify({
            rule: "manual_containment_batch_v1", ruleVersion: 1,
            reviewId: fresh.review.reviewId, reviewRevision: fresh.review.revision,
            candidateId: fresh.candidate.candidateId,
            keeperGalleryId: containmentGroup.keeper.galleryId, excludedGalleryId: fresh.excluded.galleryId,
          }),
        });
        if (!result.ok) throw new Error(result.error.message);
        completed += 1;
        setContainmentBatchProgress({ completed, total: items.length });
        setDuplicateHiddenGalleryIds((current) => new Set([...current, fresh.excluded.galleryId]));
        recordSessionDownloadActivity(fresh.excluded.galleryId, "cancelled");
        setOverlapInventory((current) => current.map((review) => review.reviewId === result.data.review.reviewId ? result.data.review : review));
        if (downloadOverlapReviewId === result.data.review.reviewId) setDownloadOverlapReview(result.data.review);
        setDownloadsRefresh((value) => value + 1);
      }
      showToast(`합본 #${containmentGroup.keeper.galleryId} 보존 · 포함 앨범 ${completed}개 제외 완료. 나머지 관계는 별도 검토로 유지합니다.`);
    } catch (error) {
      batchError = `${completed}/${items.length}개 처리 완료 · 나머지는 변경하지 않았습니다. ${error instanceof Error ? error.message : "일괄 처리를 완료하지 못했습니다."}`;
      setDownloadOverlapError(batchError);
      showToast(batchError);
    } finally {
      downloadOverlapDecisionPendingRef.current = false;
      setDownloadOverlapDecisionPending(false);
      setOverlapInventoryRefresh((value) => value + 1);
      if (downloadOverlapReviewId) await hydrateDownloadOverlapReview(downloadOverlapReviewId);
      if (batchError) setDownloadOverlapError(batchError);
    }
  }, [containmentGroup, downloadOverlapReviewId, hydrateDownloadOverlapReview, recordSessionDownloadActivity, showToast]);

  useEffect(() => {
    if (settingsLoading
      || settings.downloadOverlapAutoMode !== "strict_quarantine"
      || automationSequenceLoadingRef.current
      || downloadOverlapReviewId !== null
      || downloadOverlapDecisionPending
      || downloadOverlapDecisionPendingRef.current) return;
    if (automaticOverlapInFlightRef.current) {
      // Download pages and worker events can add another review while a sweep is
      // already running. Remember that change so it is not lost when the
      // current snapshot finishes.
      automaticOverlapRescanRequestedRef.current = true;
      return;
    }

    const reviewIds = [...new Set(allGalleries.flatMap((gallery) => {
      const download = gallery.download;
      return download?.state === "review_required"
        && download.reviewKind === "gallery_duplicate"
        && download.reviewId
        && download.reviewId !== downloadOverlapReviewId
        ? [download.reviewId]
        : [];
    }))];
    if (!reviewIds.length) return;

    automaticOverlapInFlightRef.current = true;
    void (async () => {
      let activityReview: DownloadOverlapReview | null = null;
      let systemicFailure = false;
      try {
        const loadedReviews: DownloadOverlapReview[] = [];
        for (const reviewId of reviewIds) {
          if (uiRef.current.overlays.reviewGalleryId !== null || downloadOverlapDecisionPendingRef.current) {
            automaticOverlapRescanRequestedRef.current = true;
            return;
          }
          const loaded = await backend.downloadOverlapReviewGet(reviewId);
          if (!loaded.ok) {
            if (loaded.error.code === "DOWNLOAD_OVERLAP_REVIEW_NOT_FOUND") continue;
            systemicFailure = true;
            showToast(`자동 분류 중단 · 직접 검토 필요 · ${loaded.error.message}`);
            return;
          }
          loadedReviews.push(loaded.data);
        }
        for (const loadedReview of prioritizeDownloadOverlapReviews(loadedReviews)) {
          if (uiRef.current.overlays.reviewGalleryId !== null || downloadOverlapDecisionPendingRef.current) {
            automaticOverlapRescanRequestedRef.current = true;
            return;
          }
          // Earlier decisions in this sweep may have removed a candidate from a
          // different review. Reconcile it before choosing a preservation witness.
          const refreshed = await backend.downloadOverlapReviewGet(loadedReview.reviewId);
          if (!refreshed.ok) {
            if (refreshed.error.code === "DOWNLOAD_OVERLAP_REVIEW_NOT_FOUND") continue;
            systemicFailure = true;
            showToast(`자동 분류 중단 · 직접 검토 필요 · ${refreshed.error.message}`);
            return;
          }
          let current = refreshed.data;
          activityReview = current;
          const attemptKey = `${current.reviewId}:${current.revision}`;
          if (automaticOverlapAttemptedRef.current.has(attemptKey)) continue;
          automaticOverlapAttemptedRef.current.add(attemptKey);
          const autoMerge = current.candidates.map((candidate) => automaticUncensoredMerge(current, candidate)).find(Boolean);
          if (autoMerge) {
            const result = await backend.downloadOverlapMerge(autoMerge);
            if (result.ok) {
              const merged = result.data;
              refreshMergedGallery(merged.targetGalleryId);
              setDuplicateHiddenGalleryIds((known) => new Set([...known, merged.sourceGalleryId]));
              thumbnailClient.invalidate((key) => key.kind === "overlap-review-page" ? merged.affectedReviewIds.includes(key.reviewId)
                : key.kind === "artifact-page" ? [current.entryId, ...current.candidates.map((c) => c.existing.entryId)].includes(key.entryId)
                  : key.galleryId === merged.targetGalleryId);
              recordSessionDownloadActivity(merged.sourceGalleryId, "cancelled");
              recordSessionDownloadActivity(merged.targetGalleryId);
              const detail = `무검열 ${merged.replacedPages}장 자동 병합 · #${merged.targetGalleryId} 보존 · #${merged.sourceGalleryId} 제외`;
              recordAutomaticOverlapActivity({ id: merged.mergeId, reviewId: current.reviewId, galleryId: merged.targetGalleryId,
                title: current.incoming.title, detail, occurredAt: Date.now(), state: "completed" });
              setDownloadsRefresh((value) => value + 1);
              setOverlapInventoryRefresh((value) => value + 1);
              automaticOverlapRescanRequestedRef.current = true;
              refreshDownloadOverlapAutomationHistory();
              showToast(detail);
            } else {
              // A language conflict, changed evidence, or file error leaves both
              // originals intact and the review pending; never fall back to removal.
              showToast(`자동 병합 보류 · 직접 검토 · ${result.error.message}`);
            }
            continue;
          }
          const plan = buildStrictOverlapPlan(current);
          if (!plan) continue;

          const excludedGalleryIds: GalleryId[] = [];
          let reviewBecameStale = false;
          for (const step of plan.steps) {
            if (uiRef.current.overlays.reviewGalleryId !== null || downloadOverlapDecisionPendingRef.current) {
              automaticOverlapAttemptedRef.current.delete(attemptKey);
              automaticOverlapRescanRequestedRef.current = true;
              return;
            }
            const featureSnapshotJson = JSON.stringify({
              ...(JSON.parse(step.featureSnapshotJson) as Record<string, unknown>),
              reviewRevision: current.revision,
            });
            const result = await backend.downloadOverlapDecisionApply({
              reviewId: current.reviewId,
              expectedRevision: current.revision,
              action: step.action,
              candidateId: step.candidateId,
              actor: "automation",
              reasonCode: DOWNLOAD_OVERLAP_AUTO_REASON_CODE,
              ruleVersion: DOWNLOAD_OVERLAP_AUTO_RULE_VERSION,
              featureSnapshotJson,
            });
            if (!result.ok) {
              if (["REVISION_CONFLICT", "DOWNLOAD_OVERLAP_REVIEW_NOT_FOUND"].includes(result.error.code)) {
                reviewBecameStale = true;
                automaticOverlapRescanRequestedRef.current = true;
                break;
              }
              const detail = `자동 분류 중단 · 직접 검토 필요 · ${result.error.message}`;
              recordAutomaticOverlapActivity({
                id: `${current.reviewId}:failed:${current.revision}`,
                reviewId: current.reviewId,
                galleryId: current.incoming.galleryId,
                title: current.incoming.title,
                detail,
                occurredAt: Date.now(),
                state: "failed",
              });
              refreshDownloadOverlapAutomationHistory();
              systemicFailure = true;
              showToast(detail);
              return;
            }
            const decidedCandidate = current.candidates.find((candidate) => candidate.candidateId === step.candidateId);
            const excludedGalleryId = step.action === "remove_incoming"
              ? current.incoming.galleryId
              : decidedCandidate?.existing.galleryId;
            if (excludedGalleryId !== undefined) {
              excludedGalleryIds.push(excludedGalleryId);
              // Each backend decision is durable on its own. Reflect it immediately
              // so a later candidate failure cannot leave this session's UI stale.
              setDuplicateHiddenGalleryIds((known) => new Set([...known, excludedGalleryId]));
            }
            current = result.data.review;
            activityReview = current;
          }

          if (excludedGalleryIds.length) {
            setDuplicateHiddenGalleryIds((known) => new Set([...known, ...excludedGalleryIds]));
          }
          if (reviewBecameStale) {
            if (excludedGalleryIds.length) refreshDownloadOverlapAutomationHistory();
            continue;
          }

          setDownloadsRefresh((value) => value + 1);
          setOverlapInventoryRefresh((value) => value + 1);
          const remaining = current.candidates.filter((candidate) => candidate.decision === undefined).length;
          const detail = plan.winner === "incoming"
            ? current.state === "pending"
              ? `자동 부분 정리 · 합본 보존 · 기존 판본 ${excludedGalleryIds.length}개 제외 · 나머지 ${remaining}개 직접 검토`
              : `자동 분류 완료 · 신규 앨범 B 보존 · 기존 판본 ${excludedGalleryIds.length}개 제외 처리`
            : "자동 분류 완료 · 기존 앨범 A 보존 · 신규 앨범 B 취소";
          recordAutomaticOverlapActivity({
            id: `${current.reviewId}:completed:${current.revision}`,
            reviewId: current.reviewId,
            galleryId: current.incoming.galleryId,
            title: current.incoming.title,
            detail,
            occurredAt: Date.now(),
            state: "completed",
          });
          refreshDownloadOverlapAutomationHistory();
          showToast(detail);
        }
      } catch {
        const detail = "자동 분류를 완료하지 못했습니다. 직접 검토해 주세요.";
        if (activityReview) {
          recordAutomaticOverlapActivity({
            id: `${activityReview.reviewId}:failed:exception`,
            reviewId: activityReview.reviewId,
            galleryId: activityReview.incoming.galleryId,
            title: activityReview.incoming.title,
            detail,
            occurredAt: Date.now(),
            state: "failed",
          });
          refreshDownloadOverlapAutomationHistory();
        }
        systemicFailure = true;
        showToast(detail);
      } finally {
        automaticOverlapInFlightRef.current = false;
        if (!systemicFailure && automaticOverlapRescanRequestedRef.current) {
          automaticOverlapRescanRequestedRef.current = false;
          setAutomaticOverlapSweepRevision((value) => value + 1);
        }
      }
    })();
  }, [allGalleries, automaticOverlapSweepRevision, automationSequenceLoading, downloadOverlapDecisionPending, downloadOverlapReviewId, recordAutomaticOverlapActivity, recordSessionDownloadActivity, refreshMergedGallery, refreshDownloadOverlapAutomationHistory, settings.downloadOverlapAutoMode, settingsLoading, showToast, thumbnailClient]);

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
  const loadMoreDownloadOverlapAutomationHistory = useCallback(() => {
    if (downloadOverlapAutomationHistoryLoading
      || downloadOverlapAutomationHistory.length >= downloadOverlapAutomationHistoryTotalItems) return;
    void hydrateDownloadOverlapAutomationHistoryPage(downloadOverlapAutomationHistoryPage + 1, false);
  }, [downloadOverlapAutomationHistory.length, downloadOverlapAutomationHistoryLoading, downloadOverlapAutomationHistoryPage, downloadOverlapAutomationHistoryTotalItems, hydrateDownloadOverlapAutomationHistoryPage]);
  const openActivity = useCallback(() => {
    activityOpener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setUnreadActivityCount(0);
    setUnreadAutomaticOverlapReviewIds(new Set());
    setActivityOpen(true);
    void hydrateDownloadOverlapAutomationHistoryPage(1, true);
  }, [hydrateDownloadOverlapAutomationHistoryPage, setActivityOpen]);
  const closeActivity = useCallback(() => {
    if (automationSequenceLoadingRef.current) cancelAutomationReviewSequence();
    setActivityOpen(false);
    const target = activityOpener.current;
    activityOpener.current = null;
    window.requestAnimationFrame(() => {
      if (target?.isConnected) target.focus();
      else document.querySelector<HTMLElement>("[aria-controls='activity-panel']")?.focus();
    });
  }, [cancelAutomationReviewSequence, setActivityOpen]);
  const openAutomaticOverlapReview = useCallback((reviewId: string, id: GalleryId) => {
    cancelAutomationReviewSequence();
    closeActivity();
    showAutomaticOverlapReview(reviewId, id);
  }, [cancelAutomationReviewSequence, closeActivity, showAutomaticOverlapReview]);
  const startAutomationReviewSequence = useCallback(async () => {
    if (automationSequenceLoadingRef.current || downloadOverlapDecisionPendingRef.current
      || downloadOverlapAutomationHistoryPendingRef.current.size > 0) return;
    if (automaticOverlapInFlightRef.current) {
      showToast("진행 중인 자동 판정이 끝난 뒤 순차 검토를 시작해 주세요.");
      return;
    }
    const token = ++automationSequenceToken.current;
    automationSequenceLoadingRef.current = true;
    setAutomationSequenceLoading(true);
    setDownloadOverlapAutomationHistoryError(null);
    try {
      const items = await collectUnacknowledgedAutomationHistory(
        (request) => backend.downloadOverlapAutomationHistoryList(request),
        () => token === automationSequenceToken.current,
      );
      if (items === null || token !== automationSequenceToken.current) return;
      if (items.length === 0) {
        showToast("순차 검토할 미확인 자동분류 기록이 없습니다.");
        void hydrateDownloadOverlapAutomationHistoryMeta();
        return;
      }
      automationSequenceLoadingRef.current = false;
      setAutomationSequenceLoading(false);
      closeActivity();
      const session = { items, index: 0 };
      automationReviewSessionRef.current = session;
      setAutomationReviewSession(session);
      showAutomaticOverlapReview(items[0]!.reviewId, items[0]!.incomingGalleryId);
    } catch (error) {
      if (token !== automationSequenceToken.current) return;
      const message = error instanceof Error ? error.message : "자동분류 순차 검토 목록을 불러오지 못했습니다.";
      setDownloadOverlapAutomationHistoryError(message);
      showToast(message);
    } finally {
      if (token === automationSequenceToken.current) {
        automationSequenceLoadingRef.current = false;
        setAutomationSequenceLoading(false);
      }
    }
  }, [closeActivity, hydrateDownloadOverlapAutomationHistoryMeta, showAutomaticOverlapReview, showToast]);
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

  const startFreshMetadataSearch = useCallback((value: string, options?: BackgroundOpenOptions) => {
    const target = metadataSearchToken(value);
    const kind = searchTokenKind(target.displayToken);
    const request: SearchRequest = target.includeTag
      ? {
        text: "",
        includeTags: [target.includeTag],
        excludeTags: [],
        languages: ui.search.explore.languages,
        sort: ui.exploreSort,
        pageSize: hitomiPageSize,
      }
      : {
        text: target.displayToken,
        includeTags: [],
        excludeTags: [],
        languages: ui.search.explore.languages,
        sort: ui.exploreSort,
        pageSize: hitomiPageSize,
      };
    if (!kind && !target.displayToken) return;

    if (startExploreSearch(request, { displayValue: target.displayToken, label: target.displayToken, background: options?.background }) && !options?.background) {
      dispatch({ type: "detail.minimize", minimized: true });
    }
  }, [
    startExploreSearch,
    hitomiPageSize,
    ui.exploreSort,
    ui.search.explore.languages,
  ]);

  const searchMetadata = startFreshMetadataSearch;

  const toggleMetadataFavorite = useCallback(async (value: string) => {
    const token = normalizeMetadataToken(value);
    if (!token) return;
    const key = favoriteKeyFromToken(token);
    const tutorialFollow = document.documentElement.dataset.tutorialOpen === "true" && document.documentElement.dataset.tutorialStep === "follow";
    const finishTutorialAction = beginTutorialAction("follow");
    if (tutorialFollow && pendingFavoriteTokens.current.has(token)) {
      finishTutorialAction("이전 즐겨찾기 저장을 처리 중입니다. 잠시 후 다시 시도해 주세요.");
      return;
    }
    const currentIntent = pendingFavoriteIntents.current.get(token) ?? favoriteMetadata.has(token);
    pendingFavoriteIntents.current.set(token, tutorialFollow || !currentIntent);
    if (pendingFavoriteTokens.current.has(token)) return;
    pendingFavoriteTokens.current.add(token);
    let persistedEnabled: boolean | undefined;
    try {
      while (true) {
        const enabled = pendingFavoriteIntents.current.get(token);
        if (enabled === undefined) break;
        const result = await backend.favoriteSet(key, enabled);
        if (!result.ok) {
          finishTutorialAction(result.error.message);
          showToast(result.error.message);
          return;
        }
        persistedEnabled = result.data.enabled;
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
        if (pendingFavoriteIntents.current.get(token) === result.data.enabled) break;
      }
      if (persistedEnabled !== undefined) {
        finishTutorialAction(persistedEnabled ? undefined : "작가·그룹을 즐겨찾기에 추가해 주세요.");
        showToast(`${value} 즐겨찾기를 ${persistedEnabled ? "추가" : "해제"}했습니다.`);
      }
    } catch {
      finishTutorialAction("즐겨찾기 변경을 저장하지 못했습니다.");
      showToast("즐겨찾기 변경을 저장하지 못했습니다.");
    } finally {
      pendingFavoriteIntents.current.delete(token);
      pendingFavoriteTokens.current.delete(token);
    }
  }, [favoriteMetadata, showToast]);

  const queueGalleries = useCallback(
    async (ids: GalleryId[]) => {
      const uniqueIds = [...new Set(ids)].filter((id) => !duplicateHiddenGalleryIds.has(id)
        && !pendingDownloadEntriesRef.current.has(galleriesRef.current.get(id)?.download?.entryId ?? ""));
      const newGalleryIds = uniqueIds.filter((id) => !galleries.get(id)?.download);
      const retryGalleryIds = uniqueIds.filter((id) => {
        const download = galleries.get(id)?.download;
        return download !== undefined && retryableDownloadStates.has(download.state);
      });
      const retryEntryIds = retryGalleryIds.map((id) => galleries.get(id)!.download!.entryId);
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
          retryGalleryIds.forEach((id) => recordSessionDownloadActivity(id, "queued"));
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
          queueResult.data.forEach((entry) => recordSessionDownloadActivity(entry.galleryId, entry.state));
          setDownloadIds((current) => [...new Set([...current, ...queueResult.data.map((entry) => entry.galleryId)])]);
          started += queueResult.data.length;
        }
        showToast(`${started}개 항목의 다운로드를 시작했습니다.`);
      } catch {
        showToast("다운로드 대기열에 연결하지 못했습니다.");
      }
      dispatch({ type: "selection.clear" });
    },
    [duplicateHiddenGalleryIds, galleries, recordSessionDownloadActivity, showToast],
  );

  const retryAvailableDownloads = useCallback(async (ids: GalleryId[]) => {
    if (bulkRetryBusy.current) return;
    const targets = [...new Set(ids)].flatMap((id) => {
      const entry = galleriesRef.current.get(id)?.download;
      return entry && retryableDownloadStates.has(entry.state)
        && !duplicateHiddenGalleryIds.has(id) && !pendingDownloadEntriesRef.current.has(entry.entryId)
        ? [{ id, entryId: entry.entryId }] : [];
    });
    if (!targets.length) {
      showToast("현재 상태에서 시작할 수 있는 항목이 없습니다.");
      return;
    }
    bulkRetryBusy.current = true;
    setBulkRetryPending(true);
    const batch = targets.slice(0, 200);
    batch.forEach(({ entryId }) => pendingDownloadEntriesRef.current.add(entryId));
    setPendingDownloadEntries(new Set(pendingDownloadEntriesRef.current));
    try {
      // The backend counts and fills available slots under the same work gate.
      // Passing every failed album in one ordinary request would exceed its 200-ID limit.
      const result = await backend.downloadRetry(batch.map(({ entryId }) => entryId), true);
      if (!result.ok) {
        showToast(result.error.message);
        return;
      }
      result.data.forEach((job, index) => {
        if (!job.reused) recordSessionDownloadActivity(batch[index]!.id, "queued");
      });
      const started = result.data.filter((job) => !job.reused).length;
      const remaining = targets.length - result.data.length;
      setDownloadsRefresh((value) => value + 1);
      showToast(!result.data.length
        ? "대기·진행 중인 다운로드가 이미 200개 이상입니다. 기존 작업은 그대로 유지합니다."
        : `${started}개 항목을 대기열에 추가했습니다.${remaining ? ` 나머지 ${remaining}개는 대기열이 줄어든 뒤 다시 요청해 주세요.` : ""}`);
    } catch {
      showToast("다운로드 재시도 요청을 전달하지 못했습니다.");
    } finally {
      batch.forEach(({ entryId }) => pendingDownloadEntriesRef.current.delete(entryId));
      setPendingDownloadEntries(new Set(pendingDownloadEntriesRef.current));
      bulkRetryBusy.current = false;
      setBulkRetryPending(false);
    }
  }, [duplicateHiddenGalleryIds, recordSessionDownloadActivity, showToast]);

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
        recordSessionDownloadActivity(id, "queued");
        setDownloadsRefresh((value) => value + 1);
        showToast("다운로드를 다시 시작했습니다.");
      } catch {
        showToast("재시도 요청을 backend에 전달하지 못했습니다.");
      } finally {
        finishDownloadMutation(download.entryId);
      }
    },
    [beginDownloadMutation, duplicateHiddenGalleryIds, finishDownloadMutation, recordSessionDownloadActivity, showToast],
  );

  const cancelGalleries = useCallback(async (ids: GalleryId[]) => {
    const entryIds = [...new Set(ids.flatMap((id) => {
      const download = galleriesRef.current.get(id)?.download;
      if (!download || !canCancelDownload(download.state)
        || pendingDownloadEntriesRef.current.has(download.entryId)
        || (duplicateHiddenGalleryIds.has(id) && !runningDownloadStates.has(download.state))) return [];
      return [download.entryId];
    }))];
    if (!entryIds.length) return;
    entryIds.forEach((id) => pendingDownloadEntriesRef.current.add(id));
    setPendingDownloadEntries(new Set(pendingDownloadEntriesRef.current));
    setCancellingDownloadEntries((current) => new Set([...current, ...entryIds]));
    try {
      const result = await cancelDownloads(backend, entryIds, (entries) => {
        entries.forEach((entry) => recordSessionDownloadActivity(entry.galleryId, entry.state));
        setGalleries((current) => mergeDownloadEntries(current, entries));
      });
      const failed = result.failed.reduce((sum, item) => sum + item.entryIds.length, 0);
      showToast([
        `${result.cancelled.length}개 다운로드를 취소했습니다.`,
        result.skipped.length ? `${result.skipped.length}개는 이미 완료되었거나 상태가 바뀌어 건너뛰었습니다.` : "",
        failed ? `${failed}개 취소 실패: ${result.failed[0]?.message}` : "",
      ].filter(Boolean).join(" "));
      if (result.skipped.length || failed) setDownloadsRefresh((value) => value + 1);
    } catch {
      showToast("취소 요청을 backend에 전달하지 못했습니다.");
    } finally {
      entryIds.forEach((id) => pendingDownloadEntriesRef.current.delete(id));
      setPendingDownloadEntries(new Set(pendingDownloadEntriesRef.current));
      setCancellingDownloadEntries((current) => {
        const next = new Set(current);
        entryIds.forEach((id) => next.delete(id));
        return next;
      });
    }
  }, [duplicateHiddenGalleryIds, recordSessionDownloadActivity, showToast]);

  const cancelQueueEntries = useCallback(async (ids: string[]): Promise<string> => {
    const targets = [...new Set(ids)].filter((id) => !pendingDownloadEntriesRef.current.has(id));
    targets.forEach((id) => pendingDownloadEntriesRef.current.add(id));
    setPendingDownloadEntries(new Set(pendingDownloadEntriesRef.current));
    setCancellingDownloadEntries((current) => new Set([...current, ...targets]));
    try {
      const result = await cancelDownloads(backend, targets, (entries) => {
        entries.forEach((entry) => recordSessionDownloadActivity(entry.galleryId, entry.state));
        setGalleries((current) => mergeDownloadEntries(current, entries));
      });
      const failures = result.failed.reduce((sum, group) => sum + group.entryIds.length, 0);
      return `취소 ${result.cancelled.length}개 · 완료/변경되어 건너뜀 ${result.skipped.length}개${failures ? ` · 실패 ${failures}개: ${result.failed[0]?.message}` : ""}`;
    } finally {
      targets.forEach((id) => pendingDownloadEntriesRef.current.delete(id));
      setPendingDownloadEntries(new Set(pendingDownloadEntriesRef.current));
      setCancellingDownloadEntries((current) => new Set([...current].filter((id) => !targets.includes(id))));
    }
  }, [recordSessionDownloadActivity]);

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
          at: performance.now(),
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
    const finishTutorialAction = beginTutorialAction("auto-find-refresh");
    setAutoFindPending(true);
    setAutoFindError(null);
    try {
      const result = await backend.autoFindRefresh();
      if (!result.ok) {
        finishTutorialAction(result.error.message);
        setAutoFindError(result.error.message);
        showToast(result.error.message);
        return;
      }
      setAutoFindSnapshot((current) => ({ ...current, run: result.data }));
      finishTutorialAction(result.data.state === "failed" || result.data.state === "cancelled"
        ? result.data.errorMessage || "탐색이 시작되지 않았습니다. 다시 갱신해 주세요."
        : undefined);
      await hydrateAutoFind();
    } catch {
      const message = "자동 탐색을 시작하지 못했습니다.";
      finishTutorialAction(message);
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
      ++explorationExclusionsHydrationToken.current;
      setExplorationExclusionsReady(false);
      setExplorationExcludedGalleryIds((current) => new Set([
        ...current,
        ...result.data.excludedGalleryIds,
      ]));
      void loadExplorationExclusionsAndSync().catch(() => undefined);
      dispatch({ type: "selection.clear" });
      setLastUndoAction({
        at: performance.now(),
        kind: "auto-find-exclusion",
        galleryIds: result.data.excludedGalleryIds,
      });
      showToast(`${result.data.excludedGalleryIds.length}개 후보를 다음 탐색에서도 제외합니다. Ctrl+Z로 실행 취소할 수 있습니다.`);
    } catch {
      showToast("자동 탐색 후보 제외 요청을 저장하지 못했습니다.");
    }
  }, [applyAutoFindSnapshot, autoFindIds, loadExplorationExclusionsAndSync, showToast]);

  const excludeExploreGalleries = useCallback(async (ids: GalleryId[]) => {
    if (exploreExclusionPendingRef.current) return;
    const targets = [...new Set(ids)].filter((id) => !explorationExcludedGalleryIds.has(id)
      && !duplicateHiddenGalleryIds.has(id));
    if (!targets.length) return;
    exploreExclusionPendingRef.current = true;
    const excluded: GalleryId[] = [];
    let failure: string | undefined;
    try {
      for (let offset = 0; offset < targets.length; offset += 200) {
        const result = await backend.autoFindExclude(targets.slice(offset, offset + 200), "사용자가 Explore 탐색에서 제외함");
        if (!result.ok) { failure = result.error.message; break; }
        excluded.push(...result.data.excludedGalleryIds);
        applyAutoFindSnapshot(result.data.snapshot);
        setExplorationExcludedGalleryIds((current) => new Set([...current, ...result.data.excludedGalleryIds]));
      }
    } catch {
      failure = "탐색 제외 요청을 저장하지 못했습니다.";
    } finally {
      if (excluded.length) {
        ++explorationExclusionsHydrationToken.current;
        setExplorationExclusionsReady(false);
        void loadExplorationExclusionsAndSync().catch(() => undefined);
        dispatch({ type: "selection.clear" });
        setLastUndoAction({ kind: "explore-exclusion", galleryIds: excluded, at: performance.now() });
      }
      exploreExclusionPendingRef.current = false;
      showToast(failure
        ? `${excluded.length}개 제외 완료 · 나머지는 처리하지 못했습니다. ${failure}`
        : `${excluded.length}개 앨범을 탐색에서 제외했습니다. 파일은 유지하며 Ctrl+Z로 되돌릴 수 있습니다.`);
    }
  }, [applyAutoFindSnapshot, duplicateHiddenGalleryIds, explorationExcludedGalleryIds, loadExplorationExclusionsAndSync, showToast]);

  const undoLastGalleryAction = useCallback(async () => {
    const action = lastUndoAction;
    if (!action) {
      showToast("실행 취소할 최근 제외 또는 격리 작업이 없습니다.");
      return;
    }
    if (undoPendingRef.current) return;
    undoPendingRef.current = true;
    try {
      if (action.kind !== "download-quarantine") {
        const restored = new Set<GalleryId>();
        for (let offset = 0; offset < action.galleryIds.length; offset += 200) {
          const result = await backend.explorationExclusionsRestore(action.galleryIds.slice(offset, offset + 200));
          if (!result.ok) {
            setLastUndoAction({ ...action, galleryIds: action.galleryIds.filter((id) => !restored.has(id)) });
            showToast(result.error.message);
            return;
          }
          applyAutoFindSnapshot(result.data.snapshot);
          ++explorationExclusionsHydrationToken.current;
          setExplorationExclusionsReady(false);
          result.data.restoredGalleryIds.forEach((id) => restored.add(id));
          setExplorationExcludedGalleryIds((current) => new Set([...current].filter((id) => !restored.has(id))));
          setDuplicateHiddenGalleryIds((current) => new Set([...current].filter((id) => !restored.has(id))));
          void loadExplorationExclusionsAndSync().catch(() => undefined);
        }
        showToast(`${restored.size}개 ${action.kind === "explore-exclusion" ? "탐색" : "Auto Find 후보"} 제외를 취소했습니다.`);
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
      showToast(action.kind !== "download-quarantine"
        ? "탐색 제외를 취소하지 못했습니다."
        : "격리 항목을 원래 위치로 복원하지 못했습니다.");
    } finally {
      undoPendingRef.current = false;
    }
  }, [applyAutoFindSnapshot, lastUndoAction, loadExplorationExclusionsAndSync, showToast]);

  const startDuplicateScan = useCallback(async (galleryIds?: GalleryId[]) => {
    if (duplicatePendingRef.current || duplicateRun?.state === "running") return;
    duplicatePendingRef.current = true;
    setDuplicatePending(true);
    setDuplicateError(null);
    try {
      const result = galleryIds ? await backend.duplicateScanStart(galleryIds) : await backend.duplicateScanStart();
      if (!result.ok) {
        setDuplicateError(result.error.message);
        showToast(result.error.message);
        return;
      }
      if (duplicateRunRef.current?.runId !== result.data.runId) duplicateHydrationToken.current += 1;
      duplicateRunRef.current = result.data;
      setDuplicateRun(result.data);
      setPairCompareRun(galleryIds ? { runId: result.data.runId, ids: galleryIds } : null);
      if (duplicateSnapshotRef.current) {
        const next = { ...duplicateSnapshotRef.current, run: result.data };
        duplicateSnapshotRef.current = next;
        setDuplicateSnapshot(next);
      }
      await hydrateDuplicateSnapshot();
      showToast(galleryIds ? `#${galleryIds[0]} ↔ #${galleryIds[1]} 두 앨범만 대조합니다. 자동 제외는 하지 않습니다.` : "검증된 로컬 아티팩트를 기준으로 작품 중복 검사를 시작했습니다.");
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
      || exploreContexts.current.get(context.id) !== context
      || context.query !== loading
    ) return;
    if (result.status === "failed") {
      const failed: GalleryQueryState = { ...loading, phase: "error", pendingPage: null, error: result.error };
      context.query = failed;
      publishExploreContext(context);
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
    publishExploreContext(context);
    setGalleries((current) => mergeGalleryPage(current, result.page).galleries);
    if (activeExploreContextIdRef.current !== contextId || uiRef.current.view !== "explore") return;
    exploreRestoreFrame.current = window.requestAnimationFrame(() => {
      if (navigationToken === exploreNavigationToken.current && galleryViewport.current) {
        galleryViewport.current.scrollTop = result.scrollTop;
      }
      exploreRestoreFrame.current = null;
    });
  }, [publishExploreContext]);

  const selectedIds = useMemo(() => [...ui.selection.ids], [ui.selection.ids]);
  const multiSelectionMode = ui.selection.ids.size >= 2;
  const selectedRunningIds = selectedIds.filter((id) => {
    const download = displayGalleries.get(id)?.download;
    return download && runningDownloadStates.has(download.state);
  });
  const selectionDownloadPending = selectedIds.some((id) => pendingDownloadEntries.has(displayGalleries.get(id)?.download?.entryId ?? ""));
  const selectionCancelPending = selectedIds.some((id) => cancellingDownloadEntries.has(displayGalleries.get(id)?.download?.entryId ?? ""));
  const selectedCompletedEntryIds = useMemo(() => [...new Set(selectedIds.flatMap((id) => {
    const download = displayGalleries.get(id)?.download;
    return download?.state === "completed" ? [download.entryId] : [];
  }))], [displayGalleries, selectedIds]);
  const selectedCanInternalScan = selectedIds.length > 0
    && selectedCompletedEntryIds.length === selectedIds.length;

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
  const autoFindFullGroups = useMemo(() => {
    if (ui.view !== "auto-find" || ui.grouping["auto-find"] === "all") return [];
    return groupAutoFindGalleries(
      visible,
      ui.grouping["auto-find"],
      (gallery) => autoFindDiscoveryDates.get(gallery.id) ?? gallery.publishedAt,
      favoriteMetadata,
      autoFindMatchedFavoriteTokens,
    );
  }, [autoFindDiscoveryDates, autoFindMatchedFavoriteTokens, favoriteMetadata, ui.grouping, ui.view, visible]);
  const autoFindPageContextKey = useMemo(() => [
    autoFindSnapshot.run?.runId ?? "",
    ui.search["auto-find"].committed.trim(),
    [...ui.search["auto-find"].languages].sort().join(","),
    ui.grouping["auto-find"],
    hitomiPageSize,
  ].join("\u001f"), [
    autoFindSnapshot.run?.runId,
    hitomiPageSize,
    ui.grouping,
    ui.search,
  ]);
  useEffect(() => {
    const previous = autoFindPageContext.current;
    autoFindPageContext.current = autoFindPageContextKey;
    if (previous === null || previous === autoFindPageContextKey) return;
    setAutoFindPage(1);
    setKeyboardFocusId(null);
    if (ui.view === "auto-find" && galleryViewport.current) galleryViewport.current.scrollTop = 0;
  }, [autoFindPageContextKey, ui.view]);
  const autoFindPaginationSource = useMemo(() => {
    if (ui.view !== "auto-find") return [];
    return ui.grouping["auto-find"] === "all"
      ? visible
      : uniqueAutoFindGalleries(autoFindFullGroups.flatMap((group) => group.items));
  }, [autoFindFullGroups, ui.grouping, ui.view, visible]);
  const autoFindPagination = useMemo(
    () => paginateAutoFindItems(autoFindPaginationSource, autoFindPage, hitomiPageSize),
    [autoFindPage, autoFindPaginationSource, hitomiPageSize],
  );
  const autoFindNextPageItems = useMemo(() => {
    if (autoFindPagination.page >= autoFindPagination.totalPages) return [];
    const start = autoFindPagination.startIndex + autoFindPagination.pageSize;
    return autoFindPaginationSource.slice(start, start + autoFindPagination.pageSize);
  }, [autoFindPagination, autoFindPaginationSource]);
  const downloadsPageContextKey = useMemo(() => [
    ui.search.downloads.committed.trim(),
    [...ui.search.downloads.languages].sort().join(","),
    ui.downloadsFilter,
    ui.downloadsSort ?? "recent",
    ui.grouping.downloads,
    hitomiPageSize,
  ].join("\u001f"), [hitomiPageSize, ui.downloadsFilter, ui.downloadsSort, ui.grouping.downloads, ui.search.downloads]);
  useEffect(() => {
    const previous = downloadsPageContext.current;
    downloadsPageContext.current = downloadsPageContextKey;
    if (previous === null || previous === downloadsPageContextKey) return;
    setDownloadsPage(1);
    setKeyboardFocusId(null);
    if (ui.view === "downloads" && galleryViewport.current) galleryViewport.current.scrollTop = 0;
  }, [downloadsPageContextKey, ui.view]);
  const downloadsPagination = useMemo(
    () => paginateGalleryItems(
      ui.view === "downloads" && ui.grouping.downloads === "all" ? visible : [],
      downloadsPage,
      hitomiPageSize,
    ),
    [downloadsPage, hitomiPageSize, ui.grouping.downloads, ui.view, visible],
  );
  const downloadsNextPageItems = useMemo(() => {
    if (downloadsPagination.page >= downloadsPagination.totalPages) return [];
    const start = downloadsPagination.startIndex + downloadsPagination.pageSize;
    return visible.slice(start, start + downloadsPagination.pageSize);
  }, [downloadsPagination, visible]);
  const autoFindCurrentCoverItems = useStableGalleryCoverPage(autoFindPagination.items);
  const autoFindNextCoverItems = useStableGalleryCoverPage(autoFindNextPageItems);
  const downloadsCurrentCoverItems = useStableGalleryCoverPage(downloadsPagination.items);
  const downloadsNextCoverItems = useStableGalleryCoverPage(downloadsNextPageItems);
  const renderedVisible = ui.view === "auto-find"
    ? autoFindPagination.items
    : ui.view === "downloads" && ui.grouping.downloads === "all"
      ? downloadsPagination.items
      : visible;

  useEffect(() => {
    if (ui.view === "auto-find") {
      sessionCoverRetainer.current?.visit("auto-find", autoFindCurrentCoverItems);
      return;
    }
    if (ui.view === "downloads" && ui.grouping.downloads === "all") {
      sessionCoverRetainer.current?.visit("downloads", downloadsCurrentCoverItems);
    }
  }, [autoFindCurrentCoverItems, downloadsCurrentCoverItems, ui.grouping.downloads, ui.view]);

  useEffect(() => {
    if (ui.view === "auto-find" && autoFindNextCoverItems.length > 0) {
      return prefetchNextGalleryPageAfterCurrent(
        thumbnailClient,
        "auto-find",
        autoFindCurrentCoverItems,
        autoFindNextCoverItems,
      );
    }
    if (
      ui.view === "downloads"
      && ui.grouping.downloads === "all"
      && downloadsNextCoverItems.length > 0
    ) {
      return prefetchNextGalleryPageAfterCurrent(
        thumbnailClient,
        "downloads",
        downloadsCurrentCoverItems,
        downloadsNextCoverItems,
      );
    }
    return undefined;
  }, [
    autoFindCurrentCoverItems,
    autoFindNextCoverItems,
    downloadsCurrentCoverItems,
    downloadsNextCoverItems,
    thumbnailClient,
    ui.grouping.downloads,
    ui.view,
  ]);
  const renderedActionableIds = useMemo(() => {
    if (ui.view === "explore") return actionableVisibleIds;
    return renderedVisible
      .filter((gallery) => gallery.download?.state !== "quarantined"
        && !duplicateHiddenGalleryIds.has(gallery.id))
      .map((gallery) => gallery.id);
  }, [actionableVisibleIds, duplicateHiddenGalleryIds, renderedVisible, ui.view]);
  visibleIdsRef.current = renderedActionableIds;

  useEffect(() => {
    if (ui.view !== "auto-find" || autoFindPage === autoFindPagination.page) return;
    setAutoFindPage(autoFindPagination.page);
    setKeyboardFocusId(null);
    if (galleryViewport.current) galleryViewport.current.scrollTop = 0;
  }, [autoFindPage, autoFindPagination.page, ui.view]);

  useEffect(() => {
    if (ui.view !== "downloads"
      || ui.grouping.downloads !== "all"
      || downloadsPage === downloadsPagination.page) return;
    setDownloadsPage(downloadsPagination.page);
    setKeyboardFocusId(null);
    if (galleryViewport.current) galleryViewport.current.scrollTop = 0;
  }, [downloadsPage, downloadsPagination.page, ui.grouping.downloads, ui.view]);

  const groupedVisible = useMemo(() => {
    if (ui.view !== "auto-find" && ui.view !== "downloads") return [];
    const grouping = ui.grouping[ui.view] as GalleryGrouping;
    if (grouping === "all") return [];
    if (ui.view === "auto-find") return groupAutoFindGalleries(renderedVisible, grouping,
      (gallery) => autoFindDiscoveryDates.get(gallery.id) ?? gallery.publishedAt,
      favoriteMetadata, autoFindMatchedFavoriteTokens);
    const groups = groupGalleries(renderedVisible, grouping, (gallery) => gallery.download?.createdAt ?? gallery.download?.updatedAt ?? gallery.publishedAt);
    if (ui.view === "downloads") {
      const order = new Map(renderedVisible.map((gallery, index) => [gallery.id, index]));
      groups.sort((a, b) => (order.get(a.items[0]!.id) ?? Infinity) - (order.get(b.items[0]!.id) ?? Infinity));
    }
    return groups;
  }, [autoFindDiscoveryDates, autoFindMatchedFavoriteTokens, favoriteMetadata, renderedVisible, ui.grouping, ui.view]);
  const artistDetailPreloadKey = active && ui.view === "downloads" && ui.grouping.downloads === "artist"
    ? JSON.stringify([...new Set(groupedVisible.flatMap((group) => group.items
      .filter((gallery) => gallery.tagsKnown === false).map((gallery) => gallery.id)))])
    : "";
  useEffect(() => {
    const ids = artistDetailPreloadKey ? JSON.parse(artistDetailPreloadKey) as GalleryId[] : [];
    artistDetailWanted.current = new Set(ids);
    if (!artistDetailPreloadKey) artistDetailAttempted.current.clear();
    artistDetailQueue.current = ids.filter((id) => !artistDetailAttempted.current.has(id)
      && !hydratedDetails.current.has(id) && !hydratedCardDetails.current.has(id));
    // Saved tags arrive with the list. Only missing tags need these workers;
    // visible cards still take priority within the shared six-worker budget.
    nearbyDetailPump.current();
  }, [artistDetailPreloadKey]);
  const groupedStorageKeys = useMemo(() => groupedVisible.map((group) => galleryGroupStorageKey(
    ui.view === "auto-find" ? "auto-find" : "downloads",
    group,
  )), [groupedVisible, ui.view]);
  const visibleCollapsedGroupKeys = useMemo<ReadonlySet<string>>(() => {
    if (ui.view !== "downloads") return collapsedGroupKeys;
    return new Set(groupedStorageKeys.filter((key) => !expandedDownloadGroupKeys.has(key)));
  }, [collapsedGroupKeys, expandedDownloadGroupKeys, groupedStorageKeys, ui.view]);
  const progressiveDownloadsObserveKey = useMemo(() => {
    if (ui.view !== "downloads") return "inactive";
    const collapsed = [...visibleCollapsedGroupKeys].sort().join("\u001f");
    return `${currentGalleryDisplayMode}\u001e${ui.grouping.downloads}\u001e${collapsed}\u001e${renderedVisible.map((gallery) => gallery.id).join(",")}`;
  }, [currentGalleryDisplayMode, renderedVisible, ui.grouping.downloads, ui.view, visibleCollapsedGroupKeys]);
  const nearbyDownloadIds = useProgressiveGalleryWindow({
    rootRef: galleryViewport,
    enabled: ui.view === "downloads",
    observeKey: progressiveDownloadsObserveKey,
    onEnter: hydrateNearbyDownloadDetails,
    onLeave: discardNearbyDownloadDetails,
    overscanPixels: currentGalleryDisplayMode === "compact" ? 900 : 1200,
    retainEntered: true,
  });
  const keyboardNavigableIds = useMemo(() => {
    if (ui.view === "explore" || ui.grouping[ui.view] === "all") return renderedActionableIds;
    const groupedView = ui.view;
    return [...new Set(groupedVisible.flatMap((group) => {
      const key = galleryGroupStorageKey(groupedView, group);
      return visibleCollapsedGroupKeys.has(key) ? [] : group.items.map((gallery) => gallery.id);
    }))];
  }, [groupedVisible, renderedActionableIds, ui.grouping, ui.view, visibleCollapsedGroupKeys]);
  const effectiveKeyboardFocusId = useMemo(() => {
    if (keyboardFocusId !== null && keyboardNavigableIds.includes(keyboardFocusId)) return keyboardFocusId;
    return selectedIds.find((id) => keyboardNavigableIds.includes(id))
      ?? keyboardNavigableIds.at(0)
      ?? null;
  }, [keyboardFocusId, keyboardNavigableIds, selectedIds]);
  const focusGalleryCard = useCallback((id: GalleryId) => {
    setKeyboardFocusId(id);
    window.requestAnimationFrame(() => {
      const slot = galleryViewport.current?.querySelector<HTMLElement>(`[data-progressive-gallery-id="${Number(id)}"]`);
      const card = slot?.querySelector<HTMLElement>(".gallery-card") ?? galleryViewport.current?.querySelector<HTMLElement>(`.gallery-card[data-gallery-id="${Number(id)}"]`);
      const focusTarget = card ?? slot;
      focusTarget?.focus({ preventScroll: true });
      focusTarget?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    });
  }, []);
  const changeAutoFindPage = useCallback((page: number) => {
    setAutoFindPage(page);
    setKeyboardFocusId(null);
    if (galleryViewport.current) galleryViewport.current.scrollTop = 0;
  }, []);
  const changeDownloadsPage = useCallback((page: number) => {
    setDownloadsPage(page);
    setKeyboardFocusId(null);
    if (galleryViewport.current) galleryViewport.current.scrollTop = 0;
  }, []);
  const refreshCurrentView = useCallback(() => {
    if (ui.view === "explore") {
      const activeId = activeExploreContextIdRef.current;
      const context = activeId ? exploreContexts.current.get(activeId) : undefined;
      if (!context?.request) {
        showToast("새로고침할 검색 결과가 없습니다. 먼저 검색해 주세요.");
        return;
      }
      startExploreSearch(context.request, { displayValue: context.displayValue, label: context.label, replace: true });
      return;
    }
    if (ui.view === "auto-find") {
      void hydrateAutoFind(true);
      return;
    }
    setDownloadsRefresh((current) => current + 1);
  }, [hydrateAutoFind, showToast, startExploreSearch, ui.view]);
  const allVisibleGroupsCollapsed = groupedStorageKeys.length > 0
    && groupedStorageKeys.every((key) => visibleCollapsedGroupKeys.has(key));
  const toggleGroupCollapsed = useCallback((key: string) => {
    if (ui.view === "downloads") {
      setExpandedDownloadGroupKeys((current) => {
        const next = new Set(current);
        if (next.has(key)) next.delete(key);
        else next.add(key);
        return next;
      });
      return;
    }
    const next = new Set(collapsedGroupKeys);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    persistCollapsedGroupKeys(next);
  }, [collapsedGroupKeys, persistCollapsedGroupKeys, ui.view]);
  const setAllVisibleGroupsCollapsed = useCallback((collapsed: boolean) => {
    if (ui.view === "downloads") {
      setExpandedDownloadGroupKeys(collapsed ? new Set() : new Set(groupedStorageKeys));
      return;
    }
    const next = new Set(collapsedGroupKeys);
    for (const key of groupedStorageKeys) {
      if (collapsed) next.add(key);
      else next.delete(key);
    }
    persistCollapsedGroupKeys(next);
  }, [collapsedGroupKeys, groupedStorageKeys, persistCollapsedGroupKeys, ui.view]);

  useEffect(() => {
    if (!active) return;
    const keyDown = (event: KeyboardEvent) => {
      if (document.documentElement.dataset.tutorialOpen === "true") return;
      const target = event.target instanceof HTMLElement
        ? event.target
        : document.activeElement instanceof HTMLElement
          ? document.activeElement
          : document.body;
      const primaryModifier = event.ctrlKey || event.metaKey;
      const modalOpen = Boolean(document.querySelector("dialog[open]"));
      const textEditing = Boolean(target.closest('input, textarea, select, [contenteditable="true"]'));

      if (personalLibraryOpen) {
        if (primaryModifier && event.key.toLowerCase() === "f" && !event.defaultPrevented && !modalOpen && !event.isComposing && !shell.settingsOpen && !shell.activityOpen) {
          event.preventDefault(); document.querySelector<HTMLInputElement>('#personal-library-search input')?.focus();
        }
        if (event.key === "Escape" && !event.defaultPrevented && !event.repeat && !modalOpen && !event.isComposing && !shell.settingsOpen) {
          if (shell.activityOpen) { event.preventDefault(); closeActivity(); }
          else if (ui.detail.activeId !== null && !ui.detail.minimized) { event.preventDefault(); closeDetail(ui.detail.activeId); }
        }
        return;
      }

      if (event.key === "Escape") {
        if (event.defaultPrevented || event.repeat || event.isComposing || modalOpen) return;
        if (shell.activityOpen) {
          event.preventDefault();
          closeActivity();
          return;
        }
        if (shell.settingsOpen || ui.overlays.reviewGalleryId !== null || shell.exitConfirmOpen) return;
        if (ui.search[ui.view].suggestionsOpen) {
          dispatch({ type: "search.suggestions", view: ui.view, open: false });
          return;
        }
        if (ui.detail.activeId !== null && !ui.detail.minimized) closeDetail(ui.detail.activeId);
        else if (ui.view === "explore" && activeExploreContextIdRef.current) closeExploreContext(activeExploreContextIdRef.current);
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
        markUi("f5_results");
        event.preventDefault();
        refreshCurrentView();
        return;
      }

      if (textEditing) return;

      if (primaryModifier && event.shiftKey && event.key.toLocaleLowerCase() === "t" && !event.altKey && !event.repeat) {
        event.preventDefault();
        reopenClosedNavigation();
        return;
      }

      if ((event.key === "?" || event.key === "/" || event.code === "Slash") && !primaryModifier && !event.altKey && !event.repeat) {
        event.preventDefault();
        setKeyboardShortcutsOpen(true);
        return;
      }

      if (primaryModifier && event.key.toLocaleLowerCase() === "z" && !event.shiftKey && !event.altKey && !event.repeat) {
        event.preventDefault();
        const closed = closedNavigation.current.at(-1);
        if (closed && (!lastUndoAction || closed.at > lastUndoAction.at)) reopenClosedNavigation();
        else void undoLastGalleryAction();
        return;
      }

      const galleryContext = Boolean(target.closest(".gallery-viewport, .selection-toolbar"));
      if (!galleryContext) return;

      if (primaryModifier && event.key.toLocaleLowerCase() === "a" && !event.altKey && !event.repeat) {
        event.preventDefault();
        if (event.shiftKey) dispatch({ type: "selection.clear" });
        else dispatch({ type: "selection.all", ids: renderedActionableIds });
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
        else void excludeExploreGalleries(actionIds);
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
  }, [personalLibraryOpen, closeActivity, active, effectiveKeyboardFocusId, excludeAutoFindCandidates, excludeExploreGalleries, focusGalleryCard, galleryColumns, keyboardNavigableIds, navigateView, openExitConfirm, quarantineGalleries, queueGalleries, refreshCurrentView, renderedActionableIds, selectedIds, showToast, shell.activityOpen, shell.settingsOpen, shell.exitConfirmOpen, ui.detail.activeId, ui.detail.minimized, ui.overlays, ui.search, ui.selection.anchorId, ui.view, undoLastGalleryAction, closeDetail, closeExploreContext, reopenClosedNavigation, lastUndoAction]);

  const config = viewConfig[ui.view];
  const currentAutoFindStatus = autoFindStatusLabel(
    autoFindLoading,
    autoFindError,
    autoFindCount,
    autoFindSnapshot.run,
  );
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
      busy: contextQuery.phase === "submitting" || contextQuery.phase === "loading-page",
    }];
  }), [activeExploreContextId, exploreContextIds, query]);
  // Switching presentation does not dispose the controller or its caches/jobs.
  useEffect(() => {
    dispatch({ type: "selection.clear" });
  }, [active]);
  const queueGalleryFromMenu = useCallback((id: GalleryId) => { void queueGalleries([id]); }, [queueGalleries]);
  const excludeGalleryFromMenu = useCallback((id: GalleryId) => {
    if (ui.view === "downloads") void quarantineGalleries([id]);
    else if (ui.view === "auto-find") void excludeAutoFindCandidates([id]);
    else void excludeExploreGalleries([id]);
  }, [ui.view, quarantineGalleries, excludeAutoFindCandidates, excludeExploreGalleries]);
  const inspectExclusion = useCallback((_id: GalleryId, context: ExplorationExclusionContext) => {
    setExclusionContext(context);
  }, []);
  const restoreExcludedGallery = useCallback(async (id: GalleryId): Promise<boolean> => {
    if (restoringExclusionIds.current.has(id)) return false;
    restoringExclusionIds.current.add(id);
    setRestoringExclusions(new Set(restoringExclusionIds.current));
    let filesRestored = false;
    try {
      // Refresh the entry identity at action time; the menu may have been open
      // while a background download or quarantine changed the album.
      const context = await backend.explorationExclusionContext(id);
      if (!context.ok) { showToast(context.error.message); return false; }
      if (context.data.quarantined) {
        if (!context.data.quarantineEntryId) {
          showToast("격리된 파일의 위치를 확인할 수 없어 복원을 중단했습니다.");
          return false;
        }
        const restored = await backend.downloadQuarantineUndo([context.data.quarantineEntryId]);
        if (!restored.ok) { showToast(restored.error.message); return false; }
        setGalleries((current) => mergeDownloadEntries(current, restored.data));
        filesRestored = true;
      }
      const result = await restoreExplorationExclusionsAndSync([id]);
      if (!result.ok) {
        showToast(`${filesRestored ? "파일은 복원했지만 목록 제외 해제에 실패했습니다. " : ""}${result.error.message}`);
        return false;
      }
      applyAutoFindSnapshot(result.data.snapshot);
      setDownloadsRefresh((value) => value + 1);
      showToast(filesRestored ? "격리 파일과 목록 표시를 복원했습니다." : "목록 제외를 해제했습니다. 새 다운로드는 시작하지 않습니다.");
      return true;
    } catch {
      showToast(filesRestored ? "파일은 복원했지만 목록 제외 해제를 확인하지 못했습니다. 다시 시도해 주세요." : "앨범을 복원하지 못했습니다. 다시 시도해 주세요.");
      return false;
    } finally {
      restoringExclusionIds.current.delete(id);
      setRestoringExclusions(new Set(restoringExclusionIds.current));
    }
  }, [applyAutoFindSnapshot, restoreExplorationExclusionsAndSync, showToast]);
  const restoreExclusionFromMenu = useCallback((id: GalleryId) => { void restoreExcludedGallery(id); }, [restoreExcludedGallery]);
  const reviewExclusion = () => {
    if (!exclusionContext) return;
    setExclusionContext(null);
    if (exclusionContext.reviewId) {
      openAutomaticOverlapReview(exclusionContext.reviewId, exclusionContext.reviewGalleryId ?? exclusionContext.galleryId);
    } else if (exclusionContext.legacyCandidateId) {
      cancelAutomationReviewSequence();
      setDownloadOverlapReviewId(null);
      setDownloadOverlapReview(null);
      setDuplicateReviewCandidateId(exclusionContext.legacyCandidateId);
      setDuplicateReview(null);
      setDuplicateReviewError(null);
      dispatch({ type: "overlay.review", galleryId: exclusionContext.galleryId });
      void hydrateDuplicateReview(exclusionContext.legacyCandidateId);
    }
  };
  const renderGalleryGrid = (items: Gallery[], ariaLabel: string) => (
    <GalleryGrid
      columns={galleryColumns}
      previewWidth={previewWidth}
      selectionContext={multiSelectionMode}
      ariaLabel={ariaLabel}
      displayMode={currentGalleryDisplayMode}
    >
      {items.map((gallery, index) => {
        const card = <GalleryCard
          key={gallery.id}
          gallery={gallery}
          thumbnailPriority={index < galleryColumns ? "visible" : "prefetch"}
          view={ui.view}
          displayMode={currentGalleryDisplayMode}
          explorationExcluded={duplicateHiddenGalleryIds.has(gallery.id) || explorationExcludedGalleryIds.has(gallery.id)}
          explorationExcludedLabel={duplicateHiddenGalleryIds.has(gallery.id) ? "중복 판정으로 제외" : "탐색에서 제외"}
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
            && !(ui.view === "explore" && explorationExcludedGalleryIds.has(gallery.id))
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
          pendingAction={restoringExclusions.has(gallery.id) || Boolean(gallery.download && pendingDownloadEntries.has(gallery.download.entryId))}
          onQueue={queueGalleryFromMenu}
          onExclude={excludeGalleryFromMenu}
          onInspectExclusion={inspectExclusion}
          onRestoreExclusion={restoreExclusionFromMenu}
        />;
        return ui.view === "downloads" ? (
          <ProgressiveGallerySlot
            key={gallery.id}
            gallery={gallery}
            displayMode={currentGalleryDisplayMode}
            active={nearbyDownloadIds.has(gallery.id) || gallery.id === effectiveKeyboardFocusId}
          >
            {card}
          </ProgressiveGallerySlot>
        ) : card;
      })}
    </GalleryGrid>
  );
  const persistedAutomaticReviewIds = new Set(
    downloadOverlapAutomationHistory.map((item) => item.reviewId),
  );
  const unreadAutomaticOverlapSessionCount = [...unreadAutomaticOverlapReviewIds]
    .filter((reviewId) => !persistedAutomaticReviewIds.has(reviewId)).length;
  const activityBadgeCount = unreadActivityCount
    + downloadOverlapAutomationHistoryUnacknowledgedItems
    + unreadAutomaticOverlapSessionCount;

  return (
    <DownloadProgressContext.Provider value={downloadProgress}>
      {children({
        workspace: active ? (
        <>
      <div className={`app-shell${shell.railCollapsed ? " sidebar-collapsed" : ""}`}>
        <SideRail
          view={ui.view}
          collapsed={shell.railCollapsed}
          autoFindCount={autoFindCount}
          attentionCount={attentionCount}
          source="hitomi"
          privacyMode={shell.privacyMode}
          privacyModePending={privacyModePending || settingsLoading}
          onPrivacyModeToggle={() => void togglePrivacyMode()}
          personalLibraryOpen={personalLibraryOpen}
          onOpenPersonalLibrary={() => { setPersonalLibraryVisited(true); setPersonalLibraryOpen(true); dispatch({ type: "detail.minimize", minimized: true }); }}
          onSettings={() => setSettingsOpen(true)}
          onNavigate={navigateView}
          onSourceChange={selectSource}
          onToggle={() => toggleRail()}
        />
        {personalLibraryVisited ? <div className="personal-library-host" hidden={!personalLibraryOpen}>
          <PersonalLibraryWorkspace previewWidth={previewWidth} pageSize={hitomiPageSize} privacyMode={shell.privacyMode}
            activityOpen={shell.activityOpen}
            onActivity={() => shell.activityOpen ? closeActivity() : openActivity()}
            onQueue={(id) => void queueGalleries([id])} onExclude={(id) => excludeExploreGalleries([id])} onOpenFolder={openDownloadFolder}
            queueProgress={workQueue.snapshot && !workQueue.error ? queueProgress(workQueue.snapshot).percent : undefined} queueActiveCount={workQueue.snapshot?.globalActive}
            onOpen={openSavedItem} onBack={() => setPersonalLibraryOpen(false)} />
        </div> : null}
        <main className="workspace" style={personalLibraryOpen ? { display: "none" } : undefined} onKeyDownCapture={(event) => {
          const target = event.target as HTMLElement;
          if (event.key !== "Enter" || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey
            || event.repeat || event.nativeEvent.isComposing || event.defaultPrevented || !multiSelectionMode
            || target.closest("button, input, textarea, select, [contenteditable='true']")
            || !target.closest(".gallery-viewport, .selection-toolbar")) return;
          event.preventDefault();
          event.stopPropagation();
          openSelectedDetails(selectedIds);
        }}>
          <ViewHeader
            queueProgress={workQueue.snapshot && !workQueue.error ? queueProgress(workQueue.snapshot).percent : undefined}
            queueActiveCount={workQueue.snapshot?.globalActive}
            view={ui.view}
            search={ui.search[ui.view]}
            searchPending={settingsLoading}
            suggestions={ui.view === "explore" ? searchSuggestions : []}
            historyPending={historyPending}
            onRemoveHistory={(historyId) => void removeSearchHistory(historyId)}
            onClearHistory={() => void removeSearchHistory()}
            searchEndControl={ui.view === "explore" ? <div className="search-sort-control" title="검색할 정렬 방식입니다. 검색을 실행하면 적용됩니다.">
              <label className="sr-only" htmlFor="sort-select">검색 정렬</label>
              <select id="sort-select" aria-description="검색 실행 시 적용" value={ui.exploreSort} disabled={settingsLoading}
                onFocus={() => dispatch({ type: "search.suggestions", view: "explore", open: false, active: null })}
                onChange={(event) => dispatch({ type: "sort.set", sort: event.target.value as SearchSort })}>{sortOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}</select>
              <FluentIcon glyph="\uE70D" />
            </div> : null}
            activityCount={activityBadgeCount}
            activityOpen={shell.activityOpen}
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
                  pageSize: hitomiPageSize,
                }, { displayValue, label: displayValue || "새 탐색" });
                return;
              }
              dispatch({ type: "search.commit", view: ui.view, value });
              showToast("현재 결과를 필터했습니다.");
            }}
            onSelectSuggestion={(suggestion, value, options) => {
              if (ui.view === "explore" && suggestion.request) {
                startExploreSearch({
                  ...suggestion.request,
                  pageSize: hitomiPageSize,
                }, { displayValue: value, label: value || "새 탐색", background: options?.background });
                return;
              } else if (ui.view === "explore") {
                startExploreSearch({
                  text: value.trim(),
                  includeTags: [],
                  excludeTags: [],
                  languages: [...ui.search.explore.languages],
                  sort: ui.exploreSort,
                  pageSize: hitomiPageSize,
                }, { displayValue: value, label: value || "새 탐색", background: options?.background });
                return;
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
            tagCatalogRevision={tagCatalogStatus?.revision}
            onRandomOpen={() => void openRandomGallery()}
            randomOpenPending={randomOpenPending}
            randomOpenAvailable={randomOpenAvailable}
            onActivity={() => shell.activityOpen ? closeActivity() : openActivity()}
          />
          <section className="page-heading">
            <div><span className="eyebrow">{config.eyebrow}</span><h1>{config.title}</h1></div>
            <div className="heading-actions">
              {ui.view === "auto-find" ? (
                <>
                  <button type="button" data-tour="hitomi-auto-find-refresh" data-running={autoFindSnapshot.run?.state === "running"} className="text-button" aria-busy={autoFindPending || autoFindSnapshot.run?.state === "running"} disabled={autoFindPending || autoFindSnapshot.run?.state === "running"} onClick={() => void refreshAutoFind()}>
                    {autoFindPending || autoFindSnapshot.run?.state === "running" ? <><span className="spinner catalog-refresh-spinner" aria-hidden="true" /> 즐겨찾기 작가·그룹 탐색 중…</> : <><FluentIcon glyph="\uE72C" /> {autoFindSnapshot.run?.state === "failed" ? "다시 탐색" : "즐겨찾기 작가·그룹 갱신"}</>}
                  </button>
                  {autoFindSnapshot.run?.state === "running" ? <button type="button" className="text-button danger-button" disabled={autoFindPending} onClick={() => void cancelAutoFind()}><FluentIcon glyph="\uE711" /> 탐색 취소</button> : null}
                </>
              ) : ui.view === "downloads" ? (
                <>
                  <p className="sr-only" id="duplicate-scan-explanation">작품 간 검사는 작가가 같은 서로 다른 앨범끼리 비교하고, 내부 페이지 검사는 각 앨범 안에서 반복되거나 유사한 페이지를 찾습니다.</p>
                  <button type="button" className="text-button" aria-busy={reconcilingArtifacts} disabled={reconcilingArtifacts} onClick={() => void reconcileArtifacts()}><FluentIcon glyph="\uE9D9" /> 무결성 검사</button>
                  <button type="button" className="text-button" aria-describedby="duplicate-scan-explanation" title="완료된 앨범 중 작가 정보가 하나라도 같은 작품끼리만 비교합니다." disabled={duplicateLoading || duplicatePending || duplicateRun?.state === "running"} onClick={() => void startDuplicateScan()}><FluentIcon glyph="\uE9D9" /> 작가 내 검사</button>
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
                  ><FluentIcon glyph="\uE9D9" /> 내부 중복 검사</button>
                  <button type="button" className="text-button primary" aria-busy={bulkRetryPending} disabled={bulkRetryPending} title="현재 필터의 미완료 항목을 대기·진행 합계 200개까지 추가합니다. 기존 작업은 취소하지 않습니다." onClick={() => void retryAvailableDownloads(actionableVisibleIds)}><FluentIcon glyph="\uE896" /> 전체 다운로드</button>
                </>
              ) : null}
            </div>
          </section>
          {ui.view === "downloads" && pairCompareOpen ? (
            <section className="pair-compare-panel" aria-label="두 앨범 직접 대조">
              <form onSubmit={(event) => {
                event.preventDefault();
                const ids = pairCompareIds.map((value) => Number(value.trim()));
                if (ids.some((id) => !Number.isSafeInteger(id) || id <= 0) || ids[0] === ids[1]) {
                  showToast("서로 다른 앨범 ID 두 개를 입력하세요."); return;
                }
                void startDuplicateScan(ids.map(galleryId));
              }}>
                <label>앨범 A ID<input inputMode="numeric" value={pairCompareIds[0]} onChange={(event) => setPairCompareIds([event.target.value, pairCompareIds[1]])} /></label>
                <span aria-hidden="true">↔</span>
                <label>앨범 B ID<input inputMode="numeric" value={pairCompareIds[1]} onChange={(event) => setPairCompareIds([pairCompareIds[0], event.target.value])} /></label>
                <button type="button" className="text-button" disabled={selectedIds.length !== 2} onClick={() => setPairCompareIds([String(selectedIds[0]), String(selectedIds[1])])}>선택한 두 앨범 가져오기</button>
                <button type="submit" className="text-button primary" disabled={duplicatePending || duplicateRun?.state === "running"}>이 두 앨범만 검사</button>
                <button type="button" className="text-button" onClick={() => setPairCompareOpen(false)}>닫기</button>
              </form>
              <p>다운로드 완료된 두 앨범만 대조합니다. 작가가 달라도 가능하며, 기존 해시를 재사용합니다. 자동 제외하지 않습니다.</p>
              {pairCompareRun && duplicateRun?.runId === pairCompareRun.runId ? (
                <div role="status">
                  #{pairCompareRun.ids[0]} ↔ #{pairCompareRun.ids[1]} · {duplicateRun.state === "running" ? `해시 준비 ${duplicateRun.hashedArtifacts}/2 · 대조 ${duplicateRun.comparedPairs}/1` : duplicateRun.state === "completed" ? "대조 완료" : duplicateRun.state === "cancelled" ? "대조 취소됨" : "대조 실패"}
                  {duplicateRun.state === "completed" ? (() => {
                    const candidate = duplicateSnapshot?.candidates.find((item) => pairCompareRun.ids.includes(item.parent.galleryId) && pairCompareRun.ids.includes(item.candidate.galleryId));
                    return candidate ? <button type="button" className="text-button" onClick={() => {
                      setDownloadOverlapReviewId(null);
                      setDuplicateReviewCandidateId(candidate.candidateId);
                      setDuplicateReview(null);
                      dispatch({ type: "overlay.review", galleryId: candidate.parent.galleryId });
                      void hydrateDuplicateReview(candidate.candidateId);
                    }}>이 두 앨범의 근거 검토</button> : <span> · 새 검토 후보 없음: 일치 기준 미달 또는 기존 최종 판정 유지</span>;
                  })() : null}
                </div>
              ) : null}
            </section>
          ) : null}
          {ui.view === "explore" && activeExploreContextId ? (
            <ExploreContextBar
              tabs={exploreContextTabs}
              activeId={activeExploreContextId}
              onActivate={activateExploreContext}
              onClose={closeExploreContext}
            />
          ) : null}
          <section className="context-row">
            <div className="context-left">
              <GalleryDisplayModeControl
                value={currentGalleryDisplayMode}
                onChange={(mode) => persistGalleryDisplayMode(ui.view, mode)}
              />
              {(ui.view === "auto-find" || ui.view === "downloads") ? (
                <div className="gallery-grouping-toolbar" role="group" aria-label="목록 표시 도구">
                  <GroupingControl
                    includeDays={ui.view === "auto-find"}
                    includeGroups={ui.view === "auto-find"}
                    value={ui.grouping[ui.view]}
                    onChange={(grouping) => persistGalleryGrouping(
                      ui.view === "auto-find" ? "auto-find" : "downloads",
                      grouping,
                    )}
                  />
                  {ui.view === "downloads" ? <div className="select-control downloads-sort-control" title={popularity.message || "기간별 전체 순위 · 순위 미확인은 뒤에 표시"}>
                    <label htmlFor="downloads-sort-select">정렬</label>
                    <select id="downloads-sort-select" value={ui.downloadsSort ?? "recent"} onChange={(event) => dispatch({ type: "downloads.sort", sort: event.target.value as DownloadSort })}>
                      <option value="recent">최신순</option><option value="popular_today">일간 인기순</option><option value="popular_week">주간 인기순</option><option value="popular_month">월간 인기순</option><option value="popular_year">연간 인기순</option>
                    </select>
                  </div> : null}
                  <button
                    type="button"
                    className="text-button dark gallery-groups-toggle-all"
                    disabled={ui.grouping[ui.view] === "all" || !groupedVisible.length}
                    title={ui.grouping[ui.view] === "all" ? (ui.view === "downloads" ? "작가별에서 사용할 수 있습니다." : "기간별 또는 작가별에서 사용할 수 있습니다.") : undefined}
                    onClick={() => setAllVisibleGroupsCollapsed(!allVisibleGroupsCollapsed)}
                  ><FluentIcon glyph="\uE70D" /> {allVisibleGroupsCollapsed ? "전부 펼치기" : "전부 접기"}</button>
                </div>
              ) : null}
              {ui.view !== "auto-find" ? (
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
              ) : null}
            </div>
            {ui.view === "auto-find" ? <ResultDetails key="auto-find" count={visible.length}
              busy={autoFindLoading || autoFindSnapshot.run?.state === "running"} error={!!autoFindError || autoFindSnapshot.run?.state === "failed"}>
              <p role="status">{currentAutoFindStatus}</p>
              {autoFindSnapshot.run && autoFindSnapshot.run.historyMode !== settings.autoFindHistoryMode ? (
                <p>표시된 결과는 이전 기준입니다. 다음 실행부터 ‘{autoFindHistoryModeLabel(settings.autoFindHistoryMode)}’ 기준을 적용합니다.</p>
              ) : null}
              {((autoFindSnapshot.run && autoFindSnapshot.run.historyMode !== "include_all_history" && autoFindSnapshot.cutoffEvidence.length)
                || autoFindSnapshot.truncations.length) ? <details>
                <summary>검색 범위·제한</summary>
                {autoFindSnapshot.run && autoFindSnapshot.run.historyMode !== "include_all_history" && autoFindSnapshot.cutoffEvidence.length ? (
                  <ul aria-label="Auto Find 작가·그룹별 검색 시작점">
                    {autoFindSnapshot.cutoffEvidence.map(evidence => <li key={`${evidence.namespace ?? "artist"}:${evidence.artist}`}>
                      {evidence.namespace === "group" ? "그룹" : "작가"} · {evidence.artist}: {autoFindCutoffDescription(evidence, autoFindSnapshot.run!.historyMode)}
                    </li>)}
                  </ul>
                ) : null}
                {autoFindSnapshot.truncations.length ? <ul aria-label="Auto Find 결과 제한 경고">
                  {autoFindSnapshot.truncations.map(truncation => <li key={`${truncation.namespace ?? "artist"}:${truncation.artist}-${truncation.limit}`}>
                    {truncation.namespace === "group" ? "그룹" : "작가"} · {truncation.artist}: 검색 범위 내 후보 {truncation.eligibleCount}개 중 {truncation.limit}개만 표시했습니다.
                  </li>)}
                </ul> : null}
              </details> : null}
            </ResultDetails> : ui.view === "downloads" ? <ResultDetails key="downloads" count={visible.length}
              busy={duplicateRun?.state === "running" || internalRun?.state === "running" || reconcilingArtifacts}
              error={!!duplicateError || !!internalError || duplicateRun?.state === "failed" || internalRun?.state === "failed"}>
              <p role="status">{currentDuplicateStatus}</p>
              {duplicateRun?.state === "running" ? <button type="button" className="text-button danger-button" disabled={duplicatePending} onClick={() => void cancelDuplicateScan()}>중복 검사 취소</button> : null}
              <p role="status">{currentInternalStatus}</p>
              {internalRun?.state === "running" ? <button type="button" className="text-button danger-button" disabled={internalPending} onClick={() => void cancelInternalScan()}>내부 검사 취소</button> : null}
              {internalSnapshot.skips.length ? <details className="internal-scan-skips">
                <summary>내부 검사 제외 항목 {internalSnapshot.skips.length}개</summary>
                <p>500페이지 이상 앨범은 내부 페이지 검사에서만 제외됩니다.</p>
                <ul>{internalSnapshot.skips.map(skip => <li key={skip.entryId}>#{skip.galleryId} · {skip.pageCount}p</li>)}</ul>
              </details> : null}
              {duplicateError ? <button type="button" className="text-button compact" onClick={() => void hydrateDuplicateSnapshot(true)}>결과 다시 불러오기</button> : null}
              {internalError ? <button type="button" className="text-button compact" onClick={() => void hydrateInternalSnapshot(true)}>내부 결과 다시 불러오기</button> : null}
            </ResultDetails> : <div className="context-summary">{visible.length}개 결과</div>}
          </section>
          <SelectionToolbar
            active={multiSelectionMode}
            count={ui.selection.ids.size}
            downloadsView={ui.view === "downloads"}
            restoreMode={selectedIds.length > 0 && selectedIds.every((id) => displayGalleries.get(id)?.download?.state === "quarantined")}
            cancelCount={selectedRunningIds.length}
            cancelPending={selectionCancelPending}
            downloadPending={selectionDownloadPending}
            onCompare={ui.view === "downloads" ? () => {
              if (selectedIds.length !== 2) return;
              setPairCompareIds([String(selectedIds[0]), String(selectedIds[1])]);
              setPairCompareOpen(value => !value);
            } : undefined}
            compareDisabled={duplicatePending || duplicateRun?.state === "running"}
            onCancelDownloads={() => void cancelGalleries(selectedRunningIds)}
            onAll={() => dispatch({ type: "selection.all", ids: renderedActionableIds })}
            onClear={() => dispatch({ type: "selection.clear" })}
            onPrimary={() => void queueGalleries(selectedIds)}
            onDelete={() => ui.view === "downloads"
              ? void quarantineGalleries(selectedIds)
              : ui.view === "auto-find"
                ? void excludeAutoFindCandidates(selectedIds)
                : showToast("후보 제외는 Auto Find 화면에서 사용할 수 있습니다.")}
          />
          <section id="gallery-viewport" data-tour="hitomi-albums" ref={galleryViewport} className="gallery-viewport">
            {settingsLoading ? (
              <div className="loading-state" role="status"><span className="spinner" /> 저장된 화면 설정을 불러오는 중</div>
            ) : ((ui.view === "explore" && query.phase === "submitting" && !visible.length)
              || (ui.view === "downloads" && downloadsLoading && !visible.length)
              || (ui.view === "auto-find" && autoFindLoading && !visible.length)) ? (
              <GalleryGridSkeleton columns={galleryColumns} previewWidth={previewWidth} displayMode={currentGalleryDisplayMode} />
            ) : ui.view === "explore" && query.phase === "idle" ? (
              <div className="empty-state"><FluentIcon glyph="\uE721" /><h2>검색을 시작해 주세요</h2><p>검색어와 언어·정렬 필터를 정한 뒤 검색 버튼을 눌러 주세요.</p></div>
            ) : ui.view === "explore" && query.error && !query.page ? (
              <div className="empty-state" role="alert"><FluentIcon glyph="\uE7BA" /><h2>검색 결과를 불러오지 못했습니다</h2><p>{query.error.message}</p><button type="button" className="text-button" onClick={() => {
                const activeId = activeExploreContextIdRef.current;
                const context = activeId ? exploreContexts.current.get(activeId) : undefined;
                if (context?.request) startExploreSearch(context.request, { displayValue: context.displayValue, label: context.label, replace: true });
              }}>다시 시도</button></div>
            ) : ui.view === "downloads" && downloadsError && !visible.length ? (
              <div className="empty-state" role="alert"><FluentIcon glyph="\uE7BA" /><h2>다운로드 목록을 불러오지 못했습니다</h2><p>{downloadsError}</p><button type="button" className="text-button" onClick={() => setDownloadsRefresh((value) => value + 1)}>다시 시도</button></div>
            ) : ui.view === "auto-find" && autoFindError && !autoFindSnapshot.candidates.length ? (
              <div className="empty-state" role="alert"><FluentIcon glyph="\uE7BA" /><h2>자동 탐색 결과를 불러오지 못했습니다</h2><p>{autoFindError}</p><button type="button" className="text-button" onClick={() => void hydrateAutoFind(true)}>다시 시도</button></div>
            ) : visible.length ? (
              (ui.view === "auto-find" || ui.view === "downloads") ? (
                ui.grouping[ui.view] === "all"
                  ? renderGalleryGrid(renderedVisible, `${config.title} 전체 목록`)
                  : ui.view === "downloads" && ui.grouping.downloads === "artist"
                    ? <DownloadArtistFolderGrid
                        groups={groupedVisible}
                        thumbnailClient={thumbnailClient}
                        coverRetainer={sessionCoverRetainer.current!}
                        previewGalleryIdsByArtist={savedPreviews.artistGalleryIds}
                        columns={artistFolderColumns}
                        previewWidth={previewWidth}
                        favoriteMetadata={favoriteMetadataForDisplay}
                        collapsedGroupKeys={visibleCollapsedGroupKeys}
                        onToggle={toggleGroupCollapsed}
                        onPreviewItems={hydrateNearbyDownloadDetails}
                        onStopPreviewItems={discardNearbyDownloadDetails}
                        renderGrid={renderGalleryGrid}
                      />
                    : <GalleryAccordionGroups
                        groups={groupedVisible}
                        view={ui.view}
                        previewWidth={previewWidth}
                        collapsedGroupKeys={visibleCollapsedGroupKeys}
                        onToggle={toggleGroupCollapsed}
                        renderGrid={renderGalleryGrid}
                      />
              ) : renderGalleryGrid(renderedVisible, config.title)
            ) : (
              <div className="empty-state"><FluentIcon glyph="\uE11A" /><h2>표시할 갤러리가 없습니다</h2><p>{ui.view === "auto-find" ? "즐겨찾기 작가나 그룹을 추가한 뒤 명시적으로 갱신하거나 현재 검색·언어 필터를 바꿔 보세요." : "검색어나 언어·상태 필터를 바꿔 보세요."}</p></div>
            )}
            {ui.view === "explore" && query.page ? (
              <AutoFindPager
                page={query.pendingPage ?? query.page.page}
                totalPages={Math.max(1, query.page.totalPages)}
                onPageChange={(page) => void loadExplorePage(page)}
                ariaLabel="Explore 페이지"
                className="explore-pager"
                busy={query.phase === "loading-page"}
              />
            ) : null}
            {ui.view === "auto-find" ? (
              <AutoFindPager
                page={autoFindPagination.page}
                totalPages={autoFindPagination.totalPages}
                onPageChange={changeAutoFindPage}
              />
            ) : null}
            {ui.view === "downloads" && ui.grouping.downloads === "all" ? (
              <AutoFindPager
                page={downloadsPagination.page}
                totalPages={downloadsPagination.totalPages}
                onPageChange={changeDownloadsPage}
                ariaLabel="다운로드 목록 페이지"
                className="downloads-pager"
              />
            ) : null}
          </section>
          {ui.view === "auto-find" || ui.view === "downloads" ? (
            <GalleryScrollPositionHint
              rootRef={galleryViewport}
              view={ui.view}
              grouping={ui.grouping[ui.view]}
              items={renderedVisible}
              groups={groupedVisible}
            />
          ) : null}
        </main>
      </div>

      <DetailWorkspace
        pageOpenRequest={savedPageRequest}
        tabs={ui.detail.tabs}
        activeId={ui.detail.activeId}
        minimized={ui.detail.minimized}
        galleries={displayGalleries}
        favoriteMetadata={favoriteMetadataForDisplay}
        previewWidth={previewWidth}
        relatedPreviewWidth={settings.relatedPreviewWidth}
        backend={backend}
        onActivate={(id) => dispatch({ type: "detail.activate", id })}
        onClose={closeDetail}
        onCloseAll={closeAllDetails}
        onMinimize={() => dispatch({ type: "detail.minimize", minimized: true })}
        onRestore={() => dispatch({ type: "detail.minimize", minimized: false })}
        onOpenRelated={openRelatedDetail}
        onQueue={(id) => void queueGalleries([id])}
        onCancelDownload={(id) => void cancelGalleries([id])}
        onExclude={(id) => void excludeExploreGalleries([id])}
        pendingDownloadEntryIds={pendingDownloadEntries}
        cancellingDownloadEntryIds={cancellingDownloadEntries}
        onSetRepresentativePreview={setRepresentativePreview}
        onOpenDownloadFolder={(entryId) => void openDownloadFolder(entryId)}
        onMetadataSearch={searchMetadata}
        onMetadataFavorite={toggleMetadataFavorite}
      />
        </>
        ) : null,
        favoriteMetadata: favoriteMetadataForDisplay,
        activityCount: activityBadgeCount,
        onActivity: () => shell.activityOpen ? closeActivity() : openActivity(),
        onActivityRecord: recordDanbooruActivity,
        onMetadataFavorite: toggleMetadataFavorite,
        onOpenPersonalLibrary: () => { setPersonalLibraryVisited(true); setPersonalLibraryOpen(true); dispatch({ type: "detail.minimize", minimized: true }); },
      })}

      <ActivityDrawer
        queueSummary={<QueueSummary snapshot={workQueue.snapshot} scoped={workQueue.query.sequence !== undefined} error={workQueue.error} />}
        queuePanel={<WorkQueuePanel snapshot={workQueue.snapshot} query={workQueue.query} error={workQueue.error} onQuery={workQueue.setQuery} onRefresh={workQueue.refresh} onCancelEntries={cancelQueueEntries} onOpen={(id, options) => { if (!options?.background) closeActivity(); openDetail(id, options); }} />}
        containmentGroups={priorityContainmentGroups}
        containmentLoading={overlapInventoryLoading}
        onReviewContainment={(keeperId, reviewId) => {
          cancelAutomationReviewSequence();
          setContainmentKeeperId(keeperId);
          setDownloadOverlapReviewId(reviewId);
          setDownloadOverlapReview(null);
          setContainmentBatchProgress(undefined);
          setContainmentBatchGroup(undefined);
          dispatch({ type: "overlay.review", galleryId: keeperId });
          closeActivity();
          void hydrateDownloadOverlapReview(reviewId);
        }}
        open={shell.activityOpen && shell.source !== "chzzk"}
        galleries={allGalleries}
        sessionDownloads={sessionDownloadActivities}
        automaticOverlapActivities={automaticOverlapActivities}
        automationHistory={downloadOverlapAutomationHistory}
        automationHistoryLoading={downloadOverlapAutomationHistoryLoading}
        automationHistoryError={downloadOverlapAutomationHistoryError}
        automationHistoryTotalItems={downloadOverlapAutomationHistoryTotalItems}
        automationHistoryUnacknowledgedItems={downloadOverlapAutomationHistoryUnacknowledgedItems}
        automationHistoryPendingReviewIds={downloadOverlapAutomationHistoryPendingReviewIds}
        automationSequenceLoading={automationSequenceLoading}
        onStartAutomationSequence={() => void startAutomationReviewSequence()}
        danbooruActivities={danbooruSessionActivities}
        duplicateExcludedGalleryIds={duplicateHiddenGalleryIds}
        onClose={closeActivity}
        onReview={openReview}
        onReviewOverlap={openAutomaticOverlapReview}
        onAcknowledgeAutomationHistory={(reviewId) => void acknowledgeDownloadOverlapAutomationHistory(reviewId)}
        onRestoreAutomationExclusions={(reviewId, galleryIds) => void restoreDownloadOverlapAutomationExclusions(reviewId, galleryIds)}
        onRetryAutomationHistory={() => void hydrateDownloadOverlapAutomationHistoryPage(1, true)}
        onLoadMoreAutomationHistory={loadMoreDownloadOverlapAutomationHistory}
        onRetry={(id) => void retryGallery(id)}
        onCancel={(id) => void cancelGalleries([id])}
        pendingEntryIds={pendingDownloadEntries}
      />

      <SettingsDialog
        onChooseDownloadRoot={() => backend.downloadRootChoose()}
        open={shell.settingsOpen}
        settings={settings}
        loading={settingsLoading}
        error={settingsError}
        onClose={() => setSettingsOpen(false)}
        onSave={async patch => (await shell.settingsStore.save(patch)).ok}
        onReplayTutorial={shell.replayTutorial}
        tutorialSourceLabel={workspaceRegistry[shell.source].label}
        onLoadStorageUsage={loadStorageUsage}
        onPreviewLayout={setSettingsPreview}
        onPreviewFolderName={previewFolderNameTemplate}
        onMaintenance={runMaintenance}
        onCheckForUpdates={() => shell.checkForUpdates("manual")}
        onTagCatalogRefresh={refreshTagCatalog}
        tagCatalogStatus={tagCatalogStatus}
        tagCatalogRefreshing={tagCatalogRefreshing}
        onLoadExplorationExclusions={loadExplorationExclusionsAndSync}
        onRestoreExplorationExclusions={restoreExplorationExclusionsAndSync}
      />

      <KeyboardShortcutsDialog
        open={keyboardShortcutsOpen}
        onClose={() => setKeyboardShortcutsOpen(false)}
      />

      {shell.tutorialSource ? <TutorialDialog key={shell.tutorialSource} open steps={tutorialStepsBySource[shell.tutorialSource]}
        onStart={() => {
          setSettingsOpen(false); closeActivity();
          if (shell.tutorialSource === "hitomi") navigateView("explore");
          dispatch({ type: "detail.minimize", minimized: true });
        }}
        onStepChange={(id) => {
          // Each guide stays in its own service; Back must not switch sources.
          if (shell.tutorialSource !== "hitomi") return;
          setSettingsOpen(id === "folder");
          dispatch({ type: "detail.minimize", minimized: !["detail", "follow"].includes(id) });
          if (["settings", "search", "album"].includes(id)) navigateView("explore");
          setActivityOpen(id === "activity-info");
        }}
        onClose={() => { setSettingsOpen(false); shell.closeTutorial(); }} /> : null}

      {exclusionContext ? <ExcludedAlbumDialog
        context={exclusionContext}
        title={displayGalleries.get(exclusionContext.galleryId)?.title ?? `앨범 #${exclusionContext.galleryId}`}
        onClose={() => setExclusionContext(null)}
        onRestore={() => restoreExcludedGallery(exclusionContext.galleryId)}
        onReview={reviewExclusion}
        onOpenRetained={() => {
          if (exclusionContext.retainedGallery) {
            setExclusionContext(null);
            openDetail(exclusionContext.retainedGallery.galleryId);
          }
        }}
      /> : null}
      <DuplicateReviewDialog
        open={ui.overlays.reviewGalleryId !== null && duplicateReviewCandidateId !== null}
        review={duplicateReview ?? undefined}
        galleries={displayGalleries}
        loading={duplicateReviewLoading}
        error={duplicateReviewError}
        decisionPending={duplicateDecisionPending}
        previewWidth={previewWidth}
        thumbnailClient={thumbnailClient}
        browserFixture={backend.runtime === "browser-mock"}
        onClose={closeDuplicateReview}
        onRetry={() => duplicateReviewCandidateId && void hydrateDuplicateReview(duplicateReviewCandidateId)}
        onRescan={() => duplicateReview && void startDuplicateScan([duplicateReview.candidate.parent.galleryId, duplicateReview.candidate.candidate.galleryId])}
        onDecision={(request) => void applyDuplicateDecision(request)}
        onMergePages={(request) => void applyCompletedPairMerge(request)}
      />

      <DownloadOverlapReviewDialog
        containmentGroup={containmentBatchGroup ?? containmentGroup ?? undefined}
        containmentLoading={overlapInventoryLoading}
        onApplyContainmentBatch={(keys) => void applyContainmentBatch(keys)}
        batchProgress={containmentBatchProgress}
        open={ui.overlays.reviewGalleryId !== null && downloadOverlapReviewId !== null}
        review={downloadOverlapReview ?? undefined}
        loading={downloadOverlapLoading}
        error={downloadOverlapError}
        decisionPending={downloadOverlapDecisionPending}
        automationHistoryItem={downloadOverlapAutomationHistoryItem}
        automationHistoryPending={downloadOverlapReviewId !== null
          && downloadOverlapAutomationHistoryPendingReviewIds.has(downloadOverlapReviewId)}
        onAcknowledgeAutomationHistory={(reviewId) => void acknowledgeDownloadOverlapAutomationReview(reviewId)}
        onRestoreAutomationExclusions={(reviewId, galleryIds) => void restoreDownloadOverlapAutomationReview(reviewId, galleryIds)}
        automationReviewSequence={automationReviewSession ? {
          position: automationReviewSession.index + 1,
          total: automationReviewSession.items.length,
          canPrevious: automationReviewSession.index > 0,
          canNext: automationReviewSession.index < automationReviewSession.items.length - 1,
        } : undefined}
        onPreviousAutomationReview={() => {
          const session = automationReviewSessionRef.current;
          if (session) navigateAutomationReviewSequence(session.index - 1);
        }}
        onNextAutomationReview={() => {
          const session = automationReviewSessionRef.current;
          if (session) navigateAutomationReviewSequence(session.index + 1);
        }}
        browserFixture={backend.runtime === "browser-mock"}
        autoMode={settings.downloadOverlapAutoMode}
        previewWidth={previewWidth}
        thumbnailClient={thumbnailClient}
        onClose={closeDownloadOverlapReview}
        onRetry={() => downloadOverlapReviewId && void hydrateDownloadOverlapReview(downloadOverlapReviewId)}
        onDecision={(request) => void applyDownloadOverlapDecision(request)}
        onMergePages={(request) => void applyDownloadOverlapMerge(request)}
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

    </DownloadProgressContext.Provider>
  );
}

function GroupingControl({ value, onChange, includeGroups = false, includeDays = true }: { value: GalleryGrouping; onChange: (value: GalleryGrouping) => void; includeGroups?: boolean; includeDays?: boolean }) {
  return (
    <div className="segmented gallery-grouping-control" role="group" aria-label="표시 방식">
      <button type="button" aria-pressed={value === "all"} className={value === "all" ? "is-active" : ""} onClick={() => onChange("all")}>전체</button>
      {includeDays ? <button type="button" aria-pressed={value === "day"} className={value === "day" ? "is-active" : ""} onClick={() => onChange("day")}>기간별</button> : null}
      <button type="button" aria-pressed={value === "artist"} className={value === "artist" ? "is-active" : ""} onClick={() => onChange("artist")}>{includeGroups ? "작가·그룹별" : "작가별"}</button>
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
        const label = group.label;
        return (
          <section className={`gallery-group${collapsed ? " is-collapsed" : ""}`} key={group.key}>
            <h2>
              <button
                type="button"
                className="gallery-group-toggle"
                aria-expanded={!collapsed}
                onClick={() => onToggle(storageKey)}
              >
                <span className="gallery-group-copy">
                  <span className="gallery-group-title" style={{ fontSize: `${titleSize}px` }}>{label}</span>
                </span>
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
