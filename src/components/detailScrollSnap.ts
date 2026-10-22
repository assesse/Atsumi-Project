/** Semantic stops measured in the detail body's own scroll coordinates. */
export type DetailScrollStop = { kind: "top" | "related" | "previews" | "bottom"; top: number };
type Geometry = {
  viewportHeight: number;
  scrollHeight: number;
  paddingTop: number;
  relatedTop?: number;
  relatedCards?: readonly { top: number; height: number }[];
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
  for (const card of geometry.relatedCards ?? []) {
    candidates.push({ kind: "related", top: card.top - geometry.paddingTop });
    if (card.height > geometry.viewportHeight - geometry.paddingTop * 2) {
      candidates.push({ kind: "related", top: card.top + card.height - geometry.viewportHeight + geometry.paddingTop });
    }
  }
  if (geometry.previewTop !== undefined && geometry.previewHeight !== undefined) {
    candidates.push({ kind: "previews", top: geometry.previewTop + (geometry.previewHeight - geometry.viewportHeight) / 2 });
  }
  candidates.push({ kind: "bottom", top: max });
  // Short albums / small windows can put two semantic stops at the same
  // position. Never make a downward wheel gesture scroll upward in that case.
  const ordered = candidates.map((stop) => ({ ...stop, top: Math.max(0, Math.min(max, stop.top)) }))
    .sort((a, b) => a.top - b.top);
  const separation = Math.min(120, Math.max(48, geometry.viewportHeight * .15));
  const stops: DetailScrollStop[] = [];
  for (const stop of ordered) {
    if (!stops.length || stop.top - stops.at(-1)!.top > separation) stops.push(stop);
    else if (stop.kind === "bottom") {
      if (stops.length > 1) stops[stops.length - 1] = stop;
      else if (stop.top > tolerance) stops.push(stop);
    }
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
    relatedCards: [...body.querySelectorAll<HTMLElement>(".related-card")].map((card) => {
      const rect = card.getBoundingClientRect();
      return { top: rect.top - origin + body.scrollTop, height: rect.height };
    }),
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

/** Native wheel distance and momentum stay untouched. Only a nearby forward
 * stop gently settles after the gesture has finished; no hard anchor trap. */
export function attachDetailScrollSnap(body: HTMLElement): () => void {
  let direction = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let settling: number | undefined;
  const stop = () => {
    clearTimeout(timer);
    timer = undefined;
    direction = 0;
    if (settling !== undefined && Math.abs(body.scrollTop - settling) > 1) {
      body.scrollTo({ top: body.scrollTop, behavior: "instant" });
    }
    settling = undefined;
  };
  const schedule = () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      timer = undefined;
      const next = nextDetailScrollStop(measureDetailScrollStops(body), body.scrollTop, direction);
      direction = 0;
      if (!next || Math.abs(next.top - body.scrollTop) > Math.min(96, body.clientHeight * .12)) return;
      settling = next.top;
      body.scrollTo({ top: next.top, behavior: window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" });
    }, 180);
  };
  const wheel = (event: WheelEvent) => {
    if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey
      || !event.deltaY || Math.abs(event.deltaX) > Math.abs(event.deltaY)
      || (event.target instanceof Element && ownsWheel(event.target, body, Math.sign(event.deltaY)))) {
      stop();
      return;
    }
    stop();
    if (body.scrollHeight - body.clientHeight <= tolerance) return;
    direction = Math.sign(event.deltaY);
    schedule();
  };
  const scroll = () => { if (direction) schedule(); };
  const key = (event: KeyboardEvent) => {
    if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "].includes(event.key)) stop();
  };
  body.addEventListener("wheel", wheel, { passive: true });
  body.addEventListener("scroll", scroll, { passive: true });
  body.addEventListener("pointerdown", stop);
  body.addEventListener("keydown", key);
  window.addEventListener("blur", stop);
  window.addEventListener("resize", stop);
  return () => {
    stop();
    body.removeEventListener("wheel", wheel);
    body.removeEventListener("scroll", scroll);
    body.removeEventListener("pointerdown", stop);
    body.removeEventListener("keydown", key);
    window.removeEventListener("blur", stop);
    window.removeEventListener("resize", stop);
  };
}
