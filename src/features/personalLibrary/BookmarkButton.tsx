import type { Gallery } from "../../core/types";
import { targetFor, targetKey } from "./api";
import { usePersonalLibrary } from "./PersonalLibraryProvider";

export function BookmarkIcon({ page }: { page?: number } = {}) {
  if(page)return <span className="bookmark-page-icon" aria-hidden="true"><BookmarkIcon/><span>{page}p</span></span>;
  return <svg className="bookmark-icon" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" aria-hidden="true"><path d="M6 4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v17l-6-4-6 4Z" />
  </svg>;
}

export function BookmarkButton(props: { gallery: Gallery; page?: number; compact?: boolean; pageLabelInside?: boolean }) {
  return <BookmarkAction key={`${props.gallery.id}:${props.page ?? 0}`} {...props} />;
}
function BookmarkAction({ gallery, page = 0, compact = false, pageLabelInside = false }: { gallery: Gallery; page?: number; compact?: boolean; pageLabelInside?: boolean }) {
  const library = usePersonalLibrary();
  if (!library) return null;
  const saved = library.index.has(targetKey(targetFor(gallery, page)));
  const label = `${page ? `${page}페이지` : "앨범"} 즐겨찾기 ${saved ? "해제" : "저장"}`;
  return <span className={`bookmark-actions${compact ? " is-compact" : ""}`} data-saved={saved} onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()} onKeyDown={(event) => event.stopPropagation()}>
    <button type="button" className={`icon-button bookmark-toggle${saved ? " is-saved" : ""}`} aria-label={label} title={library.error && !library.summary ? library.error : label}
      aria-pressed={saved} disabled={library.pending || !library.summary} onClick={() => void library.save(gallery, page, !saved)}><BookmarkIcon page={pageLabelInside ? page : undefined} />{page > 0 && !pageLabelInside ? <small>{page}p</small> : null}</button>
  </span>;
}
