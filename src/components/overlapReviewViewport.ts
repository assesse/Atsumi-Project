import { useEffect, type RefObject } from "react";

export const overlapReviewSizeKey = "atsumi.hitomi.overlapReview.size.v1";
type DialogSize = { width: number; height: number };
const validSize = (value: unknown): value is DialogSize => {
  if (!value || typeof value !== "object") return false;
  const size = value as Partial<DialogSize>;
  return typeof size.width === "number" && typeof size.height === "number"
    && Number.isFinite(size.width) && Number.isFinite(size.height)
    && size.width >= 240 && size.height >= 180 && size.width <= 10000 && size.height <= 10000;
};
const storedSize = (): DialogSize | null => {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(overlapReviewSizeKey) ?? "null");
    return validSize(value) ? value : null;
  } catch { return null; }
};

/** Native CSS resizing writes inline px dimensions. Remember those dimensions,
 * not a maximized or viewport-clamped bounding rectangle. */
export function useOverlapReviewSize(ref: RefObject<HTMLDialogElement | null>, enabled: boolean): void {
  useEffect(() => {
    const element = ref.current;
    if (!enabled || !element) return;
    const originalWidth = element.style.width;
    const originalHeight = element.style.height;
    const restored = storedSize();
    if (restored) {
      element.style.width = restored.width + "px";
      element.style.height = restored.height + "px";
    }
    let timer: number | undefined;
    let pending: DialogSize | null = null;
    const persist = () => {
      if (!pending) return;
      try { localStorage.setItem(overlapReviewSizeKey, JSON.stringify(pending)); } catch { /* Optional preference. */ }
      pending = null;
    };
    const remember = () => {
      // An untouched CSS default has no inline dimensions and is not a user resize.
      if (!/^\d+(\.\d+)?px$/.test(element.style.width) || !/^\d+(\.\d+)?px$/.test(element.style.height)) return;
      const size = { width: Math.round(parseFloat(element.style.width)), height: Math.round(parseFloat(element.style.height)) };
      if (!validSize(size)) return;
      pending = size;
      window.clearTimeout(timer);
      timer = window.setTimeout(persist, 200);
    };
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(remember) : null;
    observer?.observe(element);
    return () => {
      observer?.disconnect();
      remember();
      window.clearTimeout(timer);
      persist();
      // Pair/history views share this dialog but do not own this preference.
      element.style.width = originalWidth;
      element.style.height = originalHeight;
    };
  }, [enabled, ref]);
}

type WheelInput = Pick<WheelEvent, "deltaX" | "deltaY" | "deltaMode" | "shiftKey" | "ctrlKey" | "metaKey" | "preventDefault" | "stopPropagation">;
const consumeVertical = (element: HTMLElement, delta: number): number => {
  const before = element.scrollTop;
  element.scrollTop = Math.max(0, Math.min(Math.max(0, element.scrollHeight - element.clientHeight), before + delta));
  return delta - (element.scrollTop - before);
};

/** Keep wheel axes explicit, and pass unused vertical distance to the dialog. */
export function routeOverlapWheel(event: WheelInput, viewport: HTMLElement): boolean {
  if (event.ctrlKey || event.metaKey) return false;
  const delta = event.shiftKey ? event.deltaX || event.deltaY : event.deltaY || event.deltaX;
  if (!delta) return false;
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2
    ? event.shiftKey ? viewport.clientWidth : viewport.clientHeight : 1;
  event.preventDefault();
  event.stopPropagation();
  if (event.shiftKey) {
    viewport.scrollLeft = Math.max(0, Math.min(Math.max(0, viewport.scrollWidth - viewport.clientWidth), viewport.scrollLeft + delta * unit));
  } else {
    const remaining = consumeVertical(viewport, delta * unit);
    const outer = viewport.closest<HTMLElement>(".review-scroll");
    if (remaining && outer && outer !== viewport) consumeVertical(outer, remaining);
  }
  return true;
}
