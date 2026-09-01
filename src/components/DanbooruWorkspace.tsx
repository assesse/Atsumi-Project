import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { BackendClient } from "../api/backend";
import type {
  DanbooruAutocompleteItem,
  DanbooruDownloadRecord,
  DanbooruDownloadsPage,
  DanbooruPost,
  DanbooruRelatedPosts,
  DanbooruSearchPage,
} from "../api/contracts";
import type { ContentSource, ViewId } from "../core/types";
import { alignPageSizeToColumns } from "../layout/pageSizeAlignment";
import {
  activeDanbooruFilterCount,
  buildDanbooruSearchQuery,
  danbooruLimitedTermCount,
  DANBOORU_FILE_TYPES,
  DANBOORU_RATINGS,
  DANBOORU_SEARCH_PREFERENCES_CHANGED,
  DANBOORU_SORTS,
  defaultDanbooruSearchFilters,
  loadDanbooruSearchPreferences,
  sanitizeDanbooruSearchFilters,
  type DanbooruFileType,
  type DanbooruRating,
  type DanbooruSearchFilters,
  type DanbooruSort,
} from "../danbooru/searchPreferences";
import { FluentIcon } from "./FluentIcon";
import type { DanbooruSessionActivity } from "./ActivityDrawer";
import { DropdownSelect } from "./DropdownSelect";
import { SideRail } from "./SideRail";

type DanbooruView = "explore" | "downloads";

type DanbooruWorkspaceProps = {
  backend: BackendClient;
  railCollapsed: boolean;
  pageSize: number;
  previewWidth: number;
  favoriteMetadata?: ReadonlySet<string>;
  activityCount?: number;
  activityOpen?: boolean;
  privacyMode?: boolean;
  privacyModePending?: boolean;
  onToggleRail: () => void;
  onSourceChange: (source: ContentSource) => void;
  onActivity?: () => void;
  onPrivacyModeToggle?: () => void;
  onActivityRecord?: (activity: DanbooruSessionActivity) => void;
  onMetadataFavorite?: (token: string) => void;
  onOpenSettings: () => void;
};

type PersistedDanbooruState = {
  view: DanbooruView;
  exploreDraft: string;
  exploreCommitted: string;
  downloadsDraft: string;
  downloadsCommitted: string;
  explorePage: number;
  downloadsPage: number;
  filters: DanbooruSearchFilters;
};

const stateKey = "atsumi.danbooru-state.v1";
const emptyFavoriteMetadata: ReadonlySet<string> = new Set();
const defaultState: PersistedDanbooruState = {
  view: "explore",
  exploreDraft: "",
  exploreCommitted: "",
  downloadsDraft: "",
  downloadsCommitted: "",
  explorePage: 1,
  downloadsPage: 1,
  filters: defaultDanbooruSearchFilters(),
};

const loadState = (): PersistedDanbooruState => {
  const searchPreferences = loadDanbooruSearchPreferences();
  try {
    const parsed = JSON.parse(window.localStorage.getItem(stateKey) ?? "null") as Partial<PersistedDanbooruState> | null;
    if (!parsed) return { ...defaultState, filters: searchPreferences };
    return {
      view: parsed.view === "downloads" ? "downloads" : "explore",
      exploreDraft: typeof parsed.exploreDraft === "string" ? parsed.exploreDraft : "",
      exploreCommitted: typeof parsed.exploreCommitted === "string" ? parsed.exploreCommitted : "",
      downloadsDraft: typeof parsed.downloadsDraft === "string" ? parsed.downloadsDraft : "",
      downloadsCommitted: typeof parsed.downloadsCommitted === "string" ? parsed.downloadsCommitted : "",
      explorePage: Number.isInteger(parsed.explorePage) && Number(parsed.explorePage) > 0 ? Number(parsed.explorePage) : 1,
      downloadsPage: Number.isInteger(parsed.downloadsPage) && Number(parsed.downloadsPage) > 0 ? Number(parsed.downloadsPage) : 1,
      // The committed query above owns the restored result set. Filters are the
      // draft for the next search, so Settings must remain their source of truth.
      filters: searchPreferences,
    };
  } catch {
    return { ...defaultState, filters: searchPreferences };
  }
};

const saveState = (state: PersistedDanbooruState): void => {
  try {
    window.localStorage.setItem(stateKey, JSON.stringify(state));
  } catch {
    // Browsing remains usable when local preference storage is unavailable.
  }
};

const ratingLabel: Record<string, string> = {
  g: "General",
  s: "Sensitive",
  q: "Questionable",
  e: "Explicit",
};

const formatTag = (tag: string): string => tag.replaceAll("_", " ");

const formatBytes = (bytes: number): string => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "크기 미상";
  const units = ["B", "KiB", "MiB", "GiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
};

// The card is a 4:5 cover crop. A 180px `srcset` candidate can look wide enough to
// the browser while still being far too short after cropping, so cards always use
// the large/sample projection and keep only the preview as an availability fallback.
const cardMediaUrl = (post: DanbooruPost): string | undefined => post.largeUrl ?? post.previewUrl;
const isPlayableVideo = (post: DanbooruPost): boolean => post.fileExt === "mp4" || post.fileExt === "webm";
type DanbooruFavoriteKind = "artist" | "series" | "character" | "tag";

const danbooruFavoriteToken = (kind: DanbooruFavoriteKind, tag: string): string => {
  const value = tag.trim().toLocaleLowerCase();
  return kind === "tag" ? value : `${kind}:${value}`;
};

