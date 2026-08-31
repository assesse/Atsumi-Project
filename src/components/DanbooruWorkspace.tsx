import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { BackendClient } from "../api/backend";
import type {
  DanbooruAutocompleteItem,
  DanbooruDownloadRecord,
  DanbooruDownloadsPage,
  DanbooruPost,
  DanbooruSearchPage,
} from "../api/contracts";
import type { ContentSource, ViewId } from "../core/types";
import { FluentIcon } from "./FluentIcon";
import { SideRail } from "./SideRail";

type DanbooruView = "explore" | "downloads";

type DanbooruWorkspaceProps = {
  backend: BackendClient;
  railCollapsed: boolean;
  pageSize: number;
  previewWidth: number;
  onToggleRail: () => void;
  onSourceChange: (source: ContentSource) => void;
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
};

const stateKey = "atsumi.danbooru-state.v1";
const defaultState: PersistedDanbooruState = {
  view: "explore",
  exploreDraft: "",
  exploreCommitted: "",
  downloadsDraft: "",
  downloadsCommitted: "",
  explorePage: 1,
  downloadsPage: 1,
};

const loadState = (): PersistedDanbooruState => {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(stateKey) ?? "null") as Partial<PersistedDanbooruState> | null;
    if (!parsed) return defaultState;
    return {
      view: parsed.view === "downloads" ? "downloads" : "explore",
      exploreDraft: typeof parsed.exploreDraft === "string" ? parsed.exploreDraft : "",
      exploreCommitted: typeof parsed.exploreCommitted === "string" ? parsed.exploreCommitted : "",
      downloadsDraft: typeof parsed.downloadsDraft === "string" ? parsed.downloadsDraft : "",
      downloadsCommitted: typeof parsed.downloadsCommitted === "string" ? parsed.downloadsCommitted : "",
      explorePage: Number.isInteger(parsed.explorePage) && Number(parsed.explorePage) > 0 ? Number(parsed.explorePage) : 1,
      downloadsPage: Number.isInteger(parsed.downloadsPage) && Number(parsed.downloadsPage) > 0 ? Number(parsed.downloadsPage) : 1,
    };
  } catch {
    return defaultState;
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
  onToggleRail,
  onSourceChange,
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

  const normalizedPageSize = Math.max(10, Math.min(100, pageSize));
  const loadExplore = useCallback(async (tags: string, page: number) => {
    const sequence = ++requestSequence.current;
    setLoading(true);
    setError(null);
    const result = await backend.danbooruSearch({ tags, page, pageSize: normalizedPageSize }).catch(() => null);
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
  }, [backend, normalizedPageSize]);

  const loadDownloads = useCallback(async (page: number, query: string) => {
    const sequence = ++requestSequence.current;
    setLoading(true);
    setError(null);
    const result = await backend.danbooruDownloadsList({ page, pageSize: normalizedPageSize, query }).catch(() => null);
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
  }, [backend, normalizedPageSize]);

  useEffect(() => {
    saveState({
      view,
      exploreDraft,
      exploreCommitted,
      downloadsDraft,
      downloadsCommitted,
      explorePage,
      downloadsPage: downloadsPageNumber,
    });
  }, [downloadsCommitted, downloadsDraft, downloadsPageNumber, exploreCommitted, exploreDraft, explorePage, view]);

  useEffect(() => {
    if (view === "explore") void loadExplore(exploreCommitted, explorePage);
    else void loadDownloads(downloadsPageNumber, downloadsCommitted);
    // Restore the persisted source context once; explicit navigation owns subsequent requests.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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

  const submit = (value = view === "explore" ? exploreDraft : downloadsDraft) => {
    const query = value.trim();
    setSuggestions([]);
    if (view === "explore") {
      setExploreDraft(query);
      setExploreCommitted(query);
      setExplorePage(1);
      void loadExplore(query, 1);
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
      return;
    }
    if (!result.ok) {
      setNotice(result.error.message);
      return;
    }
    setDownloadedIds((current) => new Set(current).add(post.id));
    setNotice(`${result.data.fileName} 원본을 저장했습니다.`);
    if (view === "downloads") void loadDownloads(downloadsPageNumber, downloadsCommitted);
  };

  const recordsById = useMemo(() => new Map(
    downloadsPage?.items.map((record) => [record.post.id, record]) ?? [],
  ), [downloadsPage]);
  const posts = view === "explore"
    ? searchPage?.items ?? []
    : downloadsPage?.items.map((record) => record.post) ?? [];
  const gridWidth = Math.max(170, Math.min(320, previewWidth));

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
            <button type="button" className="icon-button" title="설정" aria-label="설정" onClick={onOpenSettings}><FluentIcon glyph="\uE713" /></button>
          </div>
        </header>

        <section className="danbooru-heading">
          <div>
            <span className="eyebrow">{view === "explore" ? "DANBOORU EXPLORE" : "DANBOORU DOWNLOADS"}</span>
            <h1>{view === "explore" ? "Danbooru post 탐색" : "저장한 Danbooru 원본"}</h1>
            <p>{view === "explore" ? "공개 API · 계정 없이 태그 2개까지 검색" : "다운로드 루트의 Danbooru 인덱스"}</p>
          </div>
          <span className="danbooru-result-count">{view === "downloads" ? `${downloadsPage?.total ?? 0}개 저장` : `${posts.length}개 표시`}</span>
        </section>

        <section className="danbooru-content" aria-busy={loading}>
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
          post={detail}
          downloaded={downloadedIds.has(detail.id)}
          pending={pendingDownloads.has(detail.id)}
          onClose={() => setDetail(null)}
          onDownload={() => void download(detail)}
          onSearch={(tag) => {
            setDetail(null);
            setView("explore");
            setExploreDraft(tag);
            setExploreCommitted(tag);
            setExplorePage(1);
            void loadExplore(tag, 1);
          }}
        />
      ) : null}
      {notice ? <div className="toast" role="status" onAnimationEnd={() => setNotice(null)}>{notice}</div> : null}
    </div>
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
  return (
    <article className="danbooru-card" data-post-id={post.id}>
      <button type="button" className="danbooru-card-preview" onClick={onOpen} aria-label={`${postTitle(post)} 상세 열기`}>
        {post.previewUrl ? <img src={post.previewUrl} alt="" loading="lazy" referrerPolicy="no-referrer" /> : <span className="danbooru-media-missing"><FluentIcon glyph="\uEB9F" /> 미리보기 없음</span>}
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
  post,
  downloaded,
  pending,
  onClose,
  onDownload,
  onSearch,
}: {
  post: DanbooruPost;
  downloaded: boolean;
  pending: boolean;
  onClose: () => void;
  onDownload: () => void;
  onSearch: (tag: string) => void;
}) {
  const mediaUrl = post.largeUrl ?? post.previewUrl;
  useEffect(() => {
    const keyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", keyDown);
    return () => window.removeEventListener("keydown", keyDown);
  }, [onClose]);
  return (
    <div className="modal-backdrop danbooru-detail-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <section className="danbooru-detail" role="dialog" aria-modal="true" aria-labelledby="danbooru-detail-title">
        <header>
          <div><span className="eyebrow">DANBOORU POST #{post.id}</span><h2 id="danbooru-detail-title">{postTitle(post)}</h2></div>
          <button type="button" className="icon-button" aria-label="닫기" title="닫기" onClick={onClose}><FluentIcon glyph="\uE711" /></button>
        </header>
        <div className="danbooru-detail-body">
          <div className="danbooru-detail-media">
            {mediaUrl ? <img src={mediaUrl} alt="" referrerPolicy="no-referrer" /> : <span className="danbooru-media-missing"><FluentIcon glyph="\uEB9F" /> 표시할 미디어가 없습니다.</span>}
          </div>
          <aside>
            <div className="danbooru-detail-facts">
              <span className={`danbooru-rating is-${post.rating}`}>{ratingLabel[post.rating] ?? post.rating}</span>
              <b>{post.imageWidth}×{post.imageHeight}</b><b>{formatBytes(post.fileSize)}</b><b>{post.fileExt.toUpperCase()}</b>
              <span>점수 {post.score}</span><span>즐겨찾기 {post.favoriteCount}</span>
            </div>
            <TagSection title="작가" tags={post.artists} onSearch={onSearch} />
            <TagSection title="작품" tags={post.copyrights} onSearch={onSearch} />
            <TagSection title="캐릭터" tags={post.characters} onSearch={onSearch} />
            <TagSection title="태그" tags={post.tags.slice(0, 36)} onSearch={onSearch} />
            {post.parentId ? <p className="danbooru-relation">부모 post #{post.parentId}</p> : null}
            {post.hasChildren ? <p className="danbooru-relation">연결된 자식 post가 있습니다.</p> : null}
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

function TagSection({ title, tags, onSearch }: { title: string; tags: string[]; onSearch: (tag: string) => void }) {
  if (!tags.length) return null;
  return (
    <section className="danbooru-tag-section">
      <h3>{title}</h3>
      <div>{tags.map((tag) => <button key={tag} type="button" onClick={() => onSearch(tag)}>{formatTag(tag)}</button>)}</div>
    </section>
  );
}
