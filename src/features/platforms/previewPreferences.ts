import { useSyncExternalStore } from "react";

export const VIDEO_PREVIEW_WIDTHS = [240, 320, 400] as const;
export type VideoPreviewWidth = typeof VIDEO_PREVIEW_WIDTHS[number];
export type VideoPreviewSource = "soop" | "youtube";
const CHANGED = "atsumi:video-preview-preference-changed";
const keyFor = (source: VideoPreviewSource) => `atsumi.${source}.preview-width.v1`;
const isWidth = (value: unknown): value is VideoPreviewWidth => VIDEO_PREVIEW_WIDTHS.some(width => width === value);

export function loadVideoPreviewWidth(source: VideoPreviewSource): VideoPreviewWidth {
  try {
    const value = Number(window.localStorage.getItem(keyFor(source)));
    return isWidth(value) ? value : 320;
  } catch { return 320; }
}

/** Commit first: a failed write must not pretend the preference was saved. */
export function saveVideoPreviewWidth(source: VideoPreviewSource, width: number): boolean {
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

export function useVideoPreviewWidth(source: VideoPreviewSource | null): VideoPreviewWidth {
  return useSyncExternalStore(subscribe, () => source === null ? 320 : loadVideoPreviewWidth(source), () => 320);
}
