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
    expect(f.container.textContent).toContain("판본 비교");
    expect(f.container.textContent).not.toContain("DUPLICATE REVIEW");
    expect(f.container.querySelector('[aria-label="두 판본 검토"]')).not.toBeNull();
    expect(f.container.querySelector(".overlap-group-edition-info")).toHaveTextContent("일치 2/20");
    expect(f.container.querySelector('.download-overlap-page-cell[aria-label^="#101 2페이지"]')).not.toBeNull();
    expect(f.container.querySelector('.download-overlap-page-cell[aria-label^="#202 9페이지"]')).not.toBeNull();
    expect(f.container.querySelector(".download-overlap-metrics")).toBeNull();
    await f.dispose();
  });
  it("uses edition switches for both completed artifacts and keeps false-positive distinct", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true), f = await mount();
    const mode = async (id:number,label:string) => act(async()=>[...f.container.querySelectorAll<HTMLButtonElement>('[aria-label="#'+id+' 남길 범위"] button')].find(b=>b.textContent===label)!.click());
    await mode(101,'제거'); await f.click('적용');
    await mode(101,'남기기'); await mode(202,'제거'); await f.click('적용');
    await mode(202,'남기기'); await f.click('적용'); await f.click('중복 아님');
    for (const action of ["remove_existing_continue", "remove_incoming", "false_positive_continue", "keep_both_continue"]) {
      expect(f.onDecision).toHaveBeenCalledWith({ reviewId: "duplicate:candidate-real-evidence", candidateId: "candidate-real-evidence", expectedRevision: 7, action });
    }
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining('#101'));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining('#202'));
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
    const review = reviewFixture(); review.candidate.parent.pageCount = 2; review.candidate.candidate.pageCount = 4;
    const confirm=vi.spyOn(window,"confirm").mockReturnValue(true);
    review.pagePairs = review.pagePairs.map((p, i) => ({ ...p, parentSourcePage: i + 1, candidateSourcePage:i+1 }));
    const f = await mount(review);
    for (const page of [2,1]) {
      const cell = f.container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="#101 ${page}페이지"]`)!;
      await act(async () => cell.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    }
    await f.click("적용");
    expect(f.onMergePages).toHaveBeenCalledWith({ reviewId: "duplicate:candidate-real-evidence", candidateId: "candidate-real-evidence", expectedRevision: 7, sourceSide: "existing", sourcePages: [1,2], selectedPages: { existing: [1,2], incoming: [] }, targetSelectionOnly:false,excludeSource:true });
    confirm.mockRestore();
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
