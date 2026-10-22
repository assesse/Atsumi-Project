import { useSyncExternalStore } from "react";

export const IMAGE_PREVIEW_WIDTHS = [140, 180, 220, 280, 340] as const;
export type ImagePreviewWidth = typeof IMAGE_PREVIEW_WIDTHS[number];
export type ImagePreviewSource = "pinterest" | "pixiv";
const CHANGED = "atsumi:image-preview-preference-changed";
const keyFor = (source: ImagePreviewSource) => `atsumi.${source}.preview-width.v1`;
const isWidth = (value: unknown): value is ImagePreviewWidth => IMAGE_PREVIEW_WIDTHS.some(width => width === value);

export function normalizeImagePreviewWidth(value: unknown): ImagePreviewWidth {
  return isWidth(value) ? value : 180;
}

export function loadImagePreviewWidth(source: ImagePreviewSource): ImagePreviewWidth {
  try { return normalizeImagePreviewWidth(Number(window.localStorage.getItem(keyFor(source)))); }
  catch { return 180; }
}

/** A failed save retains the last committed preference and emits no success. */
export function saveImagePreviewWidth(source: ImagePreviewSource, width: number): boolean {
  if (!isWidth(width)) return false;
  try {
    window.localStorage.setItem(keyFor(source), String(width));
    window.dispatchEvent(new CustomEvent(CHANGED, { detail: source }));
    return true;
  } catch { return false; }
}

function subscribe(callback: () => void) {
  window.addEventListener(CHANGED, callback);
  window.addEventListener("storage", callback);
  return () => {
    window.removeEventListener(CHANGED, callback);
    window.removeEventListener("storage", callback);
  };
}

export function useImagePreviewWidth(source: ImagePreviewSource | null): ImagePreviewWidth {
  return useSyncExternalStore(subscribe, () => source === null ? 180 : loadImagePreviewWidth(source), () => 180);
}
