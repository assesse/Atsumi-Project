import { describe, expect, it } from "vitest";
import { hitomiTourSteps, chzzkTourSteps } from "./tourSteps";

describe("privacy introduction", () => {
  it("explains channel addition and favorite auto recording without separate Mado modes", () => {
    const step = chzzkTourSteps.find(step => step.id === "chzzk-more")!;
    expect(step.description).toContain("+ 버튼");
    expect(step.description).toContain("최대 네 채널");
    expect(step.action).toBeUndefined();
    expect(chzzkTourSteps.find(step => step.id === "chzzk-live")).toMatchObject({ title: "방송 보기", satisfiedSelector: ':scope[aria-current="page"]' });
    expect(chzzkTourSteps.some(step => step.id === "chzzk-channels")).toBe(false); // A second click would close the same panel.
    expect(chzzkTourSteps.some(step => step.id === "chzzk-mado")).toBe(false);
    expect(chzzkTourSteps.find(step => step.id === "chzzk-auto")?.description).toContain("즐겨찾기 채널");
  });
  it("explains the default before browsing without forcing private images to appear", () => {
    const index = hitomiTourSteps.findIndex(step => step.id === "privacy");
    expect(index).toBeGreaterThan(hitomiTourSteps.findIndex(step => step.id === "folder"));
    expect(index).toBeLessThan(hitomiTourSteps.findIndex(step => step.id === "search"));
    expect(hitomiTourSteps[index]).toMatchObject({ target: '[data-tour="privacy-mode"]', description: expect.stringContaining("기본값은 켜짐") });
    expect(hitomiTourSteps[index]?.action).toBeUndefined();
  });
});
