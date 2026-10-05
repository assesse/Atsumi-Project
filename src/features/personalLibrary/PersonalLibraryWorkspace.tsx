import { useEffect, useRef, useState, type CSSProperties } from "react";
import { GalleryThumbnail } from "../../components/GalleryThumbnail";
import { ViewHeader } from "../../components/ViewHeader";
import { splitGalleryTitle } from "../../components/galleryCardLayout";
import { artifactPageThumbnailKey, galleryCoverThumbnailKey, sourcePageThumbnailKey } from "../../thumbnail";
import { galleryFor, targetKey, type Bookmark } from "./api";
import { BookmarkButton, BookmarkIcon } from "./BookmarkButton";
import { usePersonalLibrary } from "./PersonalLibraryProvider";
import { CardContextMenu, useCardContextMenu, type CardMenuItem } from "../../components/CardContextMenu";
import type { GalleryId } from "../../core/types";

const referenceLabel: Record<Bookmark["reference"]["status"], string> = {
  local: "다운로드 완료", remote: "온라인에서 열기", excluded: "제외·격리된 항목", changed: "저장 당시 페이지와 달라짐", unavailable: "원본 확인 필요",
};
const canOpen = (item: Bookmark) => item.reference.status === "local" || item.reference.status === "remote";
export type SavedItemOpenOptions = { detailOnly?: boolean };

