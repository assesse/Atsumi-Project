import type { ContentSource } from "../app/workspaceRegistry";

// The old combined guide's marker belongs to Hitomi/common only. Other services
// start their own guide on first entry, without replaying Hitomi for existing users.
const tutorialKeys: Record<ContentSource, string> = {
  hitomi: "atsumi.tutorial.dismissed.v1",
  danbooru: "atsumi.tutorial.danbooru.dismissed.v1",
  chzzk: "atsumi.tutorial.chzzk.dismissed.v1",
};

export function isTutorialDismissed(source: ContentSource = "hitomi"): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(tutorialKeys[source]) === "true";
  } catch {
    return false;
  }
}

export function setTutorialDismissed(dismissed: boolean, source: ContentSource = "hitomi"): void {
  if (typeof window === "undefined") return;
  try {
    if (dismissed) window.localStorage.setItem(tutorialKeys[source], "true");
    else window.localStorage.removeItem(tutorialKeys[source]);
  } catch {
    // The tutorial remains session-only when WebView storage is unavailable.
  }
}
