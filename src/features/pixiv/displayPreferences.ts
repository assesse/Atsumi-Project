import { useSyncExternalStore } from "react";

export const PIXIV_SAFE_ONLY_KEY = "atsumi.pixiv.safe-only.v1";
const CHANGED = "atsumi:pixiv-display-preference-changed";

/** Optional local discovery filter, not an account permission or age setting. */
export function loadPixivSafeOnly(): boolean {
  try { return localStorage.getItem(PIXIV_SAFE_ONLY_KEY) === "true"; }
  catch { return false; }
}

export function savePixivSafeOnly(value: boolean): boolean {
  try {
    localStorage.setItem(PIXIV_SAFE_ONLY_KEY, String(value));
    window.dispatchEvent(new Event(CHANGED));
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

export function usePixivSafeOnly(): boolean {
  return useSyncExternalStore(subscribe, loadPixivSafeOnly, () => false);
}
