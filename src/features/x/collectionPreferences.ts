import { useSyncExternalStore } from "react";

export type XCollection = "likes" | "bookmarks";
export type XCollectionPreference = { cards: boolean; basis: "posts" | "media"; width: number };
export const X_CARD_WIDTH = { min: 160, max: 360, step: 20, default: 220 } as const;
const CHANGED = "atsumi:x-collection-preference-changed";
const WIDTH_KEY = "atsumi.x.collection-card-width.v1";
const storageKey = (feed: XCollection) => "atsumi.x.collection-view.v1." + feed;
const validWidth = (value: unknown): value is number => typeof value === "number" && Number.isInteger(value) && value >= X_CARD_WIDTH.min && value <= X_CARD_WIDTH.max;
function stored(key: string): unknown {
  try { return JSON.parse(localStorage.getItem(key) ?? "null"); }
  catch { return null; }
}
export function loadXCollectionCardWidth(): number {
  const shared = stored(WIDTH_KEY);
  if (validWidth(shared)) return shared;
  // Deterministic legacy fallback: both tabs must agree before the first save.
  // Keep old preferences intact so view modes and media/post bases survive.
  for (const feed of ["likes", "bookmarks"] as const) {
    const legacy = stored(storageKey(feed));
    if (legacy && typeof legacy === "object" && "width" in legacy && validWidth(legacy.width)) return legacy.width;
  }
  return X_CARD_WIDTH.default;
}
export function loadXCollectionPreference(feed: XCollection): XCollectionPreference {
  const width = loadXCollectionCardWidth(), value = stored(storageKey(feed));
  if (value && typeof value === "object" && "cards" in value && "basis" in value
    && typeof value.cards === "boolean" && (value.basis === "posts" || value.basis === "media")) {
    return { cards: value.cards, basis: value.basis, width };
  }
  return { cards: false, basis: "posts", width };
}
export function saveXCollectionPreference(feed: XCollection, next: XCollectionPreference): boolean {
  if (!validWidth(next.width)) return false;
  try {
    localStorage.setItem(storageKey(feed), JSON.stringify({ ...next, width: loadXCollectionCardWidth() }));
    window.dispatchEvent(new CustomEvent(CHANGED, { detail: feed }));
    return true;
  } catch { return false; }
}
export function saveXCollectionCardWidth(width: number): boolean {
  if (!validWidth(width)) return false;
  try {
    localStorage.setItem(WIDTH_KEY, JSON.stringify(width));
    window.dispatchEvent(new Event(CHANGED));
    return true;
  } catch { return false; }
}
function subscribe(callback: () => void) {
  window.addEventListener(CHANGED, callback);
  window.addEventListener("storage", callback);
  return () => { window.removeEventListener(CHANGED, callback); window.removeEventListener("storage", callback); };
}
export function useXCollectionCardWidth(): number {
  return useSyncExternalStore(subscribe, loadXCollectionCardWidth, () => X_CARD_WIDTH.default);
}
