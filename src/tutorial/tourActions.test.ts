import { afterEach, describe, expect, it, vi } from "vitest";
import { beginTutorialAction, tutorialActionEvent } from "./tourActions";
import { tutorialStepsBySource } from "./tourSteps";

afterEach(() => {
  delete document.documentElement.dataset.tutorialOpen;
  delete document.documentElement.dataset.tutorialStep;
  delete document.documentElement.dataset.tutorialVisit;
});

describe("tutorial action results", () => {
  it("is silent outside a tour and ignores late results after navigating or closing", () => {
    const listener = vi.fn(); window.addEventListener(tutorialActionEvent, listener);
    try {
      beginTutorialAction("follow")(); expect(listener).not.toHaveBeenCalled();
      Object.assign(document.documentElement.dataset, { tutorialOpen: "true", tutorialStep: "follow", tutorialVisit: "1" });
      const finish = beginTutorialAction("follow"); expect(listener).toHaveBeenCalledOnce();
      document.documentElement.dataset.tutorialVisit = "2";
      finish(); expect(listener).toHaveBeenCalledOnce();
      const current = beginTutorialAction("follow");
      current("저장 실패"); expect(listener.mock.lastCall?.[0].detail.phase).toBe("error");
      const beforeClose = listener.mock.calls.length;
      delete document.documentElement.dataset.tutorialOpen;
      current(); expect(listener).toHaveBeenCalledTimes(beforeClose);
    } finally { window.removeEventListener(tutorialActionEvent, listener); }
  });
  it("covers all sources with required favorite registration before the real refresh", () => {
    const tourSteps = tutorialStepsBySource.hitomi;
    expect(new Set(tourSteps.map(step => step.section))).toEqual(new Set(["Hitomi", "공통"]));
    const follow = tourSteps.findIndex(step => step.id === "follow");
    const refresh = tourSteps.findIndex(step => step.id === "auto-find-refresh");
    expect(follow).toBeLessThan(refresh);
    expect(tourSteps[follow]).toMatchObject({ action: "contextmenu", waitForResult: true });
    expect(tourSteps[refresh]).toMatchObject({ action: "click", waitForResult: true });
    expect(tutorialStepsBySource.chzzk.find(step => step.id === "chzzk-auto-info")?.action).toBeUndefined();
  });
  it("keeps guides short, source-local and free of forced service switches", () => {
    expect(Object.fromEntries(Object.entries(tutorialStepsBySource).map(([source, steps]) => [source, steps.length])))
      .toEqual({ hitomi: 15, danbooru: 4, chzzk: 6, youtube: 4, soop: 4, pinterest: 4, pixiv: 4, x: 2 });
    for (const [source, steps] of Object.entries(tutorialStepsBySource)) {
      expect(new Set(steps.map(step => step.id)).size).toBe(steps.length);
      for (const step of steps) {
        expect(step.id).not.toMatch(/-close$/);
        if (source !== "hitomi") expect(step.id.startsWith(`${source}-`)).toBe(true);
        expect(step.target).not.toMatch(/data-tour="source-(?:hitomi|danbooru|chzzk)"/);
      }
    }
  });
  it("introduces native discovery and saved media without forcing a download or account action", () => {
    for (const source of ["youtube", "soop", "pinterest", "pixiv"] as const) {
      const steps = tutorialStepsBySource[source];
      expect(steps[0]).toMatchObject({ id: `${source}-explore`, action: "click" });
      expect(steps[1]!.action).toBeUndefined();
      expect(steps[2]).toMatchObject({ id: `${source}-${source === "soop" || source === "youtube" ? "live" : "favorites"}`, action: "click" });
      expect(steps[3]).toMatchObject({ id: `${source}-downloads`, action: "click" });
      expect(steps.filter(step => step.action).every(step => step.target.includes(`${source}-nav-`))).toBe(true);
      expect(steps.every(step => step.description.length <= 65)).toBe(true);
    }
    expect(tutorialStepsBySource.pinterest[0]!.description).toContain("프로필·보드");
    expect(tutorialStepsBySource.pinterest[0]!.description).not.toContain("키워드");
    expect(tutorialStepsBySource.soop.some(step => step.target.includes("soop-nav-favorites"))).toBe(false);
    expect(tutorialStepsBySource.soop[2]!.description).toContain("하단 꺾쇠");
  });
});