const noop = () => {};
export function PersonalLibraryWorkspace({ previewWidth, pageSize, privacyMode, onPrivacyToggle, onOpen,
  privacyModePending = false, activityOpen = false, onActivity = noop, onSettings = noop, queueProgress, queueActiveCount, onQueue, onExclude, onOpenFolder }: {
  previewWidth: number; pageSize: number; privacyMode: boolean; onPrivacyToggle(): void; onOpen(item: Bookmark, options?: SavedItemOpenOptions): void; onBack(): void;
  privacyModePending?: boolean; activityOpen?: boolean; onActivity?(): void; onSettings?(): void; queueProgress?: number; queueActiveCount?: number;
  onQueue?(id: GalleryId): void; onExclude?(id: GalleryId): Promise<void>; onOpenFolder?(entryId: string): void;
}) {
  const library = usePersonalLibrary()!;
  const menu = useCardContextMenu();
  const [menuKey, setMenuKey] = useState<string | null>(null);
  const [kind, setKind] = useState<"albums" | "pages">("albums");
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<Bookmark[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [opening, setOpening] = useState<string | null>(null);
  const generation = useRef(0), fetching = useRef(false), openingRef = useRef(false);
  const viewport = useRef<HTMLDivElement>(null);
  const limit = Math.min(100, Math.max(1, pageSize));
  useEffect(() => { const timer = setTimeout(() => setQuery(search), 220); return () => clearTimeout(timer); }, [search]);
  useEffect(() => { viewport.current?.scrollTo?.({ top: 0 }); setNotice(null); }, [kind, query]);
  useEffect(() => {
    const token = ++generation.current; let disposed = false;
    if (!library.summary) { setLoading(false); return; }
    fetching.current = true; setLoading(true); setError(null);
    // Always query the unified library, including items filed by older versions.
    void library.api({ action: "list", kind, collectionId: null, search: query, offset: 0, limit }).then((result) => {
      if (disposed || token !== generation.current) return;
      setItems(result.items); setTotal(result.total);
    }).catch(() => { if (!disposed && token === generation.current) { setItems([]); setError("목록을 불러오지 못했습니다. 즐겨찾기 기록은 유지됩니다."); } })
      .finally(() => { if (!disposed && token === generation.current) { fetching.current = false; setLoading(false); } });
    return () => { disposed = true; };
  }, [library.api, library.summary?.revision, kind, query, limit, refresh]);

  const more = async () => {
    if (fetching.current || items.length >= total) return;
    fetching.current = true; setLoading(true); const token = generation.current;
    try {
      const result = await library.api({ action: "list", kind, collectionId: null, search: query, offset: items.length, limit });
      if (token === generation.current) { setItems((current) => [...new Map([...current, ...result.items].map((item) => [targetKey(item), item])).values()]); setTotal(result.total); setError(null); }
    } catch { if (token === generation.current) setError("다음 목록을 불러오지 못했습니다. 다시 시도해 주세요."); }
    finally { if (token === generation.current) { fetching.current = false; setLoading(false); } }
  };
  const open = async (item: Bookmark, options?: SavedItemOpenOptions) => {
    if (openingRef.current || !canOpen(item)) return;
    openingRef.current = true; setOpening(targetKey(item)); setError(null); setNotice(null);
    try {
      const result = await library.api({ action: "get", target: { source: item.source, galleryId: item.galleryId, page: item.page } });
      const current = result.items[0];
      if (!current || !canOpen(current)) { setNotice(`${current ? referenceLabel[current.reference.status] : "즐겨찾기가 해제된 항목"}입니다. 원본 상태를 먼저 확인해 주세요.`); setRefresh((n) => n + 1); return; }
      onOpen(current, options);
    } catch { setError("저장한 항목을 확인하지 못했습니다. 다시 시도해 주세요."); }
    finally { openingRef.current = false; setOpening(null); }
  };
  const savedKeys = library.summary?.keys ?? [];
  const availableItems = items.filter(canOpen);
  const menuItem = items.find((item) => targetKey(item) === menuKey);
  const menuItems: CardMenuItem[] = menuItem ? [
    { id: "open", label: menuItem.page ? `${menuItem.page}페이지 열기` : "상세 열기", disabled: !canOpen(menuItem) || opening !== null, action: () => { void open(menuItem); } },
    ...(menuItem.page ? [{ id: "detail", label: "앨범 상세 열기", disabled: !canOpen(menuItem) || opening !== null, action: () => { void open(menuItem, { detailOnly: true }); } }] : []),
    ...(onOpenFolder && menuItem.reference.entryId && menuItem.reference.status === "local" ? [{ id: "folder", label: "저장 폴더 열기", action: () => onOpenFolder(menuItem.reference.entryId!) }] : []),
    ...(onQueue && menuItem.reference.status === "remote" ? [{ id: "download", label: "앨범 다운로드", action: () => onQueue(galleryFor(menuItem).id) }] : []),
    { id: "bookmark", label: `${menuItem.page ? "페이지" : "앨범"} 즐겨찾기 해제`, disabled: library.pending, action: () => { void library.save(galleryFor(menuItem), menuItem.page, false); } },
    ...(onExclude && canOpen(menuItem) ? [{ id: "exclude", label: "앞으로 탐색에서 제외", danger: true, separator: true, action: () => { void onExclude(galleryFor(menuItem).id).then(() => setRefresh((value) => value + 1)); } }] : []),
  ] : [];
  return <main className="workspace personal-library" data-gallery-shortcuts-suspended aria-label="내 즐겨찾기">
    <ViewHeader view="personal-library" searchFormId="personal-library-search" searchLabel="즐겨찾기 검색"
      search={{ draft: search, committed: query, languages: [], suggestionsOpen: false, activeSuggestion: null }} suggestions={[]}
      onDraft={setSearch} onCommit={(value) => setQuery((value ?? search).trim())} onSuggestions={noop} onSelectSuggestion={noop} onCompleteSuggestion={setSearch} onLanguages={noop} onTagSuggestionQuery={noop}
      activityCount={0} activityOpen={activityOpen} onActivity={onActivity} queueProgress={queueProgress} queueActiveCount={queueActiveCount}
      privacyMode={privacyMode} privacyModePending={privacyModePending} onPrivacyModeToggle={onPrivacyToggle} onSettings={onSettings}
      randomOpenPending={opening !== null} randomOpenAvailable={!loading && availableItems.length > 0} onRandomOpen={() => { const item = availableItems[Math.floor(Math.random() * availableItems.length)]; if (item) void open(item); }} />
    <section className="page-heading"><div><span className="eyebrow">MY FAVORITES</span><h1>내 즐겨찾기</h1></div></section>
    <div className="context-row personal-library-controls">
      <div className="context-left"><div className="segmented" role="group" aria-label="즐겨찾기 종류"><button className={kind === "albums" ? "is-active" : ""} aria-pressed={kind === "albums"} onClick={() => setKind("albums")}>앨범 {savedKeys.filter((key) => key.page === 0).length}</button><button className={kind === "pages" ? "is-active" : ""} aria-pressed={kind === "pages"} onClick={() => setKind("pages")}>페이지 {savedKeys.filter((key) => key.page > 0).length}</button></div>
        <span className="context-summary">최근 등록순</span></div>
      <span className="context-summary">{total}개 · 나만 보는 기록</span>
    </div>
    <div className="personal-library-layout">
      <section className="personal-library-content">
        {library.error ? <p role="alert" className="personal-library-error">{library.error} <button onClick={() => void library.reload()}>다시 불러오기</button></p> : null}
        {error ? <p role="alert" className="personal-library-error">{error} <button onClick={() => setRefresh((n) => n + 1)}>다시 불러오기</button></p> : null}
        {notice ? <p role="status" className="personal-library-error">{notice} <button onClick={() => setNotice(null)}>닫기</button></p> : null}
        <div ref={viewport} className="personal-library-viewport" aria-busy={loading}>
          {!library.summary && !library.error ? <p role="status">즐겨찾기를 준비하고 있습니다…</p> : null}
          {!loading && library.summary && !error && !items.length ? <div className="personal-library-empty"><BookmarkIcon /><h2>{query ? "검색 결과가 없습니다" : kind === "albums" ? "다시 보고 싶은 앨범을 저장해 보세요" : "마음에 든 페이지를 모아 보세요"}</h2><p>{kind === "albums" ? "앨범 카드나 상세보기의 책갈피 버튼으로 저장할 수 있어요." : "PAGE PREVIEW의 책갈피 버튼을 누르면 그 페이지로 다시 돌아올 수 있어요."}</p></div> : null}
          <div className="personal-library-grid" role="list" style={{ "--saved-preview-width": `${previewWidth}px` } as CSSProperties}>
            {items.map((item) => { const gallery = galleryFor(item), available = canOpen(item); return <article key={targetKey(item)} role="listitem" className="saved-card"
              tabIndex={available ? 0 : -1} aria-disabled={!available || undefined}
              aria-label={`${item.snapshot.title}${item.page ? ` ${item.page}페이지` : ""}`}
              title="미리보기 클릭 또는 Enter로 열기 · 우클릭으로 메뉴"
              onContextMenu={(event) => {
                if ((event.target as Element).closest(".saved-card-bookmark")) return;
                setMenuKey(targetKey(item)); menu.open(event);
              }}
              onDoubleClick={(event) => {
                if ((event.target as Element).closest("button") || event.ctrlKey || event.metaKey || event.shiftKey) return;
                void open(item);
              }}
              onKeyDown={(event) => {
                if (event.target === event.currentTarget && (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))) { setMenuKey(targetKey(item)); menu.onKeyDown(event); return; }
                if (event.target !== event.currentTarget || event.key !== "Enter" || event.ctrlKey || event.metaKey || event.altKey) return;
                event.preventDefault(); void open(item);
              }}>
              <div className="saved-card-cover">
              <button type="button" className="saved-card-preview" tabIndex={-1} disabled={!available || opening !== null} aria-label={`${item.snapshot.title}${item.page ? ` ${item.page}페이지` : " 상세보기"} 열기`} onClick={(event) => { if (event.detail <= 1) void open(item); }}>
                {available ? <GalleryThumbnail as="span" className="saved-thumbnail" consumer="detail" priority="visible" alt={item.snapshot.title} thumbnailKey={item.page ? item.reference.entryId ? artifactPageThumbnailKey(item.reference.entryId, item.page) : sourcePageThumbnailKey(gallery, item.page) : galleryCoverThumbnailKey(gallery)} /> : <span className="saved-unavailable">{referenceLabel[item.reference.status]}<small>즐겨찾기 기록은 보존됩니다</small></span>}
              </button>
              <div className="saved-card-bookmark"><BookmarkButton gallery={gallery} page={item.page} pageLabelInside /></div>
              <div className="saved-card-overlay-meta"><time dateTime={item.createdAt} title="즐겨찾기 등록일">{new Date(item.createdAt).toLocaleDateString("ko-KR")}</time><span className="saved-page-label">{item.page ? `${item.page}p` : `${item.snapshot.pages}p · 앨범`}</span></div>
              </div>
              <div className="saved-card-info"><strong title={item.snapshot.title}>{splitGalleryTitle(item.snapshot.title).primary}</strong><span title={item.snapshot.artists.join(", ") || item.snapshot.artist}>{item.snapshot.artists.join(", ") || item.snapshot.artist}</span><small>#{item.galleryId} · {referenceLabel[item.reference.status]}</small></div>
            </article>; })}
          </div>
          {loading ? <p role="status">불러오는 중…</p> : null}
          {items.length < total ? <button className="personal-library-more" disabled={loading} onClick={() => void more()}>더 보기 · {items.length}/{total}</button> : null}
        </div>
      </section>
    </div>
    <CardContextMenu anchor={menuItem ? menu.anchor : null} close={menu.close} label={menuItem?.snapshot.title ?? "즐겨찾기"} items={menuItems} />
  </main>;
}
