import { describe, expect, it } from "vitest";
import { hitomiTourSteps, chzzkTourSteps } from "./tourSteps";

describe("privacy introduction", () => {
  it("explains both Mado layouts without starting playback or changing a recording", () => {
    const step = chzzkTourSteps.find(step => step.id === "chzzk-mado")!;
    expect(step.description).toContain("1화면4챗: 선택한 영상 하나");
    expect(step.description).toContain("4화면4챗: 네 채널의 영상");
    expect(step.action).toBeUndefined();
    expect(chzzkTourSteps.find(step => step.id === "chzzk-channels")?.title).toBe("방송 보기");
  });
  it("explains the default before browsing without forcing private images to appear", () => {
    const index = hitomiTourSteps.findIndex(step => step.id === "privacy");
    expect(index).toBeGreaterThan(hitomiTourSteps.findIndex(step => step.id === "folder"));
    expect(index).toBeLessThan(hitomiTourSteps.findIndex(step => step.id === "search"));
    expect(hitomiTourSteps[index]).toMatchObject({ target: '[data-tour="privacy-mode"]', description: expect.stringContaining("기본값은 켜짐") });
    expect(hitomiTourSteps[index]?.action).toBeUndefined();
  });
});
