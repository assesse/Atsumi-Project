import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { DuplicateReview } from "../api/contracts";
import { galleryId } from "../core/types";
import { ThumbnailClient, type ThumbnailRequest } from "../thumbnail";
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

describe("DuplicateReviewDialog backend evidence", () => {
  it("renders persisted confidence and exact artifact source-page pairs without placeholder values", async () => {
    const resolve = vi.fn((_request: ThumbnailRequest) => ({
      kind: "missing" as const,
      reason: "test fixture",
    }));
    const client = new ThumbnailClient({ resolve });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <DuplicateReviewDialog
        open={false}
        review={reviewFixture()}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onRescan={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    expect(container.querySelector(".review-summary")).toHaveTextContent("신뢰도 73%");
    expect(container.querySelector(".review-summary")).toHaveTextContent("2개 페이지 일치");
    expect(container.querySelector(".match-pairs summary")).toHaveTextContent("2쌍");
    expect(container.textContent).toContain("Persisted one-to-one sequence evidence");
    expect(container.textContent).toContain("detail 37 · edge 91%");
    expect(container.textContent).not.toContain("82%");
    expect(container.textContent).not.toContain("first gid");
    expect(container.textContent).not.toContain("parent gid");

    const artifactPages = resolve.mock.calls
      .map(([request]) => request.key)
      .filter((key) => key.kind === "artifact-page")
      .map((key) => [key.entryId, key.page]);
    expect(artifactPages).toEqual([
      ["verified-parent-entry", 2],
      ["verified-candidate-entry", 9],
      ["verified-parent-entry", 11],
      ["verified-candidate-entry", 14],
    ]);

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("keeps both hide choices for non-containment reviews and hides series classification controls", async () => {
    const onDecision = vi.fn();
    const client = new ThumbnailClient({
      resolve: () => ({ kind: "missing", reason: "test fixture" }),
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <DuplicateReviewDialog
        open={false}
        review={reviewFixture()}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onRescan={vi.fn()}
        onDecision={onDecision}
      />,
    ));

    const click = async (label: string) => {
      const button = [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((item) => item.textContent?.includes(label));
      if (!button) throw new Error(`${label} button missing`);
      await act(async () => button.click());
    };
    await click("작품 A 숨기기");
    await click("작품 B 숨기기");
    await click("이 작품 쌍 제외");

    expect(onDecision).toHaveBeenCalledWith({
      candidateId: "candidate-real-evidence",
      expectedRevision: 7,
      action: "hide_parent",
    });
    expect(onDecision).toHaveBeenCalledWith(expect.objectContaining({ action: "hide_candidate", expectedRevision: 7 }));
    expect(onDecision).toHaveBeenCalledWith(expect.objectContaining({ action: "exclude_pair", expectedRevision: 7 }));
    expect(container.querySelector(".series-decision")).toBeNull();
    expect(container.textContent).not.toContain("연작 관계");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("forces a contains decision to keep the longer parent and hide only the shorter candidate", async () => {
    const onDecision = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = reviewFixture();
    review.candidate.relation = "contains";

    await act(async () => root.render(
      <DuplicateReviewDialog
        open={false}
        review={review}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onRescan={vi.fn()}
        onDecision={onDecision}
      />,
    ));

    expect(container.querySelector(".containment-policy")).toHaveTextContent("20p 포괄 작품을 남깁니다");
    expect(container.querySelectorAll(".review-card h3")[0]).toHaveTextContent("포괄 작품");
    expect(container.querySelectorAll(".review-card h3")[1]).toHaveTextContent("귀속 작품");
    expect(container.textContent).not.toContain("작품 A 숨기기");
    expect(container.textContent).not.toContain("작품 B 숨기기");
    const action = container.querySelector<HTMLButtonElement>(".containment-keep-action");
    expect(action).toHaveTextContent("20p 포괄 작품 유지 · 16p 귀속 작품 숨기기");
    await act(async () => action?.click());
    expect(onDecision).toHaveBeenCalledWith({
      candidateId: "candidate-real-evidence",
      expectedRevision: 7,
      action: "hide_candidate",
    });

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("forces the inverse contains decision to keep a longer candidate", async () => {
    const onDecision = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = reviewFixture();
    review.candidate.relation = "contains";
    review.candidate.parent.pageCount = 16;
    review.candidate.candidate.pageCount = 20;

    await act(async () => root.render(
      <DuplicateReviewDialog
        open={false}
        review={review}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onRescan={vi.fn()}
        onDecision={onDecision}
      />,
    ));

    await act(async () => container.querySelector<HTMLButtonElement>(".containment-keep-action")?.click());
    expect(onDecision).toHaveBeenCalledWith(expect.objectContaining({ action: "hide_parent" }));

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("retains the safe two-sided choice when contains page counts are equal", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = reviewFixture();
    review.candidate.relation = "contains";
    review.candidate.candidate.pageCount = 20;

    await act(async () => root.render(
      <DuplicateReviewDialog
        open={false}
        review={review}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onRescan={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    expect(container.querySelector(".containment-policy")).toBeNull();
    expect(container.textContent).toContain("작품 A 숨기기");
    expect(container.textContent).toContain("작품 B 숨기기");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });
});