const postTitle = (post: DanbooruPost): string => {
  const artist = post.artists.at(0);
  const subject = post.characters.at(0) ?? post.copyrights.at(0);
  if (artist && subject) return `${formatTag(artist)} · ${formatTag(subject)}`;
  if (artist) return formatTag(artist);
  if (subject) return formatTag(subject);
  return `Danbooru post #${post.id}`;
};

const activeToken = (value: string): string => value.trimEnd().split(/\s+/).at(-1)?.replace(/^-/, "") ?? "";

const replaceActiveToken = (value: string, replacement: string): string => {
  const trailing = value.match(/\s+$/)?.[0] ?? "";
  const trimmed = value.trimEnd();
  const separator = trimmed.lastIndexOf(" ");
  const current = separator >= 0 ? trimmed.slice(separator + 1) : trimmed;
  const prefix = current.startsWith("-") ? "-" : "";
  const head = separator >= 0 ? trimmed.slice(0, separator + 1) : "";
  return `${head}${prefix}${replacement}${trailing}`;
};

export function DanbooruWorkspace({
  backend,
  railCollapsed,
  pageSize,
  previewWidth,
  favoriteMetadata = emptyFavoriteMetadata,
  activityCount = 0,
  activityOpen = false,
  privacyMode = false,
  privacyModePending = false,
  onToggleRail,
  onSourceChange,
  onActivity = () => undefined,
  onPrivacyModeToggle = () => undefined,
  onActivityRecord = () => undefined,
  onMetadataFavorite = () => undefined,
  onOpenSettings,
}: DanbooruWorkspaceProps) {
  const persisted = useRef(loadState()).current;
  const [view, setView] = useState<DanbooruView>(persisted.view);
  const [exploreDraft, setExploreDraft] = useState(persisted.exploreDraft);
  const [exploreCommitted, setExploreCommitted] = useState(persisted.exploreCommitted);
  const [downloadsDraft, setDownloadsDraft] = useState(persisted.downloadsDraft);
  const [downloadsCommitted, setDownloadsCommitted] = useState(persisted.downloadsCommitted);
  const [explorePage, setExplorePage] = useState(persisted.explorePage);
  const [downloadsPageNumber, setDownloadsPageNumber] = useState(persisted.downloadsPage);
  const [filters, setFilters] = useState<DanbooruSearchFilters>(persisted.filters);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [searchPage, setSearchPage] = useState<DanbooruSearchPage | null>(null);
  const [downloadsPage, setDownloadsPage] = useState<DanbooruDownloadsPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [detail, setDetail] = useState<DanbooruPost | null>(null);
  const [suggestions, setSuggestions] = useState<DanbooruAutocompleteItem[]>([]);
  const [downloadedIds, setDownloadedIds] = useState<Set<number>>(new Set());
  const [pendingDownloads, setPendingDownloads] = useState<Set<number>>(new Set());
  const [notice, setNotice] = useState<string | null>(null);
  const requestSequence = useRef(0);
  const suggestionSequence = useRef(0);
  const content = useRef<HTMLElement>(null);
  const gridWidth = Math.max(160, Math.min(360, previewWidth));
  const [gridColumns, setGridColumns] = useState(1);
  const [gridMeasured, setGridMeasured] = useState(false);
  const restoredPageLoaded = useRef(false);

  const normalizedPageSize = Math.max(10, Math.min(100, pageSize));
  const alignedPageSize = alignPageSizeToColumns(normalizedPageSize, gridColumns, 100);
  const loadExplore = useCallback(async (tags: string, page: number) => {
    const sequence = ++requestSequence.current;
    setLoading(true);
    setError(null);
    const result = await backend.danbooruSearch({ tags, page, pageSize: alignedPageSize }).catch(() => null);
    if (sequence !== requestSequence.current) return;
    setLoading(false);
    if (!result) {
      setError("Danbooru 검색 요청을 전달하지 못했습니다.");
      return;
    }
    if (!result.ok) {
      setError(result.error.message);
      return;
    }
    setSearchPage(result.data);
    setExplorePage(result.data.page);
  }, [alignedPageSize, backend]);

  const loadDownloads = useCallback(async (page: number, query: string) => {
    const sequence = ++requestSequence.current;
    setLoading(true);
    setError(null);
    const result = await backend.danbooruDownloadsList({ page, pageSize: alignedPageSize, query }).catch(() => null);
    if (sequence !== requestSequence.current) return;
    setLoading(false);
    if (!result) {
      setError("Danbooru 다운로드 목록을 불러오지 못했습니다.");
      return;
    }
    if (!result.ok) {
      setError(result.error.message);
      return;
    }
    setDownloadsPage(result.data);
    setDownloadsPageNumber(result.data.page);
    setDownloadedIds((current) => new Set([...current, ...result.data.items.map((item) => item.post.id)]));
  }, [alignedPageSize, backend]);

  useLayoutEffect(() => {
    const host = content.current;
    if (!host) return;
    const update = () => {
      const available = Math.max(0, host.clientWidth - 8);
      const columns = Math.max(1, Math.floor((available + 14) / (gridWidth + 14)));
      setGridColumns((current) => current === columns ? current : columns);
      setGridMeasured(true);
    };
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(host);
    return () => observer.disconnect();
  }, [gridWidth]);

  useEffect(() => {
    saveState({
      view,
      exploreDraft,
      exploreCommitted,
      downloadsDraft,
      downloadsCommitted,
      explorePage,
      downloadsPage: downloadsPageNumber,
      filters,
    });
  }, [downloadsCommitted, downloadsDraft, downloadsPageNumber, exploreCommitted, exploreDraft, explorePage, filters, view]);

  useEffect(() => {
    const preferencesChanged = (event: Event) => {
      setFilters(sanitizeDanbooruSearchFilters((event as CustomEvent<DanbooruSearchFilters>).detail));
    };
    window.addEventListener(DANBOORU_SEARCH_PREFERENCES_CHANGED, preferencesChanged);
    return () => window.removeEventListener(DANBOORU_SEARCH_PREFERENCES_CHANGED, preferencesChanged);
  }, []);

  useEffect(() => {
    if (!gridMeasured || restoredPageLoaded.current) return;
    restoredPageLoaded.current = true;
    if (view === "explore") void loadExplore(exploreCommitted, explorePage);
    else void loadDownloads(downloadsPageNumber, downloadsCommitted);
  }, [downloadsCommitted, downloadsPageNumber, exploreCommitted, explorePage, gridMeasured, loadDownloads, loadExplore, view]);

  useEffect(() => {
    if (view !== "explore") {
      setSuggestions([]);
      return;
    }
    const token = activeToken(exploreDraft);
    if (token.length < 2 || /^\d+$/.test(token)) {
      setSuggestions([]);
      return;
    }
    const sequence = ++suggestionSequence.current;
    const timeout = window.setTimeout(() => {
      void backend.danbooruAutocomplete(token, 8).then((result) => {
        if (sequence === suggestionSequence.current) setSuggestions(result.ok ? result.data : []);
      }).catch(() => {
        if (sequence === suggestionSequence.current) setSuggestions([]);
      });
    }, 220);
    return () => window.clearTimeout(timeout);
  }, [backend, exploreDraft, view]);

  const navigate = (next: ViewId) => {
    const nextView: DanbooruView = next === "downloads" ? "downloads" : "explore";
    setView(nextView);
    setError(null);
    setSuggestions([]);
    if (nextView === "explore") void loadExplore(exploreCommitted, explorePage);
    else void loadDownloads(downloadsPageNumber, downloadsCommitted);
  };

  const submit = (
    value = view === "explore" ? exploreDraft : downloadsDraft,
    nextFilters = filters,
  ) => {
    const query = value.trim();
    setSuggestions([]);
    if (view === "explore") {
      setExploreDraft(query);
      const composed = buildDanbooruSearchQuery(query, nextFilters);
      if (danbooruLimitedTermCount(composed) > 2) {
        setError("현재 비로그인 검색은 제한 대상 조건을 2개까지 사용할 수 있습니다. 정렬을 사용하면 일반 태그는 1개까지 입력할 수 있습니다.");
        return;
      }
      setExploreCommitted(composed);
      setExplorePage(1);
      void loadExplore(composed, 1);
    } else {
      setDownloadsDraft(query);
      setDownloadsCommitted(query);
      setDownloadsPageNumber(1);
      void loadDownloads(1, query);
    }
  };

  const openRandom = async () => {
    if (loading) return;
    setLoading(true);
    setError(null);
    const result = await backend.danbooruRandom().catch(() => null);
    setLoading(false);
    if (!result) setError("랜덤 post를 불러오지 못했습니다.");
    else if (!result.ok) setError(result.error.message);
    else setDetail(result.data);
  };

  const download = async (post: DanbooruPost) => {
    if (pendingDownloads.has(post.id) || downloadedIds.has(post.id)) return;
    setPendingDownloads((current) => new Set(current).add(post.id));
    const result = await backend.danbooruDownload(post.id).catch(() => null);
    setPendingDownloads((current) => {
      const next = new Set(current);
      next.delete(post.id);
      return next;
    });
    if (!result) {
      setNotice("Danbooru 원본 저장 요청을 전달하지 못했습니다.");
      onActivityRecord({ id: `${post.id}:${Date.now()}`, postId: post.id, title: postTitle(post), detail: "원본 저장 실패", occurredAt: Date.now(), state: "failed" });
      return;
    }
    if (!result.ok) {
      setNotice(result.error.message);
      onActivityRecord({ id: `${post.id}:${Date.now()}`, postId: post.id, title: postTitle(post), detail: "원본 저장 실패", occurredAt: Date.now(), state: "failed" });
      return;
    }
    setDownloadedIds((current) => new Set(current).add(post.id));
    onActivityRecord({ id: `${post.id}:${Date.now()}`, postId: post.id, title: postTitle(post), detail: "원본 저장 완료", occurredAt: Date.now(), state: "completed" });
    setNotice(`${result.data.fileName} 원본을 저장했습니다.`);
    if (view === "downloads") void loadDownloads(downloadsPageNumber, downloadsCommitted);
  };

  const recordsById = useMemo(() => new Map(
    downloadsPage?.items.map((record) => [record.post.id, record]) ?? [],
  ), [downloadsPage]);
  const posts = view === "explore"
    ? searchPage?.items ?? []
    : downloadsPage?.items.map((record) => record.post) ?? [];
  const detailIndex = detail ? posts.findIndex((post) => post.id === detail.id) : -1;
  const previousDetail = detailIndex > 0 ? posts[detailIndex - 1] : undefined;
  const nextDetail = detailIndex >= 0 && detailIndex < posts.length - 1 ? posts[detailIndex + 1] : undefined;
  const activeFilterCount = activeDanbooruFilterCount(filters);
  const limitedTermCount = danbooruLimitedTermCount(buildDanbooruSearchQuery(exploreDraft, filters));

  return (
    <div className={`app-shell danbooru-shell${railCollapsed ? " sidebar-collapsed" : ""}`}>
      <SideRail
        view={view}
        collapsed={railCollapsed}
        autoFindCount={0}
        attentionCount={downloadsPage?.total ?? downloadedIds.size}
        sourceLabel={backend.runtime === "tauri" ? "Danbooru live" : "Danbooru fixture"}
        source="danbooru"
        onNavigate={navigate}
        onSourceChange={onSourceChange}
        onToggle={onToggleRail}
      />
      <main className="danbooru-workspace">
        <header className="danbooru-header">
          <form onSubmit={(event) => { event.preventDefault(); submit(); }} autoComplete="off">
            <FluentIcon glyph="\uE721" />
            <input
              type="search"
              value={view === "explore" ? exploreDraft : downloadsDraft}
              aria-label={view === "explore" ? "Danbooru 태그 또는 post ID 검색" : "저장한 Danbooru post 검색"}
              placeholder={view === "explore" ? "태그 최대 2개 또는 post ID" : "저장한 post ID·작가·태그 검색"}
              onChange={(event) => view === "explore" ? setExploreDraft(event.target.value) : setDownloadsDraft(event.target.value)}
            />
            <button type="submit" className="icon-button primary-soft" aria-label="검색" title="검색"><FluentIcon glyph="\uE721" /></button>
            {suggestions.length ? (
              <div className="danbooru-suggestions" role="listbox" aria-label="Danbooru 태그 제안">
                {suggestions.map((suggestion) => (
                  <button
                    key={`${suggestion.value}-${suggestion.category}`}
                    type="button"
                    role="option"
                    onClick={() => {
                      const value = replaceActiveToken(exploreDraft, suggestion.value);
                      setExploreDraft(value);
                      setSuggestions([]);
                    }}
                  >
                    <span>{formatTag(suggestion.label || suggestion.value)}</span>
                    <small>{suggestion.postCount.toLocaleString()} posts</small>
                  </button>
                ))}
              </div>
            ) : null}
          </form>
          <div className="danbooru-header-actions">
            {view === "explore" ? (
              <button type="button" className="icon-button" title="랜덤 post 열기" aria-label="랜덤 post 열기" disabled={loading} onClick={() => void openRandom()}><FluentIcon glyph="\uE8B1" /></button>
            ) : null}
            <button type="button" className="icon-button activity-button" title="활동 기록" aria-label="활동 기록" aria-controls="activity-panel" aria-expanded={activityOpen} onClick={onActivity}>
              <FluentIcon glyph="\uE9D9" />
              {activityCount > 0 ? <span className="activity-count">{activityCount}</span> : null}
            </button>
            <button type="button" className={`icon-button${privacyMode ? " is-active" : ""}`} title={privacyMode ? "미리보기 표시" : "미리보기 가리기"} aria-label="프라이버시 모드" aria-pressed={privacyMode} aria-busy={privacyModePending || undefined} disabled={privacyModePending} onClick={onPrivacyModeToggle}><FluentIcon glyph="\uE890" /></button>
            <button type="button" className="icon-button" title="설정" aria-label="설정" onClick={onOpenSettings}><FluentIcon glyph="\uE713" /></button>
          </div>
        </header>

        <div className="danbooru-overview">
          {view === "explore" ? <>
            <section className="danbooru-search-tools" aria-label="Danbooru 검색 조건과 정렬">
              <button
                type="button"
                className={`danbooru-filter-button${filtersOpen ? " is-active" : ""}`}
                aria-expanded={filtersOpen}
                onClick={() => setFiltersOpen((current) => !current)}
              >
                <FluentIcon glyph="\uE71C" /> 상세 조건
                {activeFilterCount ? <span>{activeFilterCount}</span> : null}
              </button>
              <DropdownSelect
                ariaLabel="Danbooru 정렬 기준"
                className="danbooru-sort-dropdown"
                variant="toolbar"
                prefix="정렬"
                value={filters.sort}
                options={DANBOORU_SORTS}
                onChange={(sort) => {
                    const next = { ...filters, sort: sort as DanbooruSort };
                    setFilters(next);
                    submit(exploreDraft, next);
                }}
              />
              <span className={`danbooru-tag-budget${limitedTermCount > 2 ? " is-over" : ""}`}>
                제한 대상 {limitedTermCount}/2 · rating/date/score 등은 제외 · 정렬은 1개 사용
              </span>
            </section>
            {filtersOpen ? (
              <DanbooruSearchFilterPanel
                filters={filters}
                onChange={setFilters}
                onApply={() => {
                  setFiltersOpen(false);
                  submit(exploreDraft, filters);
                }}
                onReset={() => setFilters(defaultDanbooruSearchFilters())}
              />
            ) : null}
          </> : null}

          <section className="danbooru-heading">
            <div>
              <span className="eyebrow">{view === "explore" ? "DANBOORU EXPLORE" : "DANBOORU DOWNLOADS"}</span>
              <h1>{view === "explore" ? "Danbooru post 탐색" : "저장한 Danbooru 원본"}</h1>
              <p>{view === "explore" ? "공개 API · 계정 없이 일반 태그 2개까지 검색" : "다운로드 루트의 Danbooru 인덱스"}</p>
            </div>
            <span className="danbooru-result-count">{view === "downloads" ? `${downloadsPage?.total ?? 0}개 저장` : `${posts.length}개 표시`}</span>
          </section>
        </div>

        <section ref={content} className="danbooru-content" aria-busy={loading}>
          {error ? (
            <div className="empty-state" role="alert"><FluentIcon glyph="\uE7BA" /><h2>Danbooru 결과를 불러오지 못했습니다</h2><p>{error}</p><button type="button" className="text-button" onClick={() => view === "explore" ? void loadExplore(exploreCommitted, explorePage) : void loadDownloads(downloadsPageNumber, downloadsCommitted)}>다시 시도</button></div>
          ) : loading && !posts.length ? (
            <div className="loading-state" role="status"><span className="spinner" /> Danbooru 결과를 불러오는 중</div>
          ) : posts.length ? (
            <div className="danbooru-grid" style={{ "--danbooru-card-width": `${gridWidth}px` } as CSSProperties}>
              {posts.map((post) => (
                <DanbooruCard
                  key={post.id}
                  post={post}
                  record={recordsById.get(post.id)}
                  downloaded={downloadedIds.has(post.id)}
                  pending={pendingDownloads.has(post.id)}
                  onOpen={() => setDetail(post)}
                  onDownload={() => void download(post)}
                />
              ))}
            </div>
          ) : (
            <div className="empty-state"><FluentIcon glyph="\uE11A" /><h2>{view === "explore" ? "검색 결과가 없습니다" : "저장한 Danbooru post가 없습니다"}</h2><p>{view === "explore" ? "태그 또는 post ID를 바꿔 보세요." : "Explore에서 상세 화면을 열고 원본 저장을 선택하세요."}</p></div>
          )}
          <div className="pager danbooru-pager">
            <button type="button" className="text-button" disabled={loading || (view === "explore" ? explorePage <= 1 : (downloadsPage?.page ?? 1) <= 1)} onClick={() => {
              if (view === "explore") void loadExplore(exploreCommitted, explorePage - 1);
              else void loadDownloads((downloadsPage?.page ?? 1) - 1, downloadsCommitted);
            }}>이전</button>
            <span>{view === "explore" ? `${explorePage} 페이지` : `${downloadsPage?.page ?? 1} / ${downloadsPage?.totalPages ?? 1}`}{loading ? " · 불러오는 중" : ""}</span>
            <button type="button" className="text-button" disabled={loading || (view === "explore" ? !searchPage?.hasMore : (downloadsPage?.page ?? 1) >= (downloadsPage?.totalPages ?? 1))} onClick={() => {
              if (view === "explore") void loadExplore(exploreCommitted, explorePage + 1);
              else void loadDownloads((downloadsPage?.page ?? 1) + 1, downloadsCommitted);
            }}>다음</button>
          </div>
        </section>
      </main>

      {detail ? (
        <DanbooruDetail
          backend={backend}
          post={detail}
          downloaded={downloadedIds.has(detail.id)}
          pending={pendingDownloads.has(detail.id)}
          onClose={() => setDetail(null)}
          onPrevious={previousDetail ? () => setDetail(previousDetail) : undefined}
          onNext={nextDetail ? () => setDetail(nextDetail) : undefined}
          onDownload={() => void download(detail)}
          onOpenRelated={setDetail}
          favoriteMetadata={favoriteMetadata}
          onMetadataFavorite={onMetadataFavorite}
          onSearch={(tag) => {
            setDetail(null);
            setView("explore");
            setExploreDraft(tag);
            const query = buildDanbooruSearchQuery(tag, filters);
            setExploreCommitted(query);
            setExplorePage(1);
            void loadExplore(query, 1);
          }}
        />
      ) : null}
      {notice ? <div className="toast" role="status" onAnimationEnd={() => setNotice(null)}>{notice}</div> : null}
    </div>
  );
}

