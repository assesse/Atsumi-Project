import { useLayoutEffect, useRef, type ReactNode } from "react";
import type { Gallery, GalleryDisplayMode } from "../core/types";

type ProgressiveGallerySlotProps = {
  gallery: Gallery;
  active: boolean;
  displayMode?: GalleryDisplayMode;
  children: ReactNode;
};

/**
 * Keeps a stable grid cell while the comparatively expensive interactive card
 * is mounted only around the current scroll position.
 */
export function ProgressiveGallerySlot({ gallery, active, displayMode = "detail", children }: ProgressiveGallerySlotProps) {
  const slotRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const slot = slotRef.current;
    if (!active || !slot || document.activeElement !== slot) return;
    slot.querySelector<HTMLElement>(`[data-gallery-id="${Number(gallery.id)}"]`)?.focus({ preventScroll: true });
  }, [active, gallery.id]);

  return (
    <div
      ref={slotRef}
      className={`progressive-gallery-slot${active ? " is-active" : " is-placeholder"}`}
      data-gallery-id={active ? undefined : gallery.id}
      data-progressive-gallery-id={gallery.id}
      tabIndex={active ? undefined : -1}
    >
      {active ? children : (
        <article
          className={`gallery-card progressive-gallery-placeholder${displayMode === "compact" ? " is-compact" : ""}`}
          data-display-mode={displayMode}
          role="listitem"
          aria-label={`${gallery.title}, 화면 가까이에서 상세 내용을 불러옵니다`}
          aria-busy="true"
        >
          <div className="cover progressive-gallery-placeholder-cover" aria-hidden="true">
            <span className="thumbnail-loading" />
            {displayMode === "compact" ? (
              <div className="compact-card-summary">
                <strong title={gallery.title}>{gallery.title}</strong>
                <span title={gallery.artist}>{gallery.artist}</span>
                <small><span>{gallery.pages}p · #{gallery.id}</span></small>
              </div>
            ) : null}
          </div>
          {displayMode === "detail" ? <div className="card-content progressive-gallery-placeholder-content">
            <div className="card-title" title={gallery.title}>
              <strong>{gallery.title}</strong>
            </div>
            <div className="card-byline" title={gallery.artist}>
              <span className="progressive-gallery-placeholder-artist">{gallery.artist}</span>
              {gallery.group ? <span className="progressive-gallery-placeholder-group">· {gallery.group}</span> : null}
            </div>
            <div className="progressive-gallery-placeholder-lines" aria-hidden="true">
              <i />
              <i />
            </div>
            <div className="meta-bottom">
              {gallery.download?.state === "completed" ? <span>완료</span> : null}
              <span>{gallery.pages}p</span>
              <span>#{gallery.id}</span>
            </div>
          </div> : null}
        </article>
      )}
    </div>
  );
}
