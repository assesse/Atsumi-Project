import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { Gallery, GalleryId } from "../core/types";
import type { GalleryGroup, GalleryGrouping } from "../state/galleryGrouping";

type GalleryScrollPositionHintProps = {
  rootRef: RefObject<HTMLElement | null>;
  view: "auto-find" | "downloads";
  grouping: GalleryGrouping;
  items: readonly Gallery[];
  groups: readonly GalleryGroup[];
};

type HintCopy = {
  label: string;
  title: string;
  position: string;
};

type HintSnapshot = HintCopy & {
  percent: number;
  right: number;
  top: number;
};

type GroupLocation = {
  group: GalleryGroup;
  index: number;
};

const numberFormatter = new Intl.NumberFormat("ko-KR");
const HIDE_AFTER_SCROLL_MS = 1_200;
const HIDE_AFTER_GUTTER_LEAVE_MS = 240;

const normalizedArtist = (gallery: Gallery): string => gallery.artist.trim() || "작가 정보 없음";

export function galleryScrollHintCopy(
  view: "auto-find" | "downloads",
  grouping: GalleryGrouping,
  orderedItems: readonly Gallery[],
  groups: readonly GalleryGroup[],
  index: number,
  groupLocationById?: ReadonlyMap<GalleryId, GroupLocation>,
): HintCopy | null {
  const gallery = orderedItems[index];
  if (!gallery) return null;
  const overall = `${numberFormatter.format(index + 1)} / ${numberFormatter.format(orderedItems.length)}`;

  if (grouping === "all") {
    return {
      label: `전체 · ${normalizedArtist(gallery)}`,
      title: gallery.title,
      position: overall,
    };
  }

  const knownLocation = groupLocationById?.get(gallery.id);
  const group = knownLocation?.group
    ?? groups.find((candidate) => candidate.items.some((item) => item.id === gallery.id));
  if (!group) {
    return {
      label: grouping === "artist" ? normalizedArtist(gallery) : "기간 정보 없음",
      title: gallery.title,
      position: overall,
    };
  }
  const groupIndex = knownLocation?.index ?? group.items.findIndex((item) => item.id === gallery.id);
  const groupPosition = `${numberFormatter.format(groupIndex + 1)} / ${numberFormatter.format(group.items.length)}`;
  const label = grouping === "artist"
    ? view === "auto-find" ? `즐겨찾기 작가 · ${group.label}` : `작가 · ${group.label}`
    : `기간 · ${group.label}`;
  return {
    label,
    title: gallery.title,
    position: `${groupPosition} · 전체 ${overall}`,
  };
}

const clamp = (value: number, minimum: number, maximum: number): number =>
  Math.min(maximum, Math.max(minimum, value));

