import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { InternalDuplicateReview, InternalRemovalPlan } from "../api/contracts";
import { galleryId } from "../core/types";
import { ThumbnailClient, type ThumbnailRequest } from "../thumbnail";
import { InternalDuplicateDialog } from "./InternalDuplicateDialog";

const review: InternalDuplicateReview = {
  entryId: "verified-entry-1",
  galleryId: galleryId(101),
  title: "Original Source Pages",
  groups: [{
    groupId: "group-1",
    blockId: "block-1",
    sequenceIndex: 0,
    revision: 4,
    entryId: "verified-entry-1",
    galleryId: galleryId(101),
    relation: "exact",
    confidence: 1,
    recommendedKeepSourcePage: 2,
    pages: [2, 8].map((sourcePage) => ({
      sourcePage,
      exactSha256: true,
      visualSimilarity: 1,
      detailHashDistance: 0,
      lowInformation: false,
    })),
    resolved: false,
    createdAt: "2026-08-16T00:00:00.000Z",
    updatedAt: "2026-08-16T00:00:00.000Z",
  }],
  quarantineRecords: [],
};

const plan: InternalRemovalPlan = {
  planId: "plan-1",
  entryId: review.entryId,
  selections: [{
    groupId: "group-1",
    expectedRevision: 4,
    keepSourcePage: 2,
    removeSourcePages: [8],
  }],
  filesToQuarantine: 1,
  bytesToQuarantine: 512_000,
  expiresAt: String(Date.now() + 60_000),
};

