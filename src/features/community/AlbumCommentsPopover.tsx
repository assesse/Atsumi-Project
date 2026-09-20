import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { AlbumCommentsState } from "./useAlbumComments";
import "./AlbumCommentsPopover.css";

export function AlbumCommentsPopover({ id, trigger, state, onClose }: {
  id: string; trigger: RefObject<HTMLButtonElement | null>; state: AlbumCommentsState; onClose(restoreFocus?: boolean): void;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState({ left: 12, top: 12, width: 360, maxHeight: 480, up: false });
  const nativePopover = typeof HTMLElement.prototype.showPopover === "function";
  const host = trigger.current?.closest("dialog") ?? document.body;
  const position = useCallback(() => {
    const anchor = trigger.current?.getBoundingClientRect();
    if (!anchor) return;
    const viewport = window.visualViewport;
    const x = viewport?.offsetLeft ?? 0, y = viewport?.offsetTop ?? 0;
    const width = viewport?.width ?? window.innerWidth, height = viewport?.height ?? window.innerHeight;
    const padding = 12, gap = 8, panelWidth = Math.min(360, Math.max(180, width - padding * 2));
    const below = y + height - anchor.bottom - padding - gap, above = anchor.top - y - padding - gap;
    const up = below < 280 && above > below;
    const maxHeight = Math.max(100, Math.min(480, height - padding * 2, up ? above : below));
    const panelHeight = Math.min(panel.current?.scrollHeight || 420, maxHeight);
    setPlacement({ width: panelWidth, maxHeight, up,
      left: Math.max(x + padding, Math.min(anchor.right - panelWidth, x + width - panelWidth - padding)),
      top: Math.max(y + padding, Math.min(up ? anchor.top - gap - panelHeight : anchor.bottom + gap, y + height - panelHeight - padding)) });
  }, [trigger]);
  useLayoutEffect(() => {
    const node = panel.current;
    if (!node) return;
    if (nativePopover) node.showPopover();
    position(); node.focus({ preventScroll: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(position);
    observer?.observe(node); if (trigger.current) observer?.observe(trigger.current);
    window.addEventListener("resize", position); window.addEventListener("scroll", position, true);
    window.visualViewport?.addEventListener("resize", position);
    return () => {
      observer?.disconnect(); window.removeEventListener("resize", position); window.removeEventListener("scroll", position, true);
      window.visualViewport?.removeEventListener("resize", position);
      if (nativePopover && node.matches(":popover-open")) node.hidePopover();
    };
  }, [nativePopover, position, trigger]);
  useEffect(() => {
    const outside = (event: Event) => {
      const target = event.target;
      if (target instanceof Node && !panel.current?.contains(target) && !trigger.current?.contains(target)) onClose(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing) return;
      event.preventDefault(); event.stopImmediatePropagation(); onClose(true);
    };
    document.addEventListener("pointerdown", outside, true); document.addEventListener("focusin", outside);
    window.addEventListener("keydown", escape, true);
    return () => { document.removeEventListener("pointerdown", outside, true); document.removeEventListener("focusin", outside); window.removeEventListener("keydown", escape, true); };
  }, [onClose, trigger]);
  const { page, loading, writer, draft, saving, preparing } = state;
  return createPortal(<div ref={panel} id={id} role="dialog" aria-label="앨범 코멘트" tabIndex={-1}
    popover={nativePopover ? "manual" : undefined} className={`album-comments-panel${nativePopover ? "" : " is-fallback"}`}
    data-gallery-shortcuts-suspended data-placement={placement.up ? "above" : "below"}
    style={{ left: placement.left, top: placement.top, width: placement.width, maxHeight: placement.maxHeight }}
    onKeyDown={(event) => event.stopPropagation()} onPointerDown={(event) => event.stopPropagation()}>
    <header className="album-comments-heading"><h3>코멘트{page.summary ? <span> {page.summary.reviewCount}</span> : null}</h3>
      {page.summary?.averageRating != null ? <span className="album-comments-average">★ {page.summary.averageRating.toFixed(1)}</span> : null}
      <button type="button" className="album-comments-close" aria-label="코멘트 닫기" onClick={() => onClose(true)}>×</button>
    </header>
    <div className="album-comments-list" aria-label="이 앨범의 코멘트" aria-busy={loading}>
      {loading && !page.items.length ? <p className="album-comments-muted" role="status">코멘트를 불러오는 중…</p> : null}
      {!loading && !state.readError && !page.items.length ? <p className="album-comments-empty">아직 코멘트가 없어요.<br /><span>첫 별점을 남겨보세요.</span></p> : null}
      {page.items.slice(0, state.shown).map((review) => <article key={review.id} className="album-comment">
        <div><span className="album-comment-nickname">{review.nickname}</span><span className="album-comment-stars" aria-label={`별점 ${review.rating}점`}>{"★".repeat(review.rating)}<span>{"☆".repeat(5 - review.rating)}</span></span></div>
        <p>{review.comment || "별점을 남겼어요."}</p>
      </article>)}
      {state.readError ? <p className="album-comments-error" role="alert">{state.readError} <button type="button" onClick={() => void state.reload()} disabled={loading}>다시 불러오기</button></p> : null}
      {page.items.length > state.shown || page.nextCursor ? <button type="button" className="album-comments-more" disabled={loading} onClick={state.more}>{loading ? "불러오는 중…" : "코멘트 더 보기"}</button> : null}
    </div>
    <form className="album-comments-compose" onSubmit={(event) => { event.preventDefault(); void state.submit(); }}>
      <fieldset className="album-comments-rating" disabled={saving}><legend>내 별점</legend>
        {[1, 2, 3, 4, 5].map((rating) => <button key={rating} type="button" aria-label={`${rating}점`} aria-pressed={draft.rating === rating}
          className={rating <= draft.rating ? "is-filled" : ""} onClick={() => state.setRating(rating)}>★</button>)}
        <span aria-live="polite">{draft.rating ? `${draft.rating}점` : "선택"}</span>
      </fieldset>
      <textarea aria-label="앨범에 한마디" placeholder="보고 느낀 점을 한마디…" rows={2} maxLength={500} disabled={saving}
        value={draft.comment} onFocus={state.beginWriting} onChange={(event) => state.setComment(event.target.value)} />
      <div className="album-comments-compose-footer"><span title={writer ? `작성자: ${writer.profile.nickname}` : undefined}>
        {preparing ? "작성 준비 중…" : writer ? writer.profile.nickname : "첫 작성 시 익명 키 발급"}</span>
        <button type="submit" className="album-comments-submit" disabled={saving || preparing || !writer || !draft.rating}>
          {saving ? "저장 중…" : writer?.mine || state.saved ? "수정" : "남기기"}</button>
      </div>
      <p className="album-comments-disclosure">별점만 남겨도 좋아요 · 작성한 내용은 공개됩니다.</p>
      {writer?.mine?.hidden ? <p className="album-comments-error">운영자가 숨긴 코멘트입니다. 수정해도 공개되지 않습니다.</p> : null}
      {state.writeError ? <p className="album-comments-error" role="alert">{state.writeError}{!writer ? <button type="button" disabled={preparing} onClick={state.beginWriting}>다시 시도</button> : null}</p> : null}
      {state.notice ? <p className="album-comments-notice" role="status">{state.notice}</p> : null}
    </form>
  </div>, host);
}