function DanbooruSearchFilterPanel({
  filters,
  onChange,
  onApply,
  onReset,
}: {
  filters: DanbooruSearchFilters;
  onChange: (filters: DanbooruSearchFilters) => void;
  onApply: () => void;
  onReset: () => void;
}) {
  const toggleRating = (rating: DanbooruRating, checked: boolean) => onChange({
    ...filters,
    ratings: checked
      ? DANBOORU_RATINGS.map(({ value }) => value).filter((value) => filters.ratings.includes(value) || value === rating)
      : filters.ratings.filter((value) => value !== rating),
  });
  const toggleFileType = (fileType: DanbooruFileType, checked: boolean) => onChange({
    ...filters,
    fileTypes: checked
      ? DANBOORU_FILE_TYPES.map(({ value }) => value).filter((value) => filters.fileTypes.includes(value) || value === fileType)
      : filters.fileTypes.filter((value) => value !== fileType),
  });
  return (
    <section className="danbooru-filter-panel" aria-label="Danbooru 상세 검색 조건">
      <div className="danbooru-filter-grid">
        <fieldset>
          <legend>등급</legend>
          <div className="danbooru-check-grid is-ratings">
            {DANBOORU_RATINGS.map((rating) => (
              <label key={rating.value} title={rating.description}>
                <input
                  type="checkbox"
                  checked={filters.ratings.includes(rating.value)}
                  onChange={(event) => toggleRating(rating.value, event.target.checked)}
                />
                <span className={`danbooru-rating is-${rating.value}`}>{rating.label}</span>
              </label>
            ))}
          </div>
          <small>미선택 또는 전체 선택은 등급을 제한하지 않습니다.</small>
        </fieldset>
        <fieldset>
          <legend>파일 형식</legend>
          <div className="danbooru-check-grid">
            {DANBOORU_FILE_TYPES.map((fileType) => (
              <label key={fileType.value}>
                <input
                  type="checkbox"
                  checked={filters.fileTypes.includes(fileType.value)}
                  onChange={(event) => toggleFileType(fileType.value, event.target.checked)}
                />
                {fileType.label}
              </label>
            ))}
          </div>
        </fieldset>
        <fieldset>
          <legend>등록 기간</legend>
          <div className="danbooru-range-fields">
            <label><span>시작</span><input type="date" aria-label="Danbooru 등록 시작일" value={filters.dateFrom} max={filters.dateTo || undefined} onChange={(event) => onChange({ ...filters, dateFrom: event.target.value })} /></label>
            <label><span>종료</span><input type="date" aria-label="Danbooru 등록 종료일" value={filters.dateTo} min={filters.dateFrom || undefined} onChange={(event) => onChange({ ...filters, dateTo: event.target.value })} /></label>
          </div>
          <small>기간과 점수순을 함께 쓰면 해당 기간의 인기 게시물을 볼 수 있습니다.</small>
        </fieldset>
        <fieldset>
          <legend>최소 기준</legend>
          <div className="danbooru-range-fields">
            <label><span>점수</span><input type="number" aria-label="Danbooru 최소 점수" step="1" value={filters.minimumScore} onChange={(event) => onChange({ ...filters, minimumScore: event.target.value })} /></label>
            <label><span>즐겨찾기</span><input type="number" aria-label="Danbooru 최소 즐겨찾기" min="0" step="1" value={filters.minimumFavorites} onChange={(event) => onChange({ ...filters, minimumFavorites: event.target.value })} /></label>
          </div>
        </fieldset>
        <fieldset>
          <legend>게시물 관계</legend>
          <DropdownSelect
            ariaLabel="Danbooru 게시물 관계"
            className="danbooru-filter-dropdown"
            value={filters.relationship}
            options={[
              { value: "any", label: "관계 제한 없음" },
              { value: "has_parent", label: "부모가 있는 변형판" },
              { value: "no_parent", label: "부모가 없는 게시물" },
              { value: "has_children", label: "자식 변형판이 있는 게시물" },
              { value: "no_children", label: "자식 변형판이 없는 게시물" },
            ]}
            onChange={(relationship) => onChange({ ...filters, relationship })}
          />
        </fieldset>
      </div>
      <footer>
        <button type="button" className="text-button" onClick={onReset}>조건 초기화</button>
        <button type="button" className="text-button primary" onClick={onApply}>조건 적용</button>
      </footer>
    </section>
  );
}

