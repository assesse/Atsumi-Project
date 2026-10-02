/** Semantic stops measured in the detail body's own scroll coordinates. */
export type DetailScrollStop = { kind: "top" | "related" | "previews" | "bottom"; top: number };
type Geometry = {
  viewportHeight: number;
  scrollHeight: number;
  paddingTop: number;
  relatedTop?: number;
  previewTop?: number;
  previewHeight?: number;
};
const tolerance = 8;

export function detailScrollStops(geometry: Geometry): DetailScrollStop[] {
  const max = Math.max(0, geometry.scrollHeight - geometry.viewportHeight);
  const candidates: DetailScrollStop[] = [{ kind: "top", top: 0 }];
  if (geometry.relatedTop !== undefined) {
    candidates.push({ kind: "related", top: geometry.relatedTop - geometry.paddingTop });
  }
  if (geometry.previewTop !== undefined && geometry.previewHeight !== undefined) {
    candidates.push({ kind: "previews", top: geometry.previewTop + (geometry.previewHeight - geometry.viewportHeight) / 2 });
  }
  candidates.push({ kind: "bottom", top: max });
  // Short albums / small windows can put two semantic stops at the same
  // position. Never make a downward wheel gesture scroll upward in that case.
  const ordered = candidates.map((stop) => ({ ...stop, top: Math.max(0, Math.min(max, stop.top)) }))
    .sort((a, b) => a.top - b.top);
  const stops: DetailScrollStop[] = [];
  for (const stop of ordered) {
    if (!stops.length || stop.top - stops.at(-1)!.top > tolerance) stops.push(stop);
    else if (stop.kind === "bottom" && stops.length > 1) stops[stops.length - 1] = stop;
  }
  return stops;
}

export function nextDetailScrollStop(stops: readonly DetailScrollStop[], current: number, direction: number): DetailScrollStop | undefined {
  return direction > 0
    ? stops.find((stop) => stop.top > current + tolerance)
    : [...stops].reverse().find((stop) => stop.top < current - tolerance);
}

export function measureDetailScrollStops(body: HTMLElement): DetailScrollStop[] {
  const origin = body.getBoundingClientRect().top + body.clientTop;
  const related = body.querySelector<HTMLElement>(".related-section:has(.related-card)");
  const previews = body.querySelector<HTMLElement>(".preview-window:has(.preview-thumb)");
  const relatedRect = related?.getBoundingClientRect();
  const previewRect = previews?.getBoundingClientRect();
  return detailScrollStops({
    viewportHeight: body.clientHeight,
    scrollHeight: body.scrollHeight,
    paddingTop: Number.parseFloat(getComputedStyle(body).paddingTop) || 0,
    relatedTop: relatedRect && relatedRect.top - origin + body.scrollTop,
    previewTop: previewRect && previewRect.top - origin + body.scrollTop,
    previewHeight: previewRect?.height,
  });
}

function ownsWheel(target: Element, body: HTMLElement, direction: number): boolean {
  if (target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [data-gallery-shortcuts-suspended]')) return true;
  // An independently scrollable editor/list must still get its own wheel.
  for (let node: Element | null = target; node && node !== body; node = node.parentElement) {
    if (!(node instanceof HTMLElement) || node.scrollHeight <= node.clientHeight + 1) continue;
    if (!/auto|scroll/.test(getComputedStyle(node).overflowY)) continue;
    if (direction < 0 ? node.scrollTop > 0 : node.scrollTop + node.clientHeight < node.scrollHeight - 1) return true;
  }
  return false;
}

/** One mouse-wheel / trackpad gesture advances one stop. Native scrollbar,
 * touch, and keyboard scrolling remain free; no document-wide wheel capture. */
export function attachDetailScrollSnap(body: HTMLElement): () => void {
  let gesture: { direction: number; lastAt: number; startedAt: number; top: number } | undefined;
  let pendingDelta = 0;
  let deltaAt = -Infinity;
  const stop = () => {
    if (gesture && Math.abs(body.scrollTop - gesture.top) > 1) {
      body.scrollTo({ top: body.scrollTop, behavior: "instant" });
    }
    gesture = undefined;
    pendingDelta = 0;
  };
  const wheel = (event: WheelEvent) => {
    if (event.defaultPrevented || !event.cancelable || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey
      || !event.deltaY || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
    const direction = Math.sign(event.deltaY);
    if (event.target instanceof Element && ownsWheel(event.target, body, direction)) return;
    if (body.scrollHeight - body.clientHeight <= tolerance) return;
    event.preventDefault();
    const now = performance.now();
    if (gesture && gesture.direction === direction) {
      const sameGesture = now - gesture.lastAt < 180;
      const stillMoving = now - gesture.startedAt < 900 && Math.abs(body.scrollTop - gesture.top) > 1;
      if (sameGesture || stillMoving) {
        gesture.lastAt = now;
        return;
      }
      gesture = undefined;
    }
    if (gesture && gesture.direction !== direction) stop();
    if (now - deltaAt > 180 || Math.sign(pendingDelta) !== direction) pendingDelta = 0;
    deltaAt = now;
    pendingDelta += event.deltaY * (event.deltaMode === 1 ? 32 : event.deltaMode === 2 ? body.clientHeight : 1);
    if (Math.abs(pendingDelta) < 8) return;
    pendingDelta = 0;
    // Only measure layout when advancing, not for every trackpad momentum event.
    const stops = measureDetailScrollStops(body);
    const next = nextDetailScrollStop(stops, body.scrollTop, direction);
    if (!next) return;
    gesture = { direction, lastAt: now, startedAt: now, top: next.top };
    body.scrollTo({ top: next.top, behavior: window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" });
  };
  const key = (event: KeyboardEvent) => {
    if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "].includes(event.key)) stop();
  };
  body.addEventListener("wheel", wheel, { passive: false });
  body.addEventListener("pointerdown", stop);
  body.addEventListener("keydown", key);
  window.addEventListener("blur", stop);
  window.addEventListener("resize", stop);
  return () => {
    stop();
    body.removeEventListener("wheel", wheel);
    body.removeEventListener("pointerdown", stop);
    body.removeEventListener("keydown", key);
    window.removeEventListener("blur", stop);
    window.removeEventListener("resize", stop);
  };
}
