const TUTORIAL_DISMISSED_KEY = "atsumi.tutorial.dismissed.v1";

export function isTutorialDismissed(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(TUTORIAL_DISMISSED_KEY) === "true";
  } catch {
    return false;
  }
}

export function setTutorialDismissed(dismissed: boolean): void {
  if (typeof window === "undefined") return;
  try {
    if (dismissed) window.localStorage.setItem(TUTORIAL_DISMISSED_KEY, "true");
    else window.localStorage.removeItem(TUTORIAL_DISMISSED_KEY);
  } catch {
    // The tutorial remains session-only when WebView storage is unavailable.
  }
}
