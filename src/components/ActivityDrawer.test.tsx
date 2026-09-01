import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { ActivityDrawer } from "./ActivityDrawer";

const failedGallery: Gallery = {
  id: galleryId(42),
  title: "Failure evidence",
  subtitle: "",
  artist: "serein",
  pages: 12,
  score: 0,
  publishedAt: "2026-08-14",
  coverIndex: 0,
  language: "korean",
  tags: [],
  series: [],
  characters: [],
  download: {
    entryId: "entry-42",
    revision: 7,
    state: "failed",
    attempt: 3,
    errorCode: "SOURCE_TIMEOUT",
    errorMessage: "원본 서버 응답이 제한 시간을 초과했습니다.",
  },
};

describe("ActivityDrawer download controls", () => {
  it("truncates fractional progress for the visible and accessible value", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const gallery: Gallery = {
      ...failedGallery,
      download: {
        ...failedGallery.download!,
        state: "downloading",
        progress: 41.66666666666667,
      },
    };

    await act(async () => root.render(
      <ActivityDrawer
        open
        galleries={[gallery]}
        sessionDownloads={[{ galleryId: gallery.id, occurredAt: 1 }]}
        onClose={vi.fn()}
        onReview={vi.fn()}
        onRetry={vi.fn()}
        onCancel={vi.fn()}
      />,
    ));

    const progress = container.querySelector('[role="progressbar"]');
    expect(progress).toHaveTextContent("41%");
    expect(progress).toHaveAttribute("aria-valuenow", "41");
    expect(container.textContent).not.toContain("41.666");

    await act(async () => root.unmount());
    container.remove();
  });

  it("shows persisted failure evidence and invokes retry/cancel once", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onRetry = vi.fn();
    const onCancel = vi.fn();

    await act(async () => root.render(
      <ActivityDrawer
        open
        galleries={[failedGallery]}
        sessionDownloads={[{ galleryId: failedGallery.id, occurredAt: 1 }]}
        onClose={vi.fn()}
        onReview={vi.fn()}
        onRetry={onRetry}
        onCancel={onCancel}
      />,
    ));

    expect(container.textContent).toContain("원본 서버 응답이 제한 시간을 초과했습니다.");
    expect(container.textContent).toContain("시도 3 · SOURCE_TIMEOUT");
    const buttons = [...container.querySelectorAll<HTMLButtonElement>(".mini-command")];
    const retry = buttons.find((button) => button.textContent === "재시도");
    const cancel = buttons.find((button) => button.textContent === "취소");
    await act(async () => {
      retry?.click();
      cancel?.click();
    });
    expect(onRetry).toHaveBeenCalledOnce();
    expect(onRetry).toHaveBeenCalledWith(failedGallery.id);
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledWith(failedGallery.id);

    await act(async () => root.unmount());
    container.remove();
  });

  it("disables mutation controls while the entry is pending", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <ActivityDrawer
        open
        galleries={[failedGallery]}
        sessionDownloads={[{ galleryId: failedGallery.id, occurredAt: 1 }]}
        pendingEntryIds={new Set(["entry-42"])}
        onClose={vi.fn()}
        onReview={vi.fn()}
        onRetry={vi.fn()}
        onCancel={vi.fn()}
      />,
    ));

    const controls = [...container.querySelectorAll<HTMLButtonElement>(".mini-command")];
    expect(controls).toHaveLength(2);
    expect(controls.every((button) => button.disabled)).toBe(true);

    await act(async () => root.unmount());
    container.remove();
  });

  it("marks a duplicate-excluded download as processed without retry controls", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onRetry = vi.fn();
    const onCancel = vi.fn();

    await act(async () => root.render(
      <ActivityDrawer
        open
        galleries={[failedGallery]}
        sessionDownloads={[{ galleryId: failedGallery.id, occurredAt: 1 }]}
        duplicateExcludedGalleryIds={new Set([failedGallery.id])}
        onClose={vi.fn()}
        onReview={vi.fn()}
        onRetry={onRetry}
        onCancel={onCancel}
      />,
    ));

    expect(container).toHaveTextContent("중복 검토 완료 · 탐색 및 다운로드 목록에서 제외됨");
    expect(container).toHaveTextContent("처리 완료");
    expect(container).not.toHaveTextContent("SOURCE_TIMEOUT");
    expect(container).not.toHaveTextContent("재시도");
    expect(container).not.toHaveTextContent("취소");
    expect(container.querySelector('[role="progressbar"]')).toBeNull();
    expect(onRetry).not.toHaveBeenCalled();
    expect(onCancel).not.toHaveBeenCalled();

    await act(async () => root.unmount());
    container.remove();
  });

  it("shows only this-run downloads and opens an automatic decision by its persisted review id", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onReviewOverlap = vi.fn();
    const unrelated = { ...failedGallery, id: galleryId(77), title: "Old database job" };

    await act(async () => root.render(
      <ActivityDrawer
        open
        galleries={[failedGallery, unrelated]}
        sessionDownloads={[{ galleryId: failedGallery.id, occurredAt: 1 }]}
        automaticOverlapActivities={[{
          id: "automatic-review-1",
          reviewId: "review-1",
          galleryId: failedGallery.id,
          title: "Failure evidence",
          detail: "자동 분류 완료 · 신규 앨범 B 보존",
          occurredAt: 2,
          state: "completed",
        }]}
        onClose={vi.fn()}
        onReview={vi.fn()}
        onReviewOverlap={onReviewOverlap}
        onRetry={vi.fn()}
        onCancel={vi.fn()}
      />,
    ));

    expect(container).not.toHaveTextContent("Old database job");
    expect(container).toHaveTextContent("자동 분류 완료 · 신규 앨범 B 보존");
    await act(async () => {
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === "근거 보기")?.click();
    });
    expect(onReviewOverlap).toHaveBeenCalledWith("review-1", failedGallery.id);

    await act(async () => root.unmount());
    container.remove();
  });
});
