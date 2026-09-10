import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "./backend";
import type { DownloadEntry, DuplicateReview } from "./contracts";
import { galleryId } from "../core/types";

const state = backend as unknown as {
  duplicateReviews: Map<string, DuplicateReview>;
  duplicateResolvedCandidates: Set<string>;
  downloadEntries: Map<string, DownloadEntry>;
  duplicateHiddenGalleryIds: Set<number>;
};
const ref = (id: number, count: number) => ({ galleryId: galleryId(id), entryId: `entry-${id}`, title: `Album ${id}`, pageCount: count });
beforeEach(() => {
  const a = ref(101, 2), b = ref(102, 3);
  state.duplicateResolvedCandidates.delete("pair");
  state.duplicateHiddenGalleryIds.delete(101); state.duplicateHiddenGalleryIds.delete(102);
  state.downloadEntries = new Map([a, b].map((r) => [r.entryId, { entryId: r.entryId, galleryId: r.galleryId, revision: 0, state: "completed", progress: 100 }]));
  state.duplicateReviews.set("pair", {
    candidate: { candidateId: "pair", revision: 4, parent: a, candidate: b, relation: "contains", confidence: 0.99, matchedPages: 2, parentCoverage: 1, candidateCoverage: 2 / 3, createdAt: "now", updatedAt: "now" },
    evidence: [], decisions: [], seriesGroups: [],
    pagePairs: [1, 2].map((n) => ({ parentSourcePage: n, candidateSourcePage: n === 1 ? 1 : 3, exactSha256: false, dHashDistance: 1, pHashDistance: 1, detailHashDistance: 1, edgeSimilarity: 0.99, visualSimilarity: 0.99, lowInformation: false })),
  });
});
describe("completed-pair overlap API", () => {
  it.each(["keep_both_continue", "false_positive_continue"] as const)("keeps completed jobs and preserves %s audit", async (action) => {
    const result = await backend.downloadOverlapDecisionApply({ reviewId: "duplicate:pair", candidateId: "pair", expectedRevision: 4, action });
    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error(result.error.message);
    expect(result.data).toMatchObject({ resumed: false, cancelled: false, review: { state: "resolved", decisions: [{ action, actor: "human" }] } });
    expect([...state.downloadEntries.values()].map((e) => e.state)).toEqual(["completed", "completed"]);
    expect((await backend.downloadOverlapDecisionApply({ reviewId: "duplicate:pair", candidateId: "pair", expectedRevision: 4, action })).ok).toBe(false);
  });
  it("excludes only B and preserves A without queueing it", async () => {
    const result = await backend.downloadOverlapDecisionApply({ reviewId: "duplicate:pair", candidateId: "pair", expectedRevision: 4, action: "remove_incoming" });
    expect(result.ok).toBe(true);
    expect(state.downloadEntries.get("entry-101")?.state).toBe("completed");
    expect(state.downloadEntries.get("entry-102")?.state).toBe("cancelled");
    expect(state.duplicateHiddenGalleryIds.has(101)).toBe(false);
    expect(state.duplicateHiddenGalleryIds.has(102)).toBe(true);
  });
  it("merges A into B using the shared endpoint, then excludes A and stales old evidence", async () => {
    const result = await backend.downloadOverlapMerge({ reviewId: "duplicate:pair", candidateId: "pair", expectedRevision: 4, sourceSide: "existing", sourcePages: [1, 2], excludeSource: true });
    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error(result.error.message);
    expect(result.data).toMatchObject({ sourceGalleryId: 101, targetGalleryId: 102, replacedPages: 2, sourceExcluded: true });
    expect(state.downloadEntries.get("entry-101")?.state).toBe("cancelled");
    expect(state.downloadEntries.get("entry-102")?.state).toBe("completed");
    const old = await backend.duplicateReviewGet("pair");
    expect(old.ok && old.data.artifactStale).toBe(true);
  });
});
