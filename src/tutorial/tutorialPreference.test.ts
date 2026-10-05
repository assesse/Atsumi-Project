import { beforeEach, describe, expect, it } from "vitest";
import { isTutorialDismissed, setTutorialDismissed } from "./tutorialPreference";

describe("tutorial preference", () => {
  beforeEach(() => window.localStorage.clear());

  it("defaults to visible and persists the first-display marker", () => {
    expect(isTutorialDismissed()).toBe(false);
    setTutorialDismissed(true);
    expect(isTutorialDismissed()).toBe(true);
    setTutorialDismissed(false);
    expect(isTutorialDismissed()).toBe(false);
  });
  it("keeps first-entry markers independent for each service", () => {
    setTutorialDismissed(true, "danbooru");
    expect(isTutorialDismissed("danbooru")).toBe(true);
    expect(isTutorialDismissed("hitomi")).toBe(false);
    expect(isTutorialDismissed("chzzk")).toBe(false);
    setTutorialDismissed(true, "chzzk");
    setTutorialDismissed(false, "danbooru");
    expect(isTutorialDismissed("danbooru")).toBe(false);
    expect(isTutorialDismissed("chzzk")).toBe(true);
  });
  it("preserves the legacy Hitomi dismissal without skipping unvisited services", () => {
    window.localStorage.setItem("atsumi.tutorial.dismissed.v1", "true");
    expect(isTutorialDismissed("hitomi")).toBe(true);
    expect(isTutorialDismissed("danbooru")).toBe(false);
    expect(isTutorialDismissed("chzzk")).toBe(false);
  });
});
