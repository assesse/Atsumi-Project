import { useSyncExternalStore } from "react";

export type XCollection = "likes" | "bookmarks";
export type XCollectionPreference = { cards: boolean; basis: "posts" | "media"; width: number };
export const X_CARD_WIDTH = { min: 160, max: 360, step: 20, default: 220 } as const;
const CHANGED = "atsumi:x-collection-preference-changed";
const storageKey = (feed: XCollection) => "atsumi.x.collection-view.v1." + feed;
const validWidth = (value: unknown): value is number => typeof value === "number" && Number.isInteger(value) && value >= X_CARD_WIDTH.min && value <= X_CARD_WIDTH.max;

export function loadXCollectionPreference(feed: XCollection): XCollectionPreference {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(storageKey(feed)) ?? "null");
    if (value && typeof value === "object" && "cards" in value && "basis" in value
      && typeof value.cards === "boolean" && (value.basis === "posts" || value.basis === "media")) {
      return { cards: value.cards, basis: value.basis, width: "width" in value && validWidth(value.width) ? value.width : X_CARD_WIDTH.default };
    }
  } catch { /* Presentation preferences must not block browsing. */ }
  return { cards: false, basis: "posts", width: X_CARD_WIDTH.default };
}

export function saveXCollectionPreference(feed: XCollection, next: XCollectionPreference): boolean {
  if (!validWidth(next.width)) return false;
  try {
    localStorage.setItem(storageKey(feed), JSON.stringify(next));
    window.dispatchEvent(new CustomEvent(CHANGED, { detail: feed }));
    return true;
  } catch { return false; }
}
export function saveXCollectionCardWidth(feed: XCollection, width: number): boolean {
  return saveXCollectionPreference(feed, { ...loadXCollectionPreference(feed), width });
}
function subscribe(callback: () => void) {
  window.addEventListener(CHANGED, callback);
  window.addEventListener("storage", callback);
  return () => { window.removeEventListener(CHANGED, callback); window.removeEventListener("storage", callback); };
}
export function useXCollectionCardWidth(feed: XCollection): number {
  return useSyncExternalStore(subscribe, () => loadXCollectionPreference(feed).width, () => X_CARD_WIDTH.default);
}
