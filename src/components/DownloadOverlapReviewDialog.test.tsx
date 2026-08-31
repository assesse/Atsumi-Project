import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { DownloadOverlapReview } from "../api/contracts";
import { galleryId } from "../core/types";
import { ThumbnailClient, type ThumbnailRequest } from "../thumbnail";
import { DownloadOverlapReviewDialog } from "./DownloadOverlapReviewDialog";

const fixture = (): DownloadOverlapReview => ({
  reviewId: "review-overlap",
  entryId: "incoming-entry",
  incoming: {
    entryId: "incoming-entry",
    galleryId: galleryId(200),
    title: "New edition",
    artists: ["artist a"],
    pageCount: 12,
  },
  revision: 4,
  state: "pending",
  profileVersion: 1,
  policyVersion: 1,
  incomingFingerprint: "incoming-fingerprint",
  candidates: ["near_equivalent", "incoming_contains_existing", "existing_contains_incoming", "partial_overlap"].map((relation, index) => ({
    candidateId: `candidate-${index + 1}`,
    existing: {
      entryId: `existing-entry-${index + 1}`,
      galleryId: galleryId(100 + index),
      title: `Owned edition ${index + 1}`,
      artists: ["artist a"],
      pageCount: 10,
    },
    existingFingerprint: `fingerprint-${index + 1}`,
    relation: relation as DownloadOverlapReview["candidates"][number]["relation"],
    confidence: 0.94,
    matchedPages: 8,
    exactPages: 3,
    visualPages: 5,
    existingCoverage: 0.8,
    incomingCoverage: 2 / 3,
    existingUniquePages: 2,
    incomingUniquePages: 4,
    longestAlignedRun: 6,
    rank: index + 1,
    pagePairs: [{
      incomingSourcePage: index + 2,
      existingSourcePage: index + 1,
      exactSha256: false,
      dHashDistance: 2,
      pHashDistance: 3,
      detailHashDistance: 19,
      edgeSimilarity: 0.91,
      visualSimilarity: 0.93,
      lowInformation: false,
    }],
  })),
  createdAt: "2026-08-25T00:00:00.000Z",
  updatedAt: "2026-08-25T00:00:00.000Z",
});

