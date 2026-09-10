import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { DuplicateReview } from "../api/contracts";
import { galleryId } from "../core/types";
import { ThumbnailClient } from "../thumbnail";
import { completedPairReview } from "../state/completedPairReview";
import { DuplicateReviewDialog } from "./DuplicateReviewDialog";

const reviewFixture = (patch: Partial<DuplicateReview> = {}): DuplicateReview => ({
  candidate: {
    candidateId: "candidate-real-evidence",
    revision: 7,
    parent: {
      galleryId: galleryId(101),
      entryId: "verified-parent-entry",
      title: "Verified Parent",
      artist: "artist a",
      pageCount: 20,
    },
    candidate: {
      galleryId: galleryId(202),
      entryId: "verified-candidate-entry",
      title: "Verified Candidate",
      artist: "artist a",
      pageCount: 16,
    },
    relation: "partial",
    confidence: 0.73,
    matchedPages: 2,
    parentCoverage: 0.1,
    candidateCoverage: 0.125,
    createdAt: "2026-08-15T00:00:00.000Z",
    updatedAt: "2026-08-15T00:00:00.000Z",
  },
  evidence: [{
    evidenceId: "evidence-sequence",
    kind: "sequence_alignment",
    confidence: 0.73,
    matchedPages: 2,
    description: "Persisted one-to-one sequence evidence",
  }],
  pagePairs: [
    {
      parentSourcePage: 2,
      candidateSourcePage: 9,
      exactSha256: true,
      dHashDistance: 0,
      pHashDistance: 0,
      detailHashDistance: 0,
      edgeSimilarity: 1,
      visualSimilarity: 1,
      lowInformation: false,
    },
    {
      parentSourcePage: 11,
      candidateSourcePage: 14,
      exactSha256: false,
      dHashDistance: 4,
      pHashDistance: 5,
      detailHashDistance: 37,
      edgeSimilarity: 0.91,
      visualSimilarity: 0.92,
      lowInformation: false,
    },
  ],
  decisions: [],
  seriesGroups: [],
  ...patch,
});

describe("completed pairs share DownloadOverlapReviewDialog", () => {
  const mount = async (review = reviewFixture()) => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container); const onDecision = vi.fn(); const onMergePages = vi.fn();
    await act(async () => root.render(<DuplicateReviewDialog open={false} review={review} previewWidth={420}
      thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onRescan={vi.fn()}
      onDecision={onDecision} onMergePages={onMergePages} />));
    const click = async (label: string) => {
      const button = [...container.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);
      expect(button).toBeDefined(); await act(async () => button!.click());
    };
    return { container, onDecision, onMergePages, click, dispose: async () => {
      await act(async () => root.unmount()); client.dispose(); container.remove();
    } };
  };
  it("uses the shared heading, full A/B alignment and source coordinates", async () => {
    const f = await mount();
    expect(f.container.textContent).toContain("DOWNLOAD OVERLAP REVIEW");
    expect(f.container.textContent).not.toContain("DUPLICATE REVIEW");
    expect(f.container.textContent).toContain("완료 앨범 A/B 수동 대조");
    expect(f.container.querySelector(".download-overlap-page-map")).toHaveTextContent("2쌍 일치");
    expect(f.container.querySelector('.download-overlap-page-cell[aria-label^="기존 A 2페이지"]')).not.toBeNull();
    expect(f.container.querySelector('.download-overlap-page-cell[aria-label^="신규 B 9페이지"]')).not.toBeNull();
    expect(f.container.querySelector(".download-overlap-metrics")).toHaveTextContent("1 / 1");
    await f.dispose();
  });
  it("keeps A/B explicit choices and distinct keep/false-positive labels", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const f = await mount();
    await f.click("A 제외"); await f.click("B 제외"); await f.click("오탐 판정"); await f.click("둘 다 보존");
    for (const action of ["remove_existing_continue", "remove_incoming", "false_positive_continue", "keep_both_continue"]) {
      expect(f.onDecision).toHaveBeenCalledWith({ reviewId: "duplicate:candidate-real-evidence", candidateId: "candidate-real-evidence", expectedRevision: 7, action });
    }
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("완료 앨범 B"));
    await f.dispose(); confirm.mockRestore();
  });
  it("maps inverse containment and both unequal page numbers without swapping A/B", () => {
    const review = reviewFixture(); review.candidate.relation = "contains";
    const result = completedPairReview(review);
    expect(result.candidates[0]!.relation).toBe("existing_contains_incoming");
    expect(result.candidates[0]!.pagePairs[0]).toMatchObject({ existingSourcePage: 2, incomingSourcePage: 9 });
    review.candidate.candidate.pageCount = 21;
    expect(completedPairReview(review).candidates[0]!.relation).toBe("incoming_contains_existing");
    expect(result.entryId).toBe("verified-candidate-entry");
  });
  it("provides shared Ctrl-selection and merge for a fully mapped donor", async () => {
    const review = reviewFixture(); review.candidate.parent.pageCount = 2;
    review.pagePairs = review.pagePairs.map((p, i) => ({ ...p, parentSourcePage: i + 1 }));
    const f = await mount(review);
    for (const page of [2,1]) {
      const cell = f.container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="기존 A ${page}페이지"]`)!;
      await act(async () => cell.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    }
    await f.click("선택한 2장 병합");
    expect(f.onMergePages).toHaveBeenCalledWith({ reviewId: "duplicate:candidate-real-evidence", candidateId: "candidate-real-evidence", expectedRevision: 7, sourceSide: "existing", sourcePages: [1,2] });
    await f.dispose();
  });
  it("shows previously processed decisions read-only", async () => {
    const review = reviewFixture({ decisions: [{ decisionId: "d", candidateId: "candidate-real-evidence", candidateRevision: 7, action: "exclude_pair", createdAt: "now" }] });
    const f = await mount(review);
    expect(f.container.textContent).toContain("읽기 전용 판정 기록");
    expect([...f.container.querySelectorAll("button")].some((b) => b.textContent === "A 제외")).toBe(false);
    await f.dispose();
  });
});
