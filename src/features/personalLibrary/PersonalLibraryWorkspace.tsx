import { useEffect, useRef, useState, type CSSProperties } from "react";
import { GalleryThumbnail } from "../../components/GalleryThumbnail";
import { ViewHeader } from "../../components/ViewHeader";
import { FluentIcon } from "../../components/FluentIcon";
import { splitGalleryTitle } from "../../components/galleryCardLayout";
import { artifactPageThumbnailKey, galleryCoverThumbnailKey, sourcePageThumbnailKey } from "../../thumbnail";
import { galleryFor, targetKey, type Bookmark, type Collection } from "./api";
import { BookmarkButton, BookmarkIcon } from "./BookmarkButton";
import { usePersonalLibrary } from "./PersonalLibraryProvider";

const referenceLabel: Record<Bookmark["reference"]["status"], string> = {
  local: "다운로드 완료", remote: "온라인에서 열기", excluded: "제외·격리된 항목", changed: "저장 당시 페이지와 달라짐", unavailable: "원본 확인 필요",
};
const canOpen = (item: Bookmark) => item.reference.status === "local" || item.reference.status === "remote";
export type SavedItemOpenOptions = { detailOnly?: boolean };

const noop = () => {};
export function PersonalLibraryWorkspace({ previewWidth, pageSize, privacyMode, onPrivacyToggle, onOpen, onBack,
  privacyModePending = false, activityOpen = false, onActivity = noop, onSettings = noop, queueProgress, queueActiveCount }: {
  previewWidth: number; pageSize: number; privacyMode: boolean; onPrivacyToggle(): void; onOpen(item: Bookmark, options?: SavedItemOpenOptions): void; onBack(): void;
  privacyModePending?: boolean; activityOpen?: boolean; onActivity?(): void; onSettings?(): void; queueProgress?: number; queueActiveCount?: number;
}) {
  const library = usePersonalLibrary()!;
  const [kind, setKind] = useState<"albums" | "pages">("albums");
  const [collectionId, setCollectionId] = useState<string | null>(null);
  const [collectionsOpen, setCollectionsOpen] = useState(true);
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<Bookmark[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [opening, setOpening] = useState<string | null>(null);
  const [collectionName, setCollectionName] = useState("");
  const [editing, setEditing] = useState<Collection | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState(false);
  const generation = useRef(0), fetching = useRef(false), openingRef = useRef(false);
  const viewport = useRef<HTMLDivElement>(null);
  const limit = Math.min(100, Math.max(1, pageSize));
  const selected = library.summary?.collections.find((c) => c.id === collectionId);
  useEffect(() => { const timer = setTimeout(() => setQuery(search), 220); return () => clearTimeout(timer); }, [search]);
  useEffect(() => { setEditing(null); setDeleteConfirm(false); }, [collectionId]);
  useEffect(() => {
    if (collectionId && collectionId !== "unfiled" && library.summary && !library.summary.collections.some((c) => c.id === collectionId)) setCollectionId(null);
  }, [library.summary, collectionId]);
  useEffect(() => { viewport.current?.scrollTo?.({ top: 0 }); setNotice(null); }, [kind, collectionId, query]);
  useEffect(() => {
    const token = ++generation.current; let disposed = false;
    if (!library.summary) { setLoading(false); return; }
    fetching.current = true; setLoading(true); setError(null);
    void library.api({ action: "list", kind, collectionId, search: query, offset: 0, limit }).then((result) => {
      if (disposed || token !== generation.current) return;
      setItems(result.items); setTotal(result.total);
    }).catch(() => { if (!disposed && token === generation.current) { setItems([]); setError("목록을 불러오지 못했습니다. 즐겨찾기 기록은 유지됩니다."); } })
      .finally(() => { if (!disposed && token === generation.current) { fetching.current = false; setLoading(false); } });
    return () => { disposed = true; };
  }, [library.api, library.summary?.revision, kind, collectionId, query, limit, refresh]);

  const more = async () => {
    if (fetching.current || items.length >= total) return;
    fetching.current = true; setLoading(true); const token = generation.current;
    try {
      const result = await library.api({ action: "list", kind, collectionId, search: query, offset: items.length, limit });
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
  const filteredKeys = library.summary?.keys.filter((key) => !collectionId || (collectionId === "unfiled" ? key.collectionIds.length === 0 : key.collectionIds.includes(collectionId))) ?? [];
  const availableItems = items.filter(canOpen);
  return <main className="workspace personal-library" data-gallery-shortcuts-suspended aria-label="내 즐겨찾기">
    <ViewHeader view="personal-library" searchFormId="personal-library-search" searchLabel="즐겨찾기 검색"
      search={{ draft: search, committed: query, languages: [], suggestionsOpen: false, activeSuggestion: null }} suggestions={[]}
      onDraft={setSearch} onCommit={(value) => setQuery((value ?? search).trim())} onSuggestions={noop} onSelectSuggestion={noop} onCompleteSuggestion={setSearch} onLanguages={noop} onTagSuggestionQuery={noop}
      filterControl={<button type="button" className={`icon-button${collectionsOpen ? " is-active" : ""}`} title="컬렉션 목록" aria-label="컬렉션 목록" aria-expanded={collectionsOpen} aria-controls="personal-collections" onClick={() => setCollectionsOpen((current) => !current)}><FluentIcon glyph="\uE8B7" /></button>}
      activityCount={0} activityOpen={activityOpen} onActivity={onActivity} queueProgress={queueProgress} queueActiveCount={queueActiveCount}
      privacyMode={privacyMode} privacyModePending={privacyModePending} onPrivacyModeToggle={onPrivacyToggle} onSettings={onSettings}
      randomOpenPending={opening !== null} randomOpenAvailable={!loading && availableItems.length > 0} onRandomOpen={() => { const item = availableItems[Math.floor(Math.random() * availableItems.length)]; if (item) void open(item); }} />
    <section className="page-heading"><div><span className="eyebrow">MY FAVORITES</span><h1>내 즐겨찾기</h1></div>
      <div className="heading-actions"><button type="button" className="text-button" onClick={onBack}><FluentIcon glyph="\uE72B" />탐색으로 돌아가기</button></div></section>
    <div className="context-row personal-library-controls">
      <div className="context-left"><div className="segmented" role="group" aria-label="즐겨찾기 종류"><button className={kind === "albums" ? "is-active" : ""} aria-pressed={kind === "albums"} onClick={() => setKind("albums")}>앨범 {filteredKeys.filter((key) => key.page === 0).length}</button><button className={kind === "pages" ? "is-active" : ""} aria-pressed={kind === "pages"} onClick={() => setKind("pages")}>페이지 {filteredKeys.filter((key) => key.page > 0).length}</button></div>
        <span className="context-summary">{selected?.name ?? (collectionId === "unfiled" ? "미분류" : "전체 즐겨찾기")} · 최근 등록순</span></div>
      <span className="context-summary">{total}개 · 나만 보는 기록</span>
    </div>
    <div className="personal-library-layout">
      <aside id="personal-collections" className="personal-collections" aria-label="내 컬렉션" hidden={!collectionsOpen}>
        <button className={!collectionId ? "is-active" : ""} aria-pressed={!collectionId} onClick={() => setCollectionId(null)}><BookmarkIcon />전체 즐겨찾기 <small>{library.summary?.keys.length ?? 0}</small></button>
        <button className={collectionId === "unfiled" ? "is-active" : ""} aria-pressed={collectionId === "unfiled"} onClick={() => setCollectionId("unfiled")}>미분류 <small>{library.summary?.keys.filter((key) => !key.collectionIds.length).length ?? 0}</small></button>
        <h2>컬렉션</h2>
        <div className="personal-collection-list">{library.summary?.collections.map((collection) => <button key={collection.id} className={collectionId === collection.id ? "is-active" : ""} aria-pressed={collectionId === collection.id} onClick={() => setCollectionId(collection.id)}><span>{collection.name}</span><small>{collection.count}</small></button>)}</div>
        <form onSubmit={async (event) => { event.preventDefault(); const result = await library.mutate({ action: "collection_save", id: null, name: collectionName });
          if (result?.collectionId) { setCollectionId(result.collectionId); setCollectionName(""); }
        }}><input aria-label="컬렉션 이름" placeholder="새 컬렉션" maxLength={60} value={collectionName} onChange={(event) => setCollectionName(event.target.value)} />
          <button disabled={!collectionName.trim() || library.pending || !library.summary}>추가</button></form>
      </aside>
      <section className="personal-library-content">
        {selected ? <div className="personal-collection-manage"><strong>{selected.name}</strong>
          {!editing ? <button onClick={() => { setEditing({ ...selected }); setDeleteConfirm(false); }}>이름 변경</button> : <form onSubmit={async (event) => { event.preventDefault(); if (await library.mutate({ action: "collection_save", id: editing.id, name: editing.name })) setEditing(null); }}>
            <input aria-label="컬렉션 새 이름" maxLength={60} value={editing.name} onChange={(event) => setEditing({ ...editing, name: event.target.value })} /><button disabled={library.pending}>저장</button><button type="button" onClick={() => setEditing(null)}>취소</button></form>}
          {!deleteConfirm ? <button onClick={() => setDeleteConfirm(true)}>컬렉션 삭제</button> : <div role="group" aria-label="컬렉션 삭제 확인"><span>즐겨찾기와 원본은 남습니다.</span><button disabled={library.pending} onClick={async () => { if (await library.mutate({ action: "collection_delete", id: selected.id })) { setCollectionId(null); setDeleteConfirm(false); } }}>컬렉션만 삭제</button><button onClick={() => setDeleteConfirm(false)}>취소</button></div>}
        </div> : null}
        {library.error ? <p role="alert" className="personal-library-error">{library.error} <button onClick={() => void library.reload()}>다시 불러오기</button></p> : null}
        {error ? <p role="alert" className="personal-library-error">{error} <button onClick={() => setRefresh((n) => n + 1)}>다시 불러오기</button></p> : null}
        {notice ? <p role="status" className="personal-library-error">{notice} <button onClick={() => setNotice(null)}>닫기</button></p> : null}
        <div ref={viewport} className="personal-library-viewport" aria-busy={loading}>
          {!library.summary && !library.error ? <p role="status">즐겨찾기를 준비하고 있습니다…</p> : null}
          {!loading && library.summary && !error && !items.length ? <div className="personal-library-empty"><BookmarkIcon /><h2>{query ? "검색 결과가 없습니다" : selected ? "이 컬렉션에 담긴 항목이 없습니다" : kind === "albums" ? "다시 보고 싶은 앨범을 저장해 보세요" : "마음에 든 페이지를 모아 보세요"}</h2><p>{kind === "albums" ? "앨범 카드나 상세보기의 책갈피 버튼으로 저장할 수 있어요." : "PAGE PREVIEW의 책갈피 버튼을 누르면 그 페이지로 다시 돌아올 수 있어요."}</p></div> : null}
          <div className="personal-library-grid" role="list" style={{ "--saved-preview-width": `${previewWidth}px` } as CSSProperties}>
            {items.map((item) => { const gallery = galleryFor(item), available = canOpen(item); return <article key={targetKey(item)} role="listitem" className="saved-card"
              tabIndex={available ? 0 : -1} aria-disabled={!available || undefined}
              aria-label={`${item.snapshot.title}${item.page ? ` ${item.page}페이지` : ""}`}
              title="미리보기 클릭 또는 Enter로 열기 · 우클릭으로 앨범 상세보기"
              onContextMenu={(event) => {
                if ((event.target as Element).closest(".saved-card-bookmark")) return;
                event.preventDefault();
                if (!available) return;
                event.currentTarget.focus({ preventScroll: true });
                void open(item, { detailOnly: true });
              }}
              onDoubleClick={(event) => {
                if ((event.target as Element).closest("button") || event.ctrlKey || event.metaKey || event.shiftKey) return;
                void open(item);
              }}
              onKeyDown={(event) => {
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
  </main>;
}
