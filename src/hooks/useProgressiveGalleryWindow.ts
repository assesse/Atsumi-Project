import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { galleryId, type GalleryId } from "../core/types";

type ProgressiveGalleryWindowOptions = {
  rootRef: RefObject<HTMLElement | null>;
  enabled: boolean;
  observeKey: unknown;
  onEnter?: (ids: readonly GalleryId[]) => void;
  onLeave?: (ids: readonly GalleryId[]) => void;
  overscanPixels?: number;
  /** Keep every slot that has entered the window mounted until this hook unmounts. */
  retainEntered?: boolean;
};

const EMPTY_IDS: ReadonlySet<GalleryId> = new Set<GalleryId>();

export function useProgressiveGalleryWindow({
  rootRef,
  enabled,
  observeKey,
  onEnter,
  onLeave,
  overscanPixels = 1200,
  retainEntered = false,
}: ProgressiveGalleryWindowOptions): ReadonlySet<GalleryId> {
  const [activeIds, setActiveIds] = useState<ReadonlySet<GalleryId>>(EMPTY_IDS);
  const activeIdsRef = useRef<ReadonlySet<GalleryId>>(EMPTY_IDS);
  const onEnterRef = useRef(onEnter);
  const onLeaveRef = useRef(onLeave);

  useEffect(() => {
    onEnterRef.current = onEnter;
    onLeaveRef.current = onLeave;
  }, [onEnter, onLeave]);

  useLayoutEffect(() => {
    if (!enabled) {
      if (retainEntered) return undefined;
      activeIdsRef.current = EMPTY_IDS;
      setActiveIds((current) => current.size ? EMPTY_IDS : current);
      return undefined;
    }

    const root = rootRef.current;
    if (!root) return undefined;
    const elements = [...root.querySelectorAll<HTMLElement>("[data-progressive-gallery-id]")];
    if (!elements.length) {
      if (retainEntered) return undefined;
      activeIdsRef.current = EMPTY_IDS;
      setActiveIds((current) => current.size ? EMPTY_IDS : current);
      return undefined;
    }

    const idFor = (element: Element): GalleryId | null => {
      const value = Number((element as HTMLElement).dataset.progressiveGalleryId);
      return Number.isSafeInteger(value) && value > 0 ? galleryId(value) : null;
    };

    if (typeof IntersectionObserver === "undefined") {
      const ids = elements.flatMap((element) => {
        const id = idFor(element);
        return id === null ? [] : [id];
      });
      const next = new Set(ids);
      activeIdsRef.current = next;
      setActiveIds(next);
      onEnterRef.current?.(ids);
      return undefined;
    }

    const observer = new IntersectionObserver((entries) => {
      const entered: GalleryId[] = [];
      const left: GalleryId[] = [];
      const next = new Set(activeIdsRef.current);
      let changed = false;
      for (const entry of entries) {
        const id = idFor(entry.target);
        if (id === null) continue;
        if (entry.isIntersecting) {
          const wasAdded = !next.has(id);
          if (wasAdded) {
            next.add(id);
            changed = true;
          }
          if (retainEntered || wasAdded) entered.push(id);
        } else if (retainEntered) {
          if (next.has(id)) left.push(id);
        } else if (next.delete(id)) {
          left.push(id);
          changed = true;
        }
      }
      if (changed) {
        activeIdsRef.current = next;
        setActiveIds(next);
      }
      if (left.length) onLeaveRef.current?.(left);
      if (entered.length) onEnterRef.current?.(entered);
    }, {
      root,
      rootMargin: `${Math.max(0, Math.round(overscanPixels))}px 0px`,
      threshold: 0,
    });

    elements.forEach((element) => observer.observe(element));
    return () => observer.disconnect();
  }, [enabled, observeKey, overscanPixels, retainEntered, rootRef]);

  return activeIds;
}