describe("DownloadOverlapReviewDialog", () => {
  it("shows a conservative extra-page recommendation only for an eligible review", async () => {
    const review = fixture();
    review.incoming.pageCount = 30;
    const eligible = review.candidates[1]!;
    eligible.existing.pageCount = 20;
    eligible.matchedPages = 20;
    eligible.exactPages = 20;
    eligible.visualPages = 0;
    eligible.existingCoverage = 1;
    eligible.incomingCoverage = 2 / 3;
    eligible.existingUniquePages = 0;
    eligible.incomingUniquePages = 10;
    eligible.longestAlignedRun = 20;
    eligible.pagePairs = Array.from({ length: 20 }, (_, index) => ({
      ...eligible.pagePairs[0]!,
      incomingSourcePage: index + 1,
      existingSourcePage: index + 1,
      exactSha256: true,
      lowInformation: false,
    }));
    review.candidates = [eligible];
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        autoMode="recommend"
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    expect(container.querySelector(".download-overlap-auto-recommendation")?.textContent)
      .toContain("엄격 기준 추천");
    expect(container.textContent).toContain("신규 앨범 B가 기존 앨범 A의 실질 페이지를 모두 포함");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("renders dark-theme-ready vertical A/B summaries and an aligned page lane", async () => {
    const resolve = vi.fn((_request: ThumbnailRequest) => ({ kind: "missing" as const, reason: "fixture" }));
    const client = new ThumbnailClient({ resolve });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    const review = fixture();
    review.incoming.pageCount = 16;
    const active = review.candidates[0]!;
    active.existing.pageCount = 11;
    active.pagePairs = [
      ...Array.from({ length: 8 }, (_, index) => ({
        ...active.pagePairs[0]!,
        existingSourcePage: index + 1,
        incomingSourcePage: index + 1,
      })),
      ...[14, 15, 16].map((incomingSourcePage, index) => ({
        ...active.pagePairs[0]!,
        existingSourcePage: index + 9,
        incomingSourcePage,
      })),
    ];
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(4);
    expect(container.textContent).toContain("거의 같은 판본");
    expect(container.textContent).toContain("기존 A 범위");
    expect(container.textContent).toContain("80%");
    expect(container.textContent).toContain("기존 앨범 A");
    expect(container.textContent).toContain("신규 앨범 B");
    expect(container.textContent).toContain("신규 B에만 9~13p · 5장");
    expect(container.querySelector(".download-overlap-artifacts")?.children).toHaveLength(2);
    expect(container.querySelectorAll(".download-overlap-page-cell.is-gap")).toHaveLength(5);
    expect(container.querySelectorAll(".download-overlap-page-cell.is-unique")).toHaveLength(5);
    const artifactPages = resolve.mock.calls
      .map(([request]) => request.key)
      .filter((key) => key.kind === "artifact-page")
      .map((key) => [key.entryId, key.page]);
    expect(artifactPages).toEqual(expect.arrayContaining([
      ["existing-entry-1", 1],
      ["incoming-entry", 1],
    ]));

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("submits five revision-checked actions with candidate scope and removal confirmation", async () => {
    const onDecision = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={fixture()}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={onDecision}
      />,
    ));
    const click = async (label: string) => {
      const button = [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((item) => item.textContent?.includes(label));
      if (!button) throw new Error(`${label} button missing`);
      await act(async () => button.click());
    };
    expect(container.querySelectorAll('[role="tooltip"]')).toHaveLength(5);
    const actionButtons = ["검토 미루기", "기존 A 제거", "신규 B 제거", "오탐 판정", "문제 없음"]
      .map((label) => [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === label));
    expect(actionButtons.every((button) => Boolean(button?.getAttribute("aria-describedby")))).toBe(true);
    await click("기존 A 제거");
    await click("신규 B 제거");
    await click("오탐 판정");
    await click("문제 없음");
    await click("검토 미루기");
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(onDecision).toHaveBeenCalledWith({
      reviewId: "review-overlap",
      expectedRevision: 4,
      action: "remove_existing_continue",
      candidateId: "candidate-1",
    });
    expect(onDecision).toHaveBeenCalledWith(expect.objectContaining({ action: "remove_incoming" }));
    expect(onDecision).toHaveBeenCalledWith({
      reviewId: "review-overlap",
      expectedRevision: 4,
      action: "false_positive_continue",
      candidateId: "candidate-1",
    });
    expect(onDecision).toHaveBeenCalledWith(expect.objectContaining({
      action: "keep_both_continue",
      candidateId: "candidate-1",
    }));

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
    confirm.mockRestore();
  });

  it("enlarges aligned pages on hover using the configured card preview width without leaving the dialog", async () => {
    const resolve = vi.fn((_request: ThumbnailRequest) => ({
      kind: "image" as const,
      url: "data:image/png;base64,fixture",
      width: 800,
      height: 1200,
    }));
    const client = new ThumbnailClient({ resolve });
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.matches("dialog.download-overlap-dialog")) {
        return { x: 20, y: 20, width: 500, height: 700, top: 20, right: 520, bottom: 720, left: 20, toJSON: () => ({}) };
      }
      if (this.classList.contains("download-overlap-page-cell")) {
        return { x: 425, y: 590, width: 82, height: 112, top: 590, right: 507, bottom: 702, left: 425, toJSON: () => ({}) };
      }
      return { x: 0, y: 0, width: 0, height: 0, top: 0, right: 0, bottom: 0, left: 0, toJSON: () => ({}) };
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog
          open={false}
          review={fixture()}
          previewWidth={280}
          thumbnailClient={client}
          onClose={vi.fn()}
          onRetry={vi.fn()}
          onDecision={vi.fn()}
        />,
      ));
      const dialog = container.querySelector<HTMLDialogElement>(".download-overlap-dialog")!;
      const page = container.querySelector<HTMLElement>(".download-overlap-page-cell:not(.is-gap)")!;

      await act(async () => page.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));

      const enlarged = dialog.querySelector<HTMLElement>(".download-overlap-page-hover-preview")!;
      expect(enlarged).not.toBeNull();
      expect(enlarged.parentElement).toBe(dialog);
      expect(enlarged).toHaveAttribute("data-preview-width", "280");
      expect(enlarged.style.position).toBe("fixed");
      expect(enlarged.style.width).toBe("280px");
      expect(enlarged.style.height).toBe("420px");
      expect(enlarged.textContent).toContain("기존 A 1p · 시각 93%");
      expect(Number.parseFloat(enlarged.style.left) + Number.parseFloat(enlarged.style.width)).toBeLessThanOrEqual(510);
      expect(Number.parseFloat(enlarged.style.top) + Number.parseFloat(enlarged.style.height)).toBeLessThanOrEqual(710);

      await act(async () => page.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));
      expect(dialog.querySelector(".download-overlap-page-hover-preview")).toBeNull();

      const gap = container.querySelector<HTMLElement>(".download-overlap-page-cell.is-gap")!;
      await act(async () => gap.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
      expect(dialog.querySelector(".download-overlap-page-hover-preview")).toBeNull();
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
      rect.mockRestore();
    }
  });
});
