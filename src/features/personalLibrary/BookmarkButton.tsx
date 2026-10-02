import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { Gallery } from "../../core/types";
import { targetFor, targetKey } from "./api";
import { usePersonalLibrary } from "./PersonalLibraryProvider";

export function BookmarkIcon({ page }: { page?: number } = {}) {
  return <svg className={`bookmark-icon${page ? " has-page-number" : ""}`} viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" aria-hidden="true"><path d={page ? "M4 4a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v17l-8-4-8 4Z" : "M6 4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v17l-6-4-6 4Z"} />
    {page ? <text x="12" y="10.5" textAnchor="middle" dominantBaseline="central" fill="currentColor" stroke="none" fontSize={page < 10 ? 10 : page < 100 ? 9 : page < 1000 ? 7 : 6} fontWeight="700">{page}</text> : null}
  </svg>;
}

export function BookmarkButton(props: { gallery: Gallery; page?: number; compact?: boolean; pageLabelInside?: boolean }) {
  // The key closes old popovers when a viewer changes albums/pages.
  return <BookmarkAction key={`${props.gallery.id}:${props.page ?? 0}`} {...props} />;
}
function BookmarkAction({ gallery, page = 0, compact = false, pageLabelInside = false }: { gallery: Gallery; page?: number; compact?: boolean; pageLabelInside?: boolean }) {
  const library = usePersonalLibrary();
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const close = useCallback((focus = false) => { setOpen(false); if (focus) trigger.current?.focus({ preventScroll: true }); }, []);
  if (!library) return null;
  const saved = library.index.has(targetKey(targetFor(gallery, page)));
  const label = `${page ? `${page}페이지` : "앨범"} 즐겨찾기 ${saved ? "해제" : "저장"}`;
  return <span className={`bookmark-actions${compact ? " is-compact" : " is-split"}`} data-saved={saved} onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()} onKeyDown={(event) => event.stopPropagation()}>
    <button type="button" className={`icon-button bookmark-toggle${saved ? " is-saved" : ""}`} aria-label={label} title={library.error && !library.summary ? library.error : label}
      aria-pressed={saved} disabled={library.pending || !library.summary} onClick={() => void library.save(gallery, page, !saved)}><BookmarkIcon page={pageLabelInside ? page : undefined} />{page > 0 && !pageLabelInside ? <small>{page}p</small> : null}</button>
    {!compact ? <button ref={trigger} type="button" className="icon-button bookmark-collections" title="컬렉션에 정리" aria-label={`${page ? `${page}페이지` : "앨범"} 컬렉션에 정리`}
      aria-haspopup="dialog" aria-expanded={open} disabled={library.pending || !library.summary} onClick={() => setOpen((current) => !current)}>
      <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
    </button> : null}
    {open ? <CollectionPicker gallery={gallery} page={page} trigger={trigger} close={close} /> : null}
  </span>;
}

function CollectionPicker({ gallery, page, trigger, close }: { gallery: Gallery; page: number; trigger: RefObject<HTMLButtonElement | null>; close(focus?: boolean): void }) {
  const library = usePersonalLibrary()!;
  const panel = useRef<HTMLDivElement>(null);
  const [name, setName] = useState("");
  const [placement, setPlacement] = useState({ left: 12, top: 12, width: 300, maxHeight: 420 });
  const nativePopover = typeof HTMLElement.prototype.showPopover === "function";
  const target = targetFor(gallery, page), saved = library.index.get(targetKey(target));
  const assign = async (collectionId: string, enabled: boolean) => {
    // Opening the menu never writes. Choosing a destination explicitly saves it.
    if (enabled && !saved && !await library.save(gallery, page, true)) return false;
    return library.membership(target, collectionId, enabled);
  };
  const position = useCallback(() => {
    const anchor = trigger.current?.getBoundingClientRect(); if (!anchor) return;
    const width = Math.min(320, window.innerWidth - 24), maxHeight = Math.min(420, window.innerHeight - 24);
    const height = Math.min(panel.current?.scrollHeight ?? 300, maxHeight);
    setPlacement({ width, maxHeight, left: Math.max(12, Math.min(anchor.right - width, window.innerWidth - width - 12)),
      top: Math.max(12, Math.min(anchor.bottom + 8, window.innerHeight - height - 12)) });
  }, [trigger]);
  useLayoutEffect(() => {
    const node = panel.current; if (!node) return;
    if (nativePopover) node.showPopover(); position(); node.focus({ preventScroll: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(position); observer?.observe(node);
    window.addEventListener("resize", position); window.addEventListener("scroll", position, true);
    return () => { observer?.disconnect(); window.removeEventListener("resize", position); window.removeEventListener("scroll", position, true); if (nativePopover && node.matches(":popover-open")) node.hidePopover(); };
  }, [nativePopover, position]);
  useEffect(() => {
    const outside = (event: Event) => { if (event.target instanceof Node && !panel.current?.contains(event.target) && !trigger.current?.contains(event.target)) close(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape" && !event.isComposing) { event.preventDefault(); event.stopImmediatePropagation(); close(true); } };
    document.addEventListener("pointerdown", outside, true); document.addEventListener("focusin", outside); window.addEventListener("keydown", escape, true);
    return () => { document.removeEventListener("pointerdown", outside, true); document.removeEventListener("focusin", outside); window.removeEventListener("keydown", escape, true); };
  }, [trigger, close]);
  return createPortal(<div ref={panel} role="dialog" aria-label="즐겨찾기 컬렉션" tabIndex={-1} popover={nativePopover ? "manual" : undefined}
    className={`bookmark-picker${nativePopover ? "" : " is-fallback"}`} style={placement} data-gallery-shortcuts-suspended
    onKeyDown={(event) => event.stopPropagation()} onClick={(event) => event.stopPropagation()}>
    <header><strong>컬렉션에 정리</strong><button type="button" aria-label="컬렉션 메뉴 닫기" onClick={() => close(true)}>×</button></header>
    <p>여러 곳에 담아도 원본 파일은 복사되지 않아요.</p>
    <div className="bookmark-picker-list">{library.summary?.collections.map((collection) => <label key={collection.id}>
      <input type="checkbox" checked={saved?.collectionIds.includes(collection.id) ?? false} disabled={library.pending}
        onChange={(event) => void assign(collection.id, event.target.checked)} /><span>{collection.name}</span><small>{collection.count}</small>
    </label>)}{!library.summary?.collections.length ? <p>아직 컬렉션이 없어요. 아래에서 만들어 보세요.</p> : null}</div>
    <form onSubmit={async (event) => { event.preventDefault(); const result = await library.mutate({ action: "collection_save", id: null, name });
      if (result?.collectionId) { if (await assign(result.collectionId, true)) setName(""); }
    }}><input aria-label="새 컬렉션 이름" placeholder="새 컬렉션 이름" value={name} maxLength={60} onChange={(event) => setName(event.target.value)} />
      <button disabled={library.pending || !name.trim()}>만들어 담기</button></form>
    {library.error ? <p role="alert">{library.error}</p> : null}
    <small>{saved ? "내 즐겨찾기에 저장되어 있습니다. 컬렉션은 선택 사항입니다." : "메뉴만 열어서는 저장되지 않습니다. 책갈피를 누르거나 담을 컬렉션을 선택하세요."}</small>
  </div>, trigger.current?.closest("dialog") ?? document.body);
}
