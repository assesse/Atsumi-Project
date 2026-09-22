import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { galleryId, type DownloadState, type Gallery } from "../core/types";
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
  const sessionTitles = (container: HTMLElement) => [...container.querySelectorAll("#activity-session-panel .activity-item strong")]
    .map((title) => title.textContent);
  const activityGallery = (state: DownloadState, id: number): Gallery => ({
    ...failedGallery,
    id: galleryId(id),
    title: `${state}-${id}`,
    download: { entryId: `entry-${id}`, state },
  });
  const actions = { onClose: vi.fn(), onReview: vi.fn(), onRetry: vi.fn(), onCancel: vi.fn() };

  it("keeps only the latest 50 activities without pinning old queued jobs or restoring the whole queue", async () => {
    const galleries = Array.from({ length: 70 }, (_, index) => activityGallery(index === 0 ? "queued" : "completed", index + 1));
    const sessionDownloads = galleries.map((gallery, index) => ({ galleryId: gallery.id, occurredAt: index }));
    const container = document.createElement("div");
    const root = createRoot(container);
    try {
      await act(async () => root.render(<ActivityDrawer open galleries={galleries} sessionDownloads={sessionDownloads} {...actions} />));
      expect(sessionTitles(container)).toHaveLength(50);
      expect(sessionTitles(container)[0]).toBe("completed-70");
      expect(sessionTitles(container)).not.toContain("queued-1");
      expect(container).toHaveTextContent("최근 50개만 표시합니다");
      await act(async () => root.render(<ActivityDrawer open galleries={galleries} sessionDownloads={[]} {...actions} />));
      expect(sessionTitles(container)).toHaveLength(0);
    } finally { await act(async () => root.unmount()); }
  });

  it("sorts live work and reviews before failures and finished work, newest first within each group", async () => {
    const states: DownloadState[] = [
      "queued", "resolving_metadata", "downloading", "hashing", "verifying", "retry_wait", "review_required",
      "interrupted", "failed", "completed", "quarantined", "cancelled",
    ];
    const galleries = states.map((state, index) => activityGallery(state, index + 1));
    const sessionDownloads = galleries.map((gallery, index) => ({ galleryId: gallery.id, occurredAt: index + 1 }));
    const container = document.createElement("div");
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ActivityDrawer open galleries={galleries} sessionDownloads={sessionDownloads} {...actions} />,
      ));
      expect(sessionTitles(container)).toEqual([
        "review_required-7", "retry_wait-6", "verifying-5", "hashing-4", "downloading-3", "resolving_metadata-2", "queued-1",
        "failed-9", "interrupted-8", "cancelled-12", "quarantined-11", "completed-10",
      ]);
      expect(sessionDownloads.map((activity) => activity.occurredAt)).toEqual(states.map((_, index) => index + 1));
    } finally {
      await act(async () => root.unmount());
    }
  });

  it("reorders immediately using current download state and treats duplicate removal as finished", async () => {
    const active = activityGallery("downloading", 1);
    const review = activityGallery("review_required", 2);
    const completed = activityGallery("completed", 3);
    const sessionDownloads = [active, review, completed].map((gallery) => ({
      galleryId: gallery.id, occurredAt: Number(gallery.id), state: "downloading" as const,
    }));
    const container = document.createElement("div");
    const root = createRoot(container);
    const render = (galleries: Gallery[], excluded: ReadonlySet<Gallery["id"]> = new Set()) => act(async () => root.render(
      <ActivityDrawer open galleries={galleries} sessionDownloads={sessionDownloads} duplicateExcludedGalleryIds={excluded} {...actions} />,
    ));
    try {
      await render([active, review, completed]);
      expect(sessionTitles(container)).toEqual([review.title, active.title, completed.title]);
      await render([active, review, completed], new Set([active.id, review.id]));
      expect(sessionTitles(container)).toEqual([active.title, completed.title, review.title]);
      expect(container.querySelector(".duplicate-resolved strong")).toHaveTextContent(review.title);
      const finished = { ...active, download: { ...active.download!, state: "completed" as const } };
      await render([finished, review, completed]);
      expect(sessionTitles(container)).toEqual([review.title, completed.title, active.title]);
      await render([active, review, completed]);
      expect(sessionTitles(container)).toEqual([review.title, active.title, completed.title]);
    } finally {
      await act(async () => root.unmount());
    }
  });

  it("groups mixed-source session activity without changing automatic history's newest-first ordering", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    const gallery = activityGallery("hashing", 1);
    try {
      await act(async () => root.render(
        <ActivityDrawer open galleries={[gallery]} sessionDownloads={[{ galleryId: gallery.id, occurredAt: 1 }]}
          automaticOverlapActivities={[
            { id: "old", reviewId: "resolved", galleryId: gallery.id, title: "Old failure", detail: "", occurredAt: 2, state: "failed" },
            { id: "resolved", reviewId: "resolved", galleryId: gallery.id, title: "Automatic completed", detail: "", occurredAt: 8, state: "completed" },
            { id: "failed", reviewId: "failed", galleryId: gallery.id, title: "Automatic failure", detail: "", occurredAt: 3, state: "failed" },
          ]}
          danbooruActivities={[
            { id: "danbooru-completed", postId: 4, title: "Danbooru completed", detail: "", occurredAt: 9, state: "completed" },
            { id: "danbooru-failed", postId: 5, title: "Danbooru failure", detail: "", occurredAt: 4, state: "failed" },
          ]}
          {...actions}
        />,
      ));
      expect(sessionTitles(container)).toEqual([
        gallery.title, "Danbooru failure", "Automatic failure", "Danbooru completed", "Automatic completed",
      ]);
      const automationTab = container.querySelectorAll<HTMLButtonElement>('[role="tab"]')[1]!;
      await act(async () => automationTab.click());
      expect([...container.querySelectorAll(".activity-item strong")].map((item) => item.textContent))
        .toEqual(["Automatic completed", "Automatic failure"]);
    } finally {
      await act(async () => root.unmount());
    }
  });

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

    const controls = [...container.querySelectorAll<HTMLButtonElement>(".activity-item .mini-command")];
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

    expect(container).toHaveTextContent("중복 처리 완료 · 목록에서 제외");
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

  it("merges live automatic activity into persistent review history and exposes acknowledgement and list-only restore", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onReviewOverlap = vi.fn();
    const onAcknowledge = vi.fn();
    const onRestore = vi.fn();
    const onLoadMore = vi.fn();

    await act(async () => root.render(
      <ActivityDrawer
        open
        galleries={[]}
        sessionDownloads={[]}
        automaticOverlapActivities={[{
          id: "review-persistent:completed",
          reviewId: "review-persistent",
          galleryId: galleryId(501),
          title: "자동 분류 앨범",
          detail: "자동 분류 완료 · 신규 앨범 B 보존",
          occurredAt: Date.parse("2026-09-04T01:00:00Z"),
          state: "completed",
        }]}
        automationHistory={[{
          reviewId: "review-persistent",
          incomingGalleryId: galleryId(501),
          title: "자동 분류 앨범",
          occurredAt: "2026-09-04T01:00:00Z",
          reviewState: "resolved",
          removeIncomingCount: 0,
          removeExistingCount: 2,
          removedGalleryIds: [galleryId(502), galleryId(503)],
        }]}
        automationHistoryTotalItems={2}
        automationHistoryUnacknowledgedItems={1}
        onClose={vi.fn()}
        onReview={vi.fn()}
        onReviewOverlap={onReviewOverlap}
        onAcknowledgeAutomationHistory={onAcknowledge}
        onRestoreAutomationExclusions={onRestore}
        onLoadMoreAutomationHistory={onLoadMore}
        onRetry={vi.fn()}
        onCancel={vi.fn()}
      />,
    ));

    const historyTab = [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
      .find((button) => button.textContent?.includes("자동분류 검토"));
    expect(historyTab).toHaveTextContent("1");
    await act(async () => historyTab?.click());

    const historyPanel = container.querySelector("#activity-automation-panel");
    expect(historyPanel?.querySelectorAll("article")).toHaveLength(1);
    expect(historyPanel).toHaveTextContent("자동 분류 완료 · 신규 앨범 B 보존");
    expect(historyPanel).toHaveTextContent("격리된 실제 파일은 복원하지 않습니다");
    const buttons = [...(historyPanel?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    await act(async () => {
      buttons.find((button) => button.textContent === "근거 보기")?.click();
      buttons.find((button) => button.textContent === "목록에 복원")?.click();
      buttons.find((button) => button.textContent === "확인 완료")?.click();
      buttons.find((button) => button.textContent === "더 보기")?.click();
    });
    expect(onReviewOverlap).toHaveBeenCalledWith("review-persistent", galleryId(501));
    expect(onRestore).toHaveBeenCalledWith("review-persistent", [galleryId(502), galleryId(503)]);
    expect(onAcknowledge).toHaveBeenCalledWith("review-persistent");
    expect(onLoadMore).toHaveBeenCalledOnce();
    expect(buttons.find((button) => button.textContent === "목록에 복원")).toHaveAttribute(
      "title",
      expect.stringContaining("격리된 실제 파일은 복원하지 않습니다"),
    );

    await act(async () => root.unmount());
    container.remove();
  });

  it("starts sequential review for all unread records and disables duplicate starts", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const start = vi.fn();
    const render = async (count: number, loading = false, pending = false) => {
      await act(async () => root.render(
        <ActivityDrawer open galleries={[]} sessionDownloads={[]}
          automationHistoryUnacknowledgedItems={count} automationSequenceLoading={loading}
          automationHistoryPendingReviewIds={new Set(pending ? ["review-busy"] : [])}
          onStartAutomationSequence={start} onClose={vi.fn()} onReview={vi.fn()} onRetry={vi.fn()} onCancel={vi.fn()} />,
      ));
    };
    await render(250);
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
      .find((button) => button.textContent?.includes("자동분류 검토"))!.click());
    const button = container.querySelector<HTMLButtonElement>(".activity-history-toolbar button")!;
    expect(button).toBeEnabled();
    expect(button).toHaveTextContent("미확인 순차 검토");
    await act(async () => button.click());
    expect(start).toHaveBeenCalledOnce();
    await render(250, true);
    expect(button).toBeDisabled();
    expect(button).toHaveTextContent("검토 목록 준비 중");
    await act(async () => button.click());
    await render(250, false, true);
    expect(button).toBeDisabled();
    await render(0);
    expect(button).toBeDisabled();
    expect(start).toHaveBeenCalledOnce();
    await act(async () => root.unmount());
    container.remove();
  });

  it("includes concise Danbooru activity in the shared feed", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <ActivityDrawer
        open
        galleries={[]}
        sessionDownloads={[]}
        danbooruActivities={[{
          id: "danbooru-6907632",
          postId: 6_907_632,
          title: "sample_artist · #6907632",
          detail: "원본 저장 완료",
          occurredAt: 3,
          state: "completed",
        }]}
        onClose={vi.fn()}
        onReview={vi.fn()}
        onRetry={vi.fn()}
        onCancel={vi.fn()}
      />,
    ));

    expect(container.querySelector("#activity-panel")).toHaveAccessibleName("활동 기록");
    expect(container).toHaveTextContent("sample_artist · #6907632");
    expect(container).toHaveTextContent("원본 저장 완료");
    expect(container).toHaveTextContent("Danbooru #6907632");

    await act(async () => root.unmount());
    container.remove();
  });
});
