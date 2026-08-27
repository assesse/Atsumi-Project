import { beforeEach, describe, expect, it } from "vitest";
import { isTutorialDismissed, setTutorialDismissed } from "./tutorialPreference";

describe("tutorial preference", () => {
  beforeEach(() => window.localStorage.clear());

  it("defaults to visible and persists an explicit do-not-show-again choice", () => {
    expect(isTutorialDismissed()).toBe(false);
    setTutorialDismissed(true);
    expect(isTutorialDismissed()).toBe(true);
    setTutorialDismissed(false);
    expect(isTutorialDismissed()).toBe(false);
  });
});