function DanbooruCard({
  post,
  record,
  downloaded,
  pending,
  onOpen,
  onDownload,
}: {
  post: DanbooruPost;
  record?: DanbooruDownloadRecord;
  downloaded: boolean;
  pending: boolean;
  onOpen: () => void;
  onDownload: () => void;
}) {
  const mediaUrl = cardMediaUrl(post);
  return (
    <article className="danbooru-card" data-post-id={post.id}>
      <button type="button" className="danbooru-card-preview" onClick={onOpen} aria-label={`${postTitle(post)} 상세 열기`}>
        {mediaUrl ? <img src={mediaUrl} alt="" loading="lazy" decoding="async" referrerPolicy="no-referrer" /> : <span className="danbooru-media-missing"><FluentIcon glyph="\uEB9F" /> 미리보기 없음</span>}
        <span className={`danbooru-rating is-${post.rating}`}>{ratingLabel[post.rating] ?? post.rating.toUpperCase()}</span>
        {downloaded ? <span className="danbooru-downloaded"><FluentIcon glyph="\uE73E" /> 저장됨</span> : null}
      </button>
      <div className="danbooru-card-copy">
        <button type="button" className="danbooru-card-title" onClick={onOpen}>{postTitle(post)}</button>
        <span>#{post.id} · {post.imageWidth}×{post.imageHeight} · {post.fileExt.toUpperCase()}</span>
        <div className="danbooru-card-metrics"><span>점수 {post.score}</span><span>♥ {post.favoriteCount}</span></div>
        {record ? <small>{formatBytes(record.bytes)} · 로컬 원본</small> : null}
      </div>
      <button type="button" className={`danbooru-save-button${downloaded ? " is-complete" : ""}`} disabled={pending || downloaded || !post.fileUrl} onClick={onDownload}>
        <FluentIcon glyph={downloaded ? "\uE73E" : pending ? "\uE895" : "\uE896"} /> {downloaded ? "저장 완료" : pending ? "저장 중" : "원본 저장"}
      </button>
    </article>
  );
}