describe("InternalDuplicateDialog", () => {
  it("uses verified artifact pages, preserves source numbers, previews a plan, and never offers deletion", async () => {
    const resolve = vi.fn((_request: ThumbnailRequest) => ({ kind: "missing" as const, reason: "test" }));
    const client = new ThumbnailClient({ resolve });
    const onPlan = vi.fn();
    const onApply = vi.fn();
    const onRescan = vi.fn();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = (currentPlan?: InternalRemovalPlan) => root.render(
      <InternalDuplicateDialog
        open={false}
        review={review}
        plan={currentPlan}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onRescan={onRescan}
        onPlan={onPlan}
        onApply={onApply}
        onUndo={vi.fn()}
      />,
    );
    await act(async () => render());

    const dialog = container.querySelector<HTMLDialogElement>(".internal-review-dialog");
    expect(dialog).toHaveAttribute("data-image-density", "fixed-200");
    expect(dialog?.style.getPropertyValue("--internal-scene-column-width")).toBe("208px");
    expect(dialog?.style.getPropertyValue("--internal-legacy-image-width")).toBe("200px");
    expect(container.textContent).toContain("원본 페이지 번호는 바뀌지 않습니다");
    expect(container.textContent).toContain("원본 2p");
    expect(container.textContent).toContain("원본 8p");
    expect(container.textContent).not.toContain("영구 삭제 적용");
    const rescan = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("이 앨범 다시 검사"));
    await act(async () => rescan?.click());
    expect(onRescan).toHaveBeenCalledOnce();
    expect(resolve.mock.calls.map(([request]) => request.key)).toEqual([
      expect.objectContaining({ kind: "artifact-page", entryId: "verified-entry-1", page: 2 }),
      expect.objectContaining({ kind: "artifact-page", entryId: "verified-entry-1", page: 8 }),
    ]);

    const preview = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("격리 계획 미리보기"));
    await act(async () => preview?.click());
    expect(onPlan).toHaveBeenCalledWith({
      entryId: "verified-entry-1",
      selections: [{
        groupId: "group-1",
        expectedRevision: 4,
        keepSourcePage: 2,
        removeSourcePages: [8],
      }],
    });

    await act(async () => render(plan));
    expect(container.querySelector(".internal-plan-summary")).toHaveTextContent("1개 파일");
    const apply = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("계획대로 격리 적용"));
    await act(async () => apply?.click());
    expect(onApply).toHaveBeenCalledWith(plan);

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("offers undo only for persisted quarantined records", async () => {
    const onUndo = vi.fn();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const quarantined: InternalDuplicateReview = {
      ...review,
      groups: [],
      quarantineRecords: [{
        recordId: "record-8",
        planId: "plan-1",
        entryId: review.entryId,
        galleryId: review.galleryId,
        sourcePage: 8,
        originalRelativePath: "album/0008.webp",
        quarantineRelativePath: "album/.atsumi-page-quarantine/plan-1/0008.webp",
        reason: "review",
        state: "quarantined",
        createdAt: "2026-08-16T00:00:00.000Z",
        updatedAt: "2026-08-16T00:00:01.000Z",
      }],
    };
    await act(async () => root.render(
      <InternalDuplicateDialog
        open={false}
        review={quarantined}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onRescan={vi.fn()}
        onPlan={vi.fn()}
        onApply={vi.fn()}
        onUndo={onUndo}
      />,
    ));
    const undo = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("모두 되돌리기"));
    await act(async () => undo?.click());
    expect(onUndo).toHaveBeenCalledWith(["record-8"]);
    await act(async () => root.unmount());
    container.remove();
  });

  it("keeps exactly one radio choice and plans every other page in an N-way row", async () => {
    const fourWay: InternalDuplicateReview = {
      ...review,
      groups: [{
        ...review.groups[0]!,
        groupId: "group-four-way",
        recommendedKeepSourcePage: 1,
        pages: [1, 6, 11, 16].map((sourcePage) => ({
          sourcePage, exactSha256: false, visualSimilarity: 0.91, detailHashDistance: 32, lowInformation: false,
        })),
      }],
    };
    const onPlan = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing" as const, reason: "test" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <InternalDuplicateDialog open={false} review={fourWay} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onRescan={vi.fn()} onPlan={onPlan} onApply={vi.fn()} onUndo={vi.fn()} />,
    ));
    expect(container.querySelectorAll('input[type="radio"]')).toHaveLength(4);
    expect(container.querySelector('input[type="radio"]:checked')?.parentElement).toHaveTextContent("원본 1p");
    const preview = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("격리 계획 미리보기"));
    await act(async () => preview?.click());
    expect(onPlan).toHaveBeenCalledWith(expect.objectContaining({
      selections: [expect.objectContaining({ keepSourcePage: 1, removeSourcePages: [6, 11, 16] })],
    }));
    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("keeps multiple edition sets, contains horizontal scrolling to the matrix, and preserves a fully missing scene", async () => {
    const editionReview: InternalDuplicateReview = {
      ...review,
      groups: Array.from({ length: 5 }, (_, sequenceIndex) => ({
        ...review.groups[0]!,
        groupId: `edition-row-${sequenceIndex + 1}`,
        blockId: "edition-block",
        sequenceIndex,
        recommendedKeepSourcePage: sequenceIndex + 1,
        pages: [0, 1, 2, 3]
          .filter((track) => !(track === 2 && sequenceIndex === 2))
          .map((track) => ({
            sourcePage: track * 5 + sequenceIndex + 1,
            exactSha256: track === 0,
            visualSimilarity: track === 0 ? 1 : .92,
            detailHashDistance: track === 0 ? 0 : 11,
            lowInformation: false,
            editionTrackId: `edition-block-t${track}`,
            editionTrackOrdinal: track,
          })),
      })),
    };
    const onPlan = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing" as const, reason: "test" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <InternalDuplicateDialog open={false} review={editionReview} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onRescan={vi.fn()} onPlan={onPlan} onApply={vi.fn()} onUndo={vi.fn()} />,
    ));
    expect(container.textContent).toContain("남길 판본 세트 선택 · 복수 선택 가능");
    expect(container.querySelectorAll('.internal-edition-track-control input[type="checkbox"]')).toHaveLength(4);
    expect(container.querySelectorAll('input[type="radio"]')).toHaveLength(0);
    expect(container.querySelector(".internal-review-scroll")).toHaveAttribute("data-scroll-axis", "vertical");
    expect(container.querySelector(".internal-scene-matrix")).toHaveAttribute("data-scroll-axis", "horizontal");
    expect(container.querySelector(".internal-scene-matrix")?.closest(".internal-review-scroll")).not.toBeNull();
    expect(container.querySelector(".internal-track-selector")).toBeNull();
    expect(container.querySelectorAll(".internal-scene-matrix-row")).toHaveLength(5);
    expect(container.querySelectorAll(".internal-edition-track-row")).toHaveLength(4);
    expect(container.querySelectorAll(".internal-scene-cell")).toHaveLength(20);
    expect(container.querySelectorAll(".internal-page-option")).toHaveLength(0);
    const trackRows = container.querySelectorAll<HTMLElement>(".internal-edition-track-row");
    expect(trackRows[0]?.querySelector(".internal-edition-track-control")).toHaveTextContent("세트 A1–5p · 5/5장");
    expect(trackRows[2]?.querySelector(".internal-edition-track-control")).toHaveTextContent("세트 C11–15p · 4/5장1개 장면 누락");
    expect([...trackRows[0]!.querySelectorAll(".internal-page-image > span")].map((page) => page.textContent)).toEqual(["1p", "2p", "3p", "4p", "5p"]);
    expect([...trackRows[1]!.querySelectorAll(".internal-page-image > span")].map((page) => page.textContent)).toEqual(["6p", "7p", "8p", "9p", "10p"]);
    expect(trackRows[0]).toHaveClass("is-kept");
    expect(trackRows[1]).toHaveClass("is-quarantine");
    const trackInputs = container.querySelectorAll<HTMLInputElement>('input[name="track-edition-block"]');
    const setA = trackInputs[0];
    const setC = trackInputs[2];
    await act(async () => setC?.click());
    expect(setA).toBeChecked();
    expect(setC).toBeChecked();
    expect(container.querySelectorAll(".internal-edition-track-row")[0]).toHaveClass("is-kept");
    expect(container.querySelectorAll(".internal-edition-track-row")[2]).toHaveClass("is-kept");
    expect(container.querySelectorAll(".internal-edition-track-row")[1]).toHaveClass("is-quarantine");
    const preview = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("격리 계획 미리보기"));
    await act(async () => preview?.click());
    expect(onPlan).toHaveBeenCalledWith(expect.objectContaining({
      selections: expect.arrayContaining([
        expect.objectContaining({ groupId: "edition-row-1", keepSourcePage: 1, removeSourcePages: [6, 16] }),
        expect.objectContaining({ groupId: "edition-row-3", keepSourcePage: 3, removeSourcePages: [8, 18] }),
      ]),
    }));
    expect(onPlan.mock.calls[0]?.[0].selections).toHaveLength(5);
    expect(onPlan.mock.calls[0]?.[0].selections.reduce(
      (count: number, selection: { removeSourcePages: number[] }) => count + selection.removeSourcePages.length,
      0,
    )).toBe(10);

    onPlan.mockClear();
    await act(async () => setA?.click());
    expect(setA).not.toBeChecked();
    expect(setC).toBeChecked();
    const selectedMissing = container.querySelector(".internal-edition-track-row.is-kept .internal-scene-cell.is-missing");
    expect(selectedMissing).toHaveTextContent("누락 · 행 보존");
    expect(selectedMissing).toHaveAttribute("aria-label", "선택 세트 전체 누락 · 이 행 보존");
    await act(async () => preview?.click());
    expect(onPlan.mock.calls[0]?.[0].selections).toHaveLength(4);
    expect(onPlan.mock.calls[0]?.[0].selections).not.toEqual(expect.arrayContaining([
      expect.objectContaining({ groupId: "edition-row-3" }),
    ]));
    await act(async () => setC?.click());
    expect(setC).toBeChecked();
    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("keeps a six-track edition matrix at the fixed comparison size without restoring page radios", async () => {
    const sixTrackReview: InternalDuplicateReview = {
      ...review,
      groups: Array.from({ length: 2 }, (_, sequenceIndex) => ({
        ...review.groups[0]!,
        groupId: `six-track-row-${sequenceIndex}`,
        blockId: "six-track-block",
        sequenceIndex,
        recommendedKeepSourcePage: sequenceIndex + 1,
        pages: Array.from({ length: 6 }, (_, track) => ({
          sourcePage: track * 2 + sequenceIndex + 1,
          exactSha256: track === 0,
          visualSimilarity: track === 0 ? 1 : .92,
          detailHashDistance: track === 0 ? 0 : 12,
          lowInformation: false,
          editionTrackId: `six-track-block-t${track}`,
          editionTrackOrdinal: track,
        })),
      })),
    };
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing" as const, reason: "test" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <InternalDuplicateDialog open={false} review={sixTrackReview} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onRescan={vi.fn()} onPlan={vi.fn()} onApply={vi.fn()} onUndo={vi.fn()} />,
    ));

    const matrix = container.querySelector<HTMLElement>(".internal-scene-matrix");
    expect(matrix?.style.getPropertyValue("--internal-scene-count")).toBe("2");
    expect(container.querySelector(".internal-track-selector")).toBeNull();
    expect(container.querySelectorAll('.internal-edition-track-control input[type="checkbox"]')).toHaveLength(6);
    expect(container.querySelectorAll('.internal-edition-track-control input[type="radio"]')).toHaveLength(0);
    expect(container.querySelectorAll(".internal-scene-matrix-row")).toHaveLength(7);
    expect(container.querySelectorAll(".internal-edition-track-row")).toHaveLength(6);
    expect(container.querySelectorAll(".internal-scene-cell")).toHaveLength(12);
    expect(container.querySelectorAll(".internal-page-option")).toHaveLength(0);
    expect(container.querySelectorAll('.internal-scene-cell input[type="radio"]')).toHaveLength(0);

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });
});
