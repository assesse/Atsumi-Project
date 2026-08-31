import type { ContentSource } from "../core/types";

const storageKey = "atsumi.content-source.v1";

export const loadContentSource = (): ContentSource => {
  if (typeof window === "undefined") return "hitomi";
  try {
    return window.localStorage.getItem(storageKey) === "danbooru" ? "danbooru" : "hitomi";
  } catch {
    return "hitomi";
  }
};

export const saveContentSource = (source: ContentSource): void => {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(storageKey, source);
  } catch {
    // A blocked preference store must never prevent source switching for this session.
  }
};