function DanbooruDetail({
  backend,
  post,
  downloaded,
  pending,
  onClose,
  onPrevious,
  onNext,
  onDownload,
  onOpenRelated,
  favoriteMetadata,
  onMetadataFavorite,
  onSearch,
}: {
  backend: BackendClient;
  post: DanbooruPost;
  downloaded: boolean;
  pending: boolean;
  onClose: () => void;
  onPrevious?: () => void;
  onNext?: () => void;
  onDownload: () => void;
  onOpenRelated: (post: DanbooruPost) => void;
  favoriteMetadata: ReadonlySet<string>;
  onMetadataFavorite: (token: string) => void;
  onSearch: (tag: string) => void;
}) {
  const mediaUrl = post.largeUrl ?? post.previewUrl;
  const playableVideo = isPlayableVideo(post) && Boolean(post.fileUrl);
  const dialog = useRef<HTMLElement>(null);
  const [related, setRelated] = useState<DanbooruRelatedPosts | null>(null);
  const [relatedLoading, setRelatedLoading] = useState(true);
  const [relatedError, setRelatedError] = useState<string | null>(null);
  const [relatedRevision, setRelatedRevision] = useState(0);
  useEffect(() => {
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const frame = window.requestAnimationFrame(() => dialog.current?.focus({ preventScroll: true }));
    return () => {
      window.cancelAnimationFrame(frame);
      if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
    };
  }, []);
  useEffect(() => {
    const keyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || document.querySelector("dialog[open]")) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
        return;
      }
      if (event.repeat || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
      const target = event.target instanceof Element
        ? event.target
        : document.activeElement instanceof Element
          ? document.activeElement
          : null;
      if (target?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      const key = event.key.toLocaleLowerCase();
      const navigate = event.key === "ArrowLeft" || event.code === "KeyA" || key === "a"
        ? onPrevious
        : event.key === "ArrowRight" || event.code === "KeyD" || key === "d"
          ? onNext
          : undefined;
      if (!navigate) return;
      event.preventDefault();
      event.stopPropagation();
      navigate();
    };
    window.addEventListener("keydown", keyDown, true);
    return () => window.removeEventListener("keydown", keyDown, true);
  }, [onClose, onNext, onPrevious]);
  useEffect(() => {
    let cancelled = false;
    setRelated(null);
    setRelatedError(null);
    setRelatedLoading(true);
    void backend.danbooruRelated({
      postId: post.id,
      ...(post.parentId ? { parentId: post.parentId } : {}),
      hasChildren: post.hasChildren,
    }).then((result) => {
      if (cancelled) return;
      if (result.ok) setRelated(result.data);
      else setRelatedError(result.error.message);
    }).catch(() => {
      if (!cancelled) setRelatedError("연결된 post 정보를 불러오지 못했습니다.");
    }).finally(() => {
      if (!cancelled) setRelatedLoading(false);
    });
    return () => { cancelled = true; };
  }, [backend, post.hasChildren, post.id, post.parentId, relatedRevision]);
  return (
    <div className="modal-backdrop danbooru-detail-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <section ref={dialog} className="danbooru-detail" role="dialog" aria-modal="true" aria-labelledby="danbooru-detail-title" aria-describedby="danbooru-detail-shortcuts" tabIndex={-1}>
        <header>
          <div><span className="eyebrow">FLOATING DETAIL · DANBOORU POST #{post.id}</span><h2 id="danbooru-detail-title">{postTitle(post)}</h2></div>
          <span id="danbooru-detail-shortcuts" className="sr-only">A, D 또는 왼쪽, 오른쪽 방향키로 현재 결과의 이전과 다음 post로 이동</span>
          <button type="button" className="icon-button" aria-label="닫기" title="닫기" onClick={onClose}><FluentIcon glyph="\uE711" /></button>
        </header>
        <div className="danbooru-detail-body">
          <div key={`media-${post.id}`} className="danbooru-detail-media">
            {playableVideo ? (
              <video src={post.fileUrl} poster={mediaUrl} controls autoPlay muted preload="auto" playsInline aria-label={`Danbooru post #${post.id} 영상 · 자동재생 · 음소거`} />
            ) : mediaUrl ? <img src={mediaUrl} alt="" width={post.imageWidth} height={post.imageHeight} referrerPolicy="no-referrer" /> : <span className="danbooru-media-missing"><FluentIcon glyph="\uEB9F" /> 표시할 미디어가 없습니다.</span>}
          </div>
          <aside key={`metadata-${post.id}`}>
            <div className="danbooru-detail-facts">
              <span className={`danbooru-rating is-${post.rating}`}>{ratingLabel[post.rating] ?? post.rating}</span>
              <b>{post.imageWidth}×{post.imageHeight}</b><b>{formatBytes(post.fileSize)}</b><b>{post.fileExt.toUpperCase()}</b>
              <span>점수 {post.score}</span><span>즐겨찾기 {post.favoriteCount}</span>
            </div>
            <TagSection title="작가" kind="artist" tags={post.artists} favoriteMetadata={favoriteMetadata} onSearch={onSearch} onFavorite={onMetadataFavorite} />
            <TagSection title="작품" kind="series" tags={post.copyrights} favoriteMetadata={favoriteMetadata} onSearch={onSearch} onFavorite={onMetadataFavorite} />
            <TagSection title="캐릭터" kind="character" tags={post.characters} favoriteMetadata={favoriteMetadata} onSearch={onSearch} onFavorite={onMetadataFavorite} />
            <TagSection title="태그" kind="tag" tags={post.tags.slice(0, 36)} favoriteMetadata={favoriteMetadata} onSearch={onSearch} onFavorite={onMetadataFavorite} />
            <DanbooruRelationsPanel
              currentPostId={post.id}
              related={related}
              loading={relatedLoading}
              error={relatedError}
              onOpen={onOpenRelated}
              onPoolSearch={(poolId) => onSearch(`pool:${poolId}`)}
              onRetry={() => setRelatedRevision((revision) => revision + 1)}
            />
          </aside>
        </div>
        <footer>
          <span>원본은 설정된 다운로드 루트의 Danbooru 폴더에 저장됩니다.</span>
          <button type="button" className={`primary-button${downloaded ? " is-complete" : ""}`} disabled={pending || downloaded || !post.fileUrl} onClick={onDownload}><FluentIcon glyph={downloaded ? "\uE73E" : "\uE896"} /> {downloaded ? "원본 저장 완료" : pending ? "검증하며 저장 중" : "원본 저장"}</button>
        </footer>
      </section>
    </div>
  );
}

