import { describe, expect, it } from "vitest";
import { galleryId } from "../core/types";
import type { AutoFindCutoffEvidence } from "../api/contracts";
import { autoFindCutoffDescription } from "./autoFindHistory";

describe("Auto Find saved search evidence", () => {
  const evidence: AutoFindCutoffEvidence = {
    namespace: "group", artist: "circle", qualifiedOwnedCount: 2,
    source: "verified_owned_artifact", policyVersion: 2,
    latestOwnedGalleryId: galleryId(300),
  };

  it("describes the highest owned gallery ID without calling it a download date", () => {
    expect(autoFindCutoffDescription(evidence, "newer_than_latest_owned"))
      .toBe("검증된 소유본 2개 중 가장 최근 작품 #300 이후를 검색 (작품번호 기준)");
  });

  it("keeps historical minimum evidence labeled as the previous policy", () => {
    expect(autoFindCutoffDescription({
      ...evidence, latestOwnedGalleryId: undefined, oldestOwnedGalleryId: galleryId(100), policyVersion: 1,
    }, "newer_than_oldest_downloaded"))
      .toBe("이전 기준 · 검증된 소유본 2개 중 가장 오래된 #100 이후를 검색");
  });

  it("makes the no-owned fallback explicit", () => {
    expect(autoFindCutoffDescription({
      ...evidence, latestOwnedGalleryId: undefined, qualifiedOwnedCount: 0,
    }, "newer_than_latest_owned")).toBe("검증 완료·격리 소유 작품 없음 · 전체 범위 탐색");
  });
});
