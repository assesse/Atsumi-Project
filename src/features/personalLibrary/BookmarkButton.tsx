import type { Gallery } from "../../core/types";
import { targetFor, targetKey } from "./api";
import { usePersonalLibrary } from "./PersonalLibraryProvider";

export function BookmarkIcon({ page }: { page?: number } = {}) {
  return <svg className={`bookmark-icon${page ? " has-page-number" : ""}`} viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" aria-hidden="true"><path d={page ? "M4 4a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v17l-8-4-8 4Z" : "M6 4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v17l-6-4-6 4Z"} />
    {page ? <text x="12" y="10.5" textAnchor="middle" dominantBaseline="central" fill="currentColor" stroke="none" fontSize={page < 10 ? 10 : page < 100 ? 9 : page < 1000 ? 7 : 6} fontWeight="700">{page}</text> : null}
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