const nearestRenderedIndex = (
  root: HTMLElement,
  orderedIndexById: ReadonlyMap<GalleryId, number>,
  groups: readonly GalleryGroup[],
  fallbackIndex: number,
): number => {
  const rootRect = root.getBoundingClientRect();
  const targetY = rootRect.top + Math.min(rootRect.height, root.clientHeight) * 0.34;
  let nearestIndex = fallbackIndex;
  let nearestDistance = Number.POSITIVE_INFINITY;

  if (typeof document.elementFromPoint === "function") {
    const usableWidth = Math.max(1, Math.min(rootRect.width, root.clientWidth));
    const sampleXs = [0.12, 0.5, 0.88].map((ratio) => rootRect.left + usableWidth * ratio);
    const sampleYs = [0, -30, 30, -64, 64].map((offset) => clamp(
      targetY + offset,
      rootRect.top + 2,
      rootRect.bottom - 2,
    ));
    for (const y of sampleYs) {
      for (const x of sampleXs) {
        const element = document.elementFromPoint(x, y);
        if (!(element instanceof Element) || !root.contains(element)) continue;
        const card = element.closest<HTMLElement>("[data-gallery-id]");
        const rawId = Number(card?.dataset.galleryId);
        const cardIndex = Number.isFinite(rawId) ? orderedIndexById.get(rawId as GalleryId) : undefined;
        if (cardIndex !== undefined) return cardIndex;

        const groupSection = element.closest<HTMLElement>(".gallery-group");
        if (!groupSection?.parentElement) continue;
        const groupIndex = [...groupSection.parentElement.children].indexOf(groupSection);
        const firstItem = groups[groupIndex]?.items[0];
        const firstIndex = firstItem ? orderedIndexById.get(firstItem.id) : undefined;
        if (firstIndex !== undefined) return firstIndex;
      }
    }
    return fallbackIndex;
  }

  for (const card of root.querySelectorAll<HTMLElement>("[data-gallery-id]")) {
    const rawId = Number(card.dataset.galleryId);
    if (!Number.isFinite(rawId)) continue;
    const index = orderedIndexById.get(rawId as GalleryId);
    if (index === undefined) continue;
    const rect = card.getBoundingClientRect();
    if (rect.bottom < rootRect.top || rect.top > rootRect.bottom) continue;
    const distance = targetY < rect.top
      ? rect.top - targetY
      : targetY > rect.bottom
        ? targetY - rect.bottom
        : 0;
    if (distance < nearestDistance) {
      nearestDistance = distance;
      nearestIndex = index;
    }
  }

  [...root.querySelectorAll<HTMLElement>(".gallery-group")].forEach((section, groupIndex) => {
    const firstItem = groups[groupIndex]?.items[0];
    const index = firstItem ? orderedIndexById.get(firstItem.id) : undefined;
    const heading = section.querySelector<HTMLElement>(":scope > h2");
    if (index === undefined || !heading) return;
    const rect = heading.getBoundingClientRect();
    if (rect.bottom < rootRect.top || rect.top > rootRect.bottom) return;
    const distance = targetY < rect.top
      ? rect.top - targetY
      : targetY > rect.bottom
        ? targetY - rect.bottom
        : 0;
    if (distance < nearestDistance) {
      nearestDistance = distance;
      nearestIndex = index;
    }
  });
  return nearestIndex;
};

