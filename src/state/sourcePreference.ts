import { isContentSource, type ContentSource } from "../app/workspaceRegistry";

const storageKey = "atsumi.content-source.v1";

export const loadContentSource = (): ContentSource => {
  if (typeof window === "undefined") return "hitomi";
  try {
    const source = window.localStorage.getItem(storageKey);
    return isContentSource(source) ? source : "hitomi";
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