function DanbooruRelationsPanel({
  currentPostId,
  related,
  loading,
  error,
  onOpen,
  onPoolSearch,
  onRetry,
}: {
  currentPostId: number;
  related: DanbooruRelatedPosts | null;
  loading: boolean;
  error: string | null;
  onOpen: (post: DanbooruPost) => void;
  onPoolSearch: (poolId: number) => void;
  onRetry: () => void;
}) {
  if (loading) return <div className="danbooru-relations-status" role="status"><span /> 관계를 확인하는 중…</div>;
  if (error) {
    return (
      <div className="danbooru-relations-status is-error">
        <span>{error}</span>
        <button type="button" onClick={onRetry}>다시 시도</button>
      </div>
    );
  }
  if (!related) return null;
  const groups = [
    ...(related.parent ? [{ key: "parent", title: "부모", items: [related.parent] }] : []),
    ...(related.siblings.length ? [{ key: "siblings", title: "같은 부모", items: related.siblings }] : []),
    ...(related.children.length ? [{ key: "children", title: "자식", items: related.children }] : []),
  ];
  if (!related.pools.length && !groups.length) return null;
  return (
    <section className="danbooru-relations" aria-label="연결된 Danbooru post">
      <div className="danbooru-relations-heading">
        <h3>연결된 항목</h3>
        <small>선택하면 이 창에서 이어서 봅니다</small>
      </div>
      {related.pools.map((pool) => (
        <section key={`pool-${pool.id}`} className="danbooru-relation-group is-pool">
          <header>
            <button type="button" onClick={() => onPoolSearch(pool.id)} title={`${formatTag(pool.name)} Pool 전체 검색`}>
              <span>POOL · {pool.category === "series" ? "시리즈" : "컬렉션"}</span>
              <b>{formatTag(pool.name)}</b>
            </button>
            <small>{pool.currentIndex + 1} / {pool.postCount.toLocaleString()}</small>
          </header>
          <RelatedPostStrip items={pool.items} currentPostId={currentPostId} onOpen={onOpen} />
        </section>
      ))}
      {groups.map((group) => (
        <section key={group.key} className="danbooru-relation-group">
          <header><h4>{group.title}</h4><small>{group.items.length}</small></header>
          <RelatedPostStrip items={group.items} currentPostId={currentPostId} onOpen={onOpen} />
        </section>
      ))}
    </section>
  );
}