export function GalleryScrollPositionHint({
  rootRef,
  view,
  grouping,
  items,
  groups,
}: GalleryScrollPositionHintProps) {
  const [snapshot, setSnapshot] = useState<HintSnapshot | null>(null);
  const hoveringGutter = useRef(false);
  const draggingGutter = useRef(false);
  const hideTimer = useRef<number | null>(null);
  const updateFrame = useRef<number | null>(null);
  const orderedItems = useMemo(
    () => grouping === "all" ? [...items] : groups.flatMap((group) => group.items),
    [grouping, groups, items],
  );
  const orderedIndexById = useMemo(() => new Map(
    orderedItems.map((gallery, index) => [gallery.id, index]),
  ), [orderedItems]);
  const groupLocationById = useMemo(() => {
    const locations = new Map<GalleryId, GroupLocation>();
    for (const group of groups) {
      group.items.forEach((gallery, index) => locations.set(gallery.id, { group, index }));
    }
    return locations;
  }, [groups]);

  useEffect(() => {
    const root = rootRef.current;
    if (!root || !orderedItems.length) {
      setSnapshot(null);
      return undefined;
    }
    setSnapshot(null);

    const clearHideTimer = () => {
      if (hideTimer.current !== null) window.clearTimeout(hideTimer.current);
      hideTimer.current = null;
    };
    const hide = () => {
      clearHideTimer();
      setSnapshot(null);
    };
    const scheduleHide = (delay: number) => {
      clearHideTimer();
      if (hoveringGutter.current || draggingGutter.current) return;
      hideTimer.current = window.setTimeout(hide, delay);
    };
    const update = () => {
      updateFrame.current = null;
      const maximumScroll = Math.max(0, root.scrollHeight - root.clientHeight);
      if (maximumScroll <= 1) {
        hide();
        return;
      }
      const ratio = clamp(root.scrollTop / maximumScroll, 0, 1);
      const fallbackIndex = Math.round(ratio * Math.max(0, orderedItems.length - 1));
      const index = nearestRenderedIndex(root, orderedIndexById, groups, fallbackIndex);
      const copy = galleryScrollHintCopy(view, grouping, orderedItems, groups, index, groupLocationById);
      if (!copy) {
        hide();
        return;
      }

      const rect = root.getBoundingClientRect();
      const gutterWidth = Math.max(18, root.offsetWidth - root.clientWidth + 8);
      const visibleTop = clamp(rect.top, 8, Math.max(8, window.innerHeight - 8));
      const visibleBottom = clamp(rect.bottom, visibleTop, Math.max(visibleTop, window.innerHeight - 8));
      const thumbPadding = Math.min(42, Math.max(12, (visibleBottom - visibleTop) / 4));
      const top = visibleBottom > visibleTop
        ? visibleTop + thumbPadding + ratio * Math.max(0, visibleBottom - visibleTop - thumbPadding * 2)
        : clamp(window.innerHeight / 2, 42, Math.max(42, window.innerHeight - 42));
      const right = clamp(
        window.innerWidth - rect.right + gutterWidth + 7,
        8,
        Math.max(8, window.innerWidth - 24),
      );
      setSnapshot({ ...copy, percent: Math.round(ratio * 100), right, top });
    };
    const requestUpdate = () => {
      if (updateFrame.current === null) updateFrame.current = window.requestAnimationFrame(update);
    };
    const showDuringScroll = () => {
      requestUpdate();
      scheduleHide(HIDE_AFTER_SCROLL_MS);
    };
    const pointerIsInGutter = (event: PointerEvent): boolean => {
      const rect = root.getBoundingClientRect();
      const gutterWidth = Math.max(20, root.offsetWidth - root.clientWidth + 10);
      return event.clientX >= rect.right - gutterWidth && event.clientX <= rect.right + 2;
    };
    const handlePointerMove = (event: PointerEvent) => {
      const inGutter = pointerIsInGutter(event);
      hoveringGutter.current = inGutter;
      if (inGutter) {
        clearHideTimer();
        requestUpdate();
      } else if (!draggingGutter.current) {
        scheduleHide(HIDE_AFTER_GUTTER_LEAVE_MS);
      }
    };
    const handlePointerDown = (event: PointerEvent) => {
      if (!pointerIsInGutter(event)) return;
      draggingGutter.current = true;
      hoveringGutter.current = true;
      clearHideTimer();
      requestUpdate();
    };
    const handlePointerLeave = () => {
      hoveringGutter.current = false;
      if (!draggingGutter.current) scheduleHide(HIDE_AFTER_GUTTER_LEAVE_MS);
    };
    const handlePointerUp = () => {
      if (!draggingGutter.current) return;
      draggingGutter.current = false;
      hoveringGutter.current = false;
      scheduleHide(HIDE_AFTER_SCROLL_MS);
    };

    root.addEventListener("scroll", showDuringScroll, { passive: true });
    root.addEventListener("wheel", showDuringScroll, { passive: true });
    root.addEventListener("pointermove", handlePointerMove, { passive: true });
    root.addEventListener("pointerdown", handlePointerDown, { passive: true });
    root.addEventListener("pointerleave", handlePointerLeave, { passive: true });
    window.addEventListener("pointerup", handlePointerUp, { passive: true });
    window.addEventListener("resize", requestUpdate, { passive: true });

    return () => {
      root.removeEventListener("scroll", showDuringScroll);
      root.removeEventListener("wheel", showDuringScroll);
      root.removeEventListener("pointermove", handlePointerMove);
      root.removeEventListener("pointerdown", handlePointerDown);
      root.removeEventListener("pointerleave", handlePointerLeave);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("resize", requestUpdate);
      clearHideTimer();
      if (updateFrame.current !== null) window.cancelAnimationFrame(updateFrame.current);
      updateFrame.current = null;
      hoveringGutter.current = false;
      draggingGutter.current = false;
    };
  }, [groupLocationById, grouping, groups, orderedIndexById, orderedItems, rootRef, view]);

  if (!snapshot) return null;
  return createPortal(
    <aside
      className="gallery-scroll-position-hint"
      style={{ right: `${snapshot.right}px`, top: `${snapshot.top}px` }}
      role="status"
      aria-live="polite"
      aria-atomic="true"
      data-view={view}
    >
      <strong>{snapshot.label}</strong>
      <span className="gallery-scroll-position-title">{snapshot.title}</span>
      <span className="gallery-scroll-position-meta">
        <span>{snapshot.position}</span>
        <b>{snapshot.percent}%</b>
      </span>
    </aside>,
    document.body,
  );
}
