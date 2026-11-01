import { describe, expect, it } from "vitest";
import { queueRemainingLabel } from "./queueRemainingLabel";

describe("activity queue estimate", () => {
  it("does not claim completion without a usable active-work estimate", () => {
    expect(queueRemainingLabel(null)).toBe("");
    expect(queueRemainingLabel({ globalActive: 0, etaSeconds: 0, recentCompleted: 12 })).toBe("");
    for (const etaSeconds of [null, 0, -1, NaN, Infinity]) {
      expect(queueRemainingLabel({ globalActive: 14, etaSeconds, recentCompleted: 12 })).toBe("예상 시간 계산 중");
    }
    expect(queueRemainingLabel({ globalActive: 14, etaSeconds: 3600, recentCompleted: 2 })).toBe("예상 시간 계산 중");
    expect(queueRemainingLabel({ globalActive: 14, etaSeconds: 3600, recentCompleted: 12 }, "offline")).toBe("예상 시간 확인 중");
  });
  it("uses coarse durations, including queues consisting only of postprocessing", () => {
    expect(queueRemainingLabel({ globalActive: 18, etaSeconds: 126274, recentCompleted: 23 })).toBe("약 35시간 5분 남음");
    expect(queueRemainingLabel({ globalActive: 1, etaSeconds: 1, recentCompleted: 3 })).toBe("약 1분 남음");
    expect(queueRemainingLabel({ globalActive: 1, etaSeconds: 3600, recentCompleted: 3 })).toBe("약 1시간 남음");
  });
});