function RelatedPostStrip({ items, currentPostId, onOpen }: {
  items: DanbooruPost[];
  currentPostId: number;
  onOpen: (post: DanbooruPost) => void;
}) {
  return (
    <div className="danbooru-related-strip">
      {items.map((item) => {
        const mediaUrl = cardMediaUrl(item);
        const current = item.id === currentPostId;
        return (
          <button
            key={item.id}
            type="button"
            className={`danbooru-related-card${current ? " is-current" : ""}`}
            aria-current={current ? "true" : undefined}
            aria-label={`post #${item.id}${current ? " 현재 항목" : " 열기"}`}
            onClick={() => onOpen(item)}
          >
            {mediaUrl ? <img src={mediaUrl} alt="" loading="lazy" decoding="async" referrerPolicy="no-referrer" /> : <FluentIcon glyph="\uEB9F" />}
            <span>#{item.id}</span>
            {current ? <b>현재</b> : null}
          </button>
        );
      })}
    </div>
  );
}

function TagSection({
  title,
  kind,
  tags,
  favoriteMetadata,
  onSearch,
  onFavorite,
}: {
  title: string;
  kind: DanbooruFavoriteKind;
  tags: string[];
  favoriteMetadata: ReadonlySet<string>;
  onSearch: (tag: string) => void;
  onFavorite: (token: string) => void;
}) {
  if (!tags.length) return null;
  return (
    <section className="danbooru-tag-section">
      <h3>{title}</h3>
      <div>{tags.map((tag) => {
        const favoriteToken = danbooruFavoriteToken(kind, tag);
        const favorite = favoriteMetadata.has(favoriteToken);
        const label = formatTag(tag);
        return (
          <button
            key={tag}
            type="button"
            className={favorite ? "is-favorite" : undefined}
            data-favorite-token={favoriteToken}
            aria-label={`${label}${favorite ? ", 즐겨찾기" : ""} · 좌클릭 검색 · 우클릭 즐겨찾기 변경`}
            title={`${label} · 좌클릭 검색 / 우클릭 즐겨찾기`}
            onClick={() => onSearch(tag)}
            onContextMenu={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onFavorite(favoriteToken);
            }}
          >
            <span>{label}</span>
            {favorite ? <span className="danbooru-tag-favorite" aria-hidden="true">★</span> : null}
          </button>
        );
      })}</div>
    </section>
  );
}
