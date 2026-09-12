import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { DownloadOverlapAutomationHistoryItem, DownloadOverlapReview } from "../api/contracts";
import { galleryId } from "../core/types";
import type { DownloadOverlapContainmentGroup } from "../state/downloadOverlapContainment";
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

const containmentFixture = (): {
  review: DownloadOverlapReview;
  group: DownloadOverlapContainmentGroup;
} => {
  const review = fixture();
  review.incoming = {
    ...review.incoming,
    galleryId: galleryId(3003760),
    title: "Compilation edition",
    pageCount: 155,
  };
  const containedPageCounts = [17, 9, 9];
  const completeCandidates = containedPageCounts.map((pageCount, index) => ({
    ...review.candidates[index]!,
    candidateId: `contained-${index + 1}`,
    existing: {
      ...review.candidates[index]!.existing,
      galleryId: galleryId(3003700 + index),
      title: `Contained edition ${index + 1}`,
      pageCount,
    },
    relation: "incoming_contains_existing" as const,
    confidence: 0.97,
    matchedPages: pageCount,
    exactPages: pageCount,
    visualPages: 0,
    existingCoverage: 1,
    incomingCoverage: pageCount / 155,
    existingUniquePages: 0,
    incomingUniquePages: 155 - pageCount,
    longestAlignedRun: pageCount,
    rank: index + 1,
    pagePairs: Array.from({ length: pageCount }, (_, pageIndex) => ({
      ...review.candidates[index]!.pagePairs[0]!,
      incomingSourcePage: pageIndex + 20 * index + 1,
      existingSourcePage: pageIndex + 1,
      exactSha256: true,
      lowInformation: false,
    })),
  }));
  const ambiguous = {
    ...review.candidates[3]!,
    candidateId: "ambiguous-156",
    existing: {
      ...review.candidates[3]!.existing,
      galleryId: galleryId(3003999),
      title: "Similar 156 page edition",
      pageCount: 156,
    },
    relation: "partial_overlap" as const,
    confidence: 0.91,
    matchedPages: 140,
    exactPages: 80,
    visualPages: 60,
    existingCoverage: 140 / 156,
    incomingCoverage: 140 / 155,
    existingUniquePages: 16,
    incomingUniquePages: 15,
    longestAlignedRun: 60,
    rank: 4,
  };
  review.candidates = [...completeCandidates, ambiguous];
  const group: DownloadOverlapContainmentGroup = {
    keeper: review.incoming,
    items: completeCandidates.map((candidate) => ({
      key: `${review.reviewId}:${candidate.candidateId}`,
      review,
      candidate,
      action: "remove_existing_continue",
      excluded: candidate.existing,
      keeperIsIncoming: true,
    })),
  };
  return { review, group };
};

const pageMergeFixture = (): DownloadOverlapReview => {
  const review = fixture();
  review.incoming = { ...review.incoming, pageCount: 12 };
  const original = review.candidates[0]!;
  const candidate = {
    ...original,
    existing: { ...original.existing, pageCount: 10 },
    matchedPages: 10,
    exactPages: 4,
    visualPages: 6,
    existingCoverage: 1,
    incomingCoverage: 10 / 12,
    existingUniquePages: 0,
    incomingUniquePages: 2,
    longestAlignedRun: 10,
    pagePairs: Array.from({ length: 10 }, (_, index) => ({
      ...original.pagePairs[0]!,
      existingSourcePage: index + 1,
      incomingSourcePage: index + 1,
    })),
  };
  review.candidates = [candidate];
  return review;
};

describe("DownloadOverlapReviewDialog", () => {
  it("selects mapped source pages with Ctrl+click and submits a sorted merge request", async () => {
    const review = pageMergeFixture();
    const onMergePages = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = async (decisionPending = false) => act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        decisionPending={decisionPending}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
        onMergePages={onMergePages}
      />,
    ));

    await render();
    const existingPage = (page: number) => container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="기존 A ${page}페이지"]`)!;
    const incomingPage = (page: number) => container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="신규 B ${page}페이지"]`)!;
    await act(async () => existingPage(3).dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(container.querySelector(".download-overlap-page-cell.is-merge-source")).toBeNull();

    await act(async () => existingPage(3).dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));
    await act(async () => existingPage(1).dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));
    expect(existingPage(1)).toHaveClass("is-merge-source");
    expect(existingPage(3)).toHaveClass("is-merge-source");
    expect(incomingPage(1)).toHaveClass("is-merge-target");
    expect(incomingPage(3)).toHaveClass("is-merge-target");
    expect(incomingPage(11)).toHaveClass("is-unique");
    expect(incomingPage(11)).not.toHaveClass("is-merge-target");
    expect(container.querySelectorAll('[data-merge-state="source"]')).toHaveLength(2);
    expect(container.querySelectorAll('[data-merge-state="target"]')).toHaveLength(2);
    expect(container.querySelector(".download-overlap-action-note")).toHaveTextContent("기존 A 2장 → 신규 B 대응 2장 교체");
    expect(container.querySelector(".download-overlap-action-note")).toHaveTextContent("추가 페이지는 그대로 둡니다");
    expect(container.querySelector(".download-overlap-action-note")).toHaveTextContent("교체 전 대상 파일은 백업합니다");
    expect(container.querySelector(".download-overlap-action-note")).toHaveTextContent("기존 A 앨범은 제외하되 원본 파일은 보존합니다");
    expect(container.textContent).not.toContain("검토 미루기");
    for (const label of ["기존 A 제거", "신규 B 제거", "오탐 판정", "둘 다 보존"]) {
      const button = [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((item) => item.textContent === label)!;
      expect(button).toBeDisabled();
    }

    await render(true);
    expect(container.querySelector<HTMLButtonElement>(".download-overlap-merge-apply")).toBeDisabled();
    await render(false);
    const mergeButton = container.querySelector<HTMLButtonElement>(".download-overlap-merge-apply")!;
    expect(mergeButton).toHaveTextContent("선택한 2장 병합");
    await act(async () => mergeButton.click());
    expect(onMergePages).toHaveBeenCalledWith({
      reviewId: "review-overlap",
      expectedRevision: 4,
      candidateId: "candidate-1",
      sourceSide: "existing",
      sourcePages: [1, 3],
    });

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("refuses unmapped, lossy-source, and opposite-side page merge selections with an explanation", async () => {
    const review = pageMergeFixture();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
        onMergePages={vi.fn()}
      />,
    ));
    const page = (side: "기존 A" | "신규 B", number: number) =>
      container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="${side} ${number}페이지"]`)!;
    const ctrlClick = async (element: HTMLElement) => act(async () =>
      element.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));

    await ctrlClick(page("신규 B", 11));
    expect(container.querySelector(".download-overlap-merge-guide")).toHaveTextContent("대응 위치가 없어 병합할 수 없습니다");
    expect(container.querySelector(".download-overlap-merge-apply")).toBeNull();

    await ctrlClick(page("신규 B", 1));
    expect(container.querySelector(".download-overlap-merge-guide")).toHaveTextContent("신규 B 전체 페이지 중 2장의 대응을 확인할 수 없어");
    expect(container.querySelector(".download-overlap-merge-guide")).toHaveTextContent("자동 제외를 전제로 한 병합이 불가능합니다");

    await ctrlClick(page("기존 A", 1));
    expect(page("기존 A", 1)).toHaveClass("is-merge-source");
    await ctrlClick(page("신규 B", 1));
    expect(container.querySelector(".download-overlap-merge-guide")).toHaveTextContent("현재 기존 A를 병합 원본으로 선택 중입니다");
    expect(page("기존 A", 1)).toHaveClass("is-merge-source");
    expect(page("신규 B", 1)).toHaveClass("is-merge-target");

    await act(async () => container.querySelector<HTMLButtonElement>(".download-overlap-merge-clear")!.click());
    expect(container.querySelector(".download-overlap-page-cell.is-merge-source")).toBeNull();
    expect(container.querySelector(".download-overlap-page-cell.is-merge-target")).toBeNull();
    expect(container.querySelector(".download-overlap-merge-guide")).toHaveTextContent("선택을 모두 해제했습니다");
    expect(container.textContent).toContain("검토 미루기");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("blocks a malformed mapping when summary uniqueness says the source is complete", async () => {
    const review = pageMergeFixture();
    const candidate = review.candidates[0]!;
    candidate.existingUniquePages = 0;
    candidate.pagePairs = candidate.pagePairs.slice(0, -1);
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
        onMergePages={vi.fn()}
      />,
    ));

    const mappedCell = container.querySelector<HTMLElement>('.download-overlap-page-cell[aria-label^="기존 A 1페이지"]')!;
    expect(mappedCell).toHaveClass("is-merge-source-blocked");
    await act(async () => mappedCell.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));
    expect(container.querySelector(".download-overlap-merge-guide")).toHaveTextContent("기존 A 전체 페이지 중 1장의 대응을 확인할 수 없어");
    expect(container.querySelector(".download-overlap-merge-guide")).toHaveTextContent("자동 제외를 전제로 한 병합이 불가능합니다");
    expect(container.querySelector(".download-overlap-page-cell.is-merge-source")).toBeNull();
    expect(container.querySelector(".download-overlap-merge-apply")).toBeNull();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("clears page merge selection when the candidate or review revision changes", async () => {
    const review = pageMergeFixture();
    const first = review.candidates[0]!;
    review.candidates = [first, {
      ...first,
      candidateId: "candidate-second",
      existing: { ...first.existing, entryId: "existing-second", galleryId: galleryId(98), title: "Second owned edition" },
      existingFingerprint: "second-fingerprint",
      rank: 2,
    }];
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = async (currentReview: DownloadOverlapReview) => act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={currentReview}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
        onMergePages={vi.fn()}
      />,
    ));
    const selectFirstExistingPage = async () => {
      const cell = container.querySelector<HTMLElement>('.download-overlap-page-cell[aria-label^="기존 A 1페이지"]')!;
      await act(async () => cell.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));
    };

    await render(review);
    await selectFirstExistingPage();
    expect(container.querySelector(".download-overlap-merge-apply")).not.toBeNull();
    const secondTab = [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
      .find((button) => button.textContent?.includes("#98"))!;
    await act(async () => secondTab.click());
    expect(container.querySelector(".download-overlap-merge-apply")).toBeNull();
    expect(container.querySelector(".download-overlap-page-cell.is-merge-source")).toBeNull();

    await selectFirstExistingPage();
    expect(container.querySelector(".download-overlap-merge-apply")).not.toBeNull();
    await render({ ...review, revision: 5 });
    expect(container.querySelector(".download-overlap-merge-apply")).toBeNull();
    expect(container.querySelector(".download-overlap-page-cell.is-merge-source")).toBeNull();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("shows verified contained editions together and keeps ambiguous candidates in direct review", async () => {
    const { review, group } = containmentFixture();
    const onApplyContainmentBatch = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        containmentGroup={group}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
        onApplyContainmentBatch={onApplyContainmentBatch}
      />,
    ));

    const batch = container.querySelector<HTMLElement>(".download-overlap-containment")!;
    expect(batch).not.toBeNull();
    expect(batch.textContent).toContain("합본 포함 검토");
    expect(batch.textContent).toContain("보존 합본");
    expect(batch.textContent).toContain("Compilation edition");
    expect(batch.textContent).toContain("완전 포함 3개");
    expect(batch.textContent).toContain("17p → 합본 155p");
    expect(batch.textContent).toContain("제외본 1~17p · 합본 1~17p");
    expect(batch.textContent).toContain("별도 직접 검토 1개");
    expect(batch.textContent).toContain("완전 포함이 확인되지 않아 일괄 제외에 포함하지 않았습니다");

    const checks = [...batch.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
    expect(checks).toHaveLength(3);
    expect(checks.every((check) => check.checked)).toBe(true);
    await act(async () => checks[1]!.click());
    expect(checks[1]).not.toBeChecked();

    const evidenceButton = [...batch.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "페이지 근거 보기")!;
    await act(async () => evidenceButton.click());
    expect(batch.querySelector(".download-overlap-containment-evidence .download-overlap-page-map")).not.toBeNull();
    expect(evidenceButton).toHaveAttribute("aria-expanded", "true");

    const ambiguousButton = [...batch.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("#3003999"))!;
    await act(async () => ambiguousButton.click());
    const activeTab = container.querySelector<HTMLButtonElement>('.download-overlap-candidate-tabs [role="tab"][aria-selected="true"]');
    expect(activeTab?.textContent).toContain("#3003999");

    const applyButton = [...batch.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("제외 · 합본 보존"))!;
    expect(applyButton.textContent).toContain("선택한 2개");
    await act(async () => applyButton.click());
    expect(onApplyContainmentBatch).toHaveBeenCalledWith([
      "review-overlap:contained-1",
      "review-overlap:contained-3",
    ]);

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("locks a running containment batch and preserves only unfinished selections after a partial failure", async () => {
    const { review, group } = containmentFixture();
    const onApplyContainmentBatch = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = async (props: {
      decisionPending?: boolean;
      error?: string;
      completed?: number;
      currentReview?: DownloadOverlapReview;
      currentGroup?: DownloadOverlapContainmentGroup;
    } = {}) => {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog
          open={false}
          review={props.currentReview ?? review}
          containmentGroup={props.currentGroup ?? group}
          decisionPending={props.decisionPending}
          error={props.error}
          batchProgress={props.completed === undefined ? undefined : { completed: props.completed, total: 2 }}
          previewWidth={220}
          thumbnailClient={client}
          onClose={vi.fn()}
          onRetry={vi.fn()}
          onDecision={vi.fn()}
          onApplyContainmentBatch={onApplyContainmentBatch}
        />,
      ));
    };

    await render();
    let batch = container.querySelector<HTMLElement>(".download-overlap-containment")!;
    let checks = [...batch.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
    await act(async () => checks[1]!.click());
    let applyButton = [...batch.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("제외 · 합본 보존"))!;
    await act(async () => applyButton.click());

    await render({ decisionPending: true, completed: 1 });
    batch = container.querySelector<HTMLElement>(".download-overlap-containment")!;
    expect(batch).toHaveAttribute("aria-busy", "true");
    expect(batch.textContent).toContain("선택 항목 처리 중 · 1/2");
    applyButton = [...batch.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("제외 처리 중"))!;
    expect(applyButton).toBeDisabled();

    const refreshedReview = { ...review, revision: review.revision + 1 };
    const refreshedGroup: DownloadOverlapContainmentGroup = {
      keeper: refreshedReview.incoming,
      items: group.items.map((item, index) => ({
        ...item,
        review: refreshedReview,
        candidate: refreshedReview.candidates[index]!,
        excluded: refreshedReview.candidates[index]!.existing,
      })),
    };
    await render({ error: "두 번째 판본 처리 실패", completed: 1, currentReview: refreshedReview, currentGroup: refreshedGroup });
    batch = container.querySelector<HTMLElement>(".download-overlap-containment")!;
    expect(batch.textContent).toContain("일부 처리 후 중단 · 1/2 완료");
    expect(batch.textContent).toContain("처리되지 않은 선택은 유지됩니다");
    checks = [...batch.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
    expect(checks[0]).toBeDisabled();
    expect(checks[0]).not.toBeChecked();
    expect(checks[1]).not.toBeChecked();
    expect(checks[2]).toBeChecked();
    applyButton = [...batch.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("제외 · 합본 보존"))!;
    expect(applyButton.textContent).toContain("선택한 1개");
    await act(async () => applyButton.click());
    expect(onApplyContainmentBatch).toHaveBeenLastCalledWith(["review-overlap:contained-3"]);

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("explains stale evidence as a read-only recheck instead of another actionable failure", async () => {
    const review = fixture();
    review.state = "stale";
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

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

    expect(container.querySelector(".review-signal")).toHaveTextContent("보유 목록 재검사 중");
    expect(container.querySelector(".download-overlap-stale-note")).toHaveTextContent("이미 제외된 상대를 정리하고 현재 보유 목록으로 다시 검사 중입니다");
    expect(container.textContent).not.toContain("기존 A 제거");
    expect(container.textContent).toContain("읽기 전용 판정 기록");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("shows the strict recommendation only for an eligible review", async () => {
    const review = fixture();
    review.incoming.pageCount = 25;
    const eligible = review.candidates[1]!;
    eligible.existing.pageCount = 20;
    eligible.matchedPages = 20;
    eligible.exactPages = 20;
    eligible.visualPages = 0;
    eligible.existingCoverage = 1;
    eligible.incomingCoverage = 0.8;
    eligible.existingUniquePages = 0;
    eligible.incomingUniquePages = 5;
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

    const banner = container.querySelector(".download-overlap-auto-recommendation")!;
    expect(banner).toHaveTextContent("추천");
    expect(banner).toHaveTextContent("신규 B 유지 · 기존 #101 제외 (완전 포함)");
    expect(banner).toHaveTextContent("직접 선택해야 적용됩니다.");
    expect(banner.textContent!.length).toBeLessThan(100);
    expect(banner).not.toHaveTextContent("신뢰도 85%");
    expect(container.textContent).not.toContain("일반 판본: 포함률 95% 이상");

    const help = banner.querySelector<HTMLButtonElement>('[aria-label="자동 판정 기준"]')!;
    expect(help).not.toHaveAttribute("aria-describedby");
    await act(async () => help.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    let tooltip = document.getElementById(help.getAttribute("aria-describedby")!);
    expect(tooltip).toHaveAttribute("role", "tooltip");
    expect(tooltip?.parentElement).toBe(container.querySelector("dialog"));
    expect(tooltip).toHaveTextContent("작은 판본의 모든 페이지 대응");
    expect(tooltip).toHaveTextContent("일반 판본: 포함률 95% 이상");
    expect(tooltip).toHaveTextContent("제목 표식(무검열 > 미표시 > 검열) → 페이지 수 → 기존본");
    expect(banner).not.toContainElement(tooltip);
    await act(async () => help.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));
    expect(help).not.toHaveAttribute("aria-describedby");
    await act(async () => help.focus());
    tooltip = document.getElementById(help.getAttribute("aria-describedby")!);
    expect(tooltip).toHaveTextContent("횟수 제한 없이 재검증 후 적용");
    expect(tooltip).toHaveTextContent("영구 삭제하지 않습니다");
    expect(tooltip).not.toHaveTextContent("하루 10건");
    await act(async () => help.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(help).not.toHaveAttribute("aria-describedby");

    const renderWith = async (autoMode: "off" | "recommend" | "strict_quarantine", current = review) => {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog
          open={false} review={current} autoMode={autoMode} previewWidth={220}
          thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()}
        />,
      ));
    };
    await renderWith("strict_quarantine");
    expect(container.querySelector(".download-overlap-auto-recommendation")).toHaveTextContent("자동 정리 예정");
    expect(container.querySelector(".download-overlap-auto-recommendation")).toHaveTextContent("닫으면 재검증 후 적용 · 영구 삭제 없음");
    const defer = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "검토 미루기")!;
    expect(document.getElementById(defer.getAttribute("aria-describedby")!)).toHaveTextContent("자동 기준을 충족한 검토가 재검증 후 처리될 수 있습니다");
    await renderWith("off");
    expect(container.querySelector(".download-overlap-auto-recommendation")).toBeNull();
    for (const state of ["resolved", "cancelled", "stale"] as const) {
      await renderWith("recommend", { ...review, state });
      expect(container.querySelector(".download-overlap-auto-recommendation")).toBeNull();
    }
    await renderWith("recommend", { ...review, candidates: [{ ...eligible, confidence: 0.5 }] });
    expect(container.querySelector(".download-overlap-auto-recommendation")).toBeNull();
    await renderWith("recommend", { ...review, candidates: [{ ...eligible, decision: "keep_both" }] });
    expect(container.querySelector(".download-overlap-auto-recommendation")).toBeNull();

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
    expect(container.querySelector(".download-overlap-result")).toBeNull();
    expect(container.querySelector(".download-overlap-artifact.is-kept")).toBeNull();
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

  it.each(["resolved", "cancelled", "pending"] as const)("allows restoring and acknowledging automatic %s evidence without a footer close button", async (state) => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = { ...fixture(), state };
    const removedGalleryIds = state === "cancelled"
      ? [review.incoming.galleryId]
      : [review.candidates[0]!.existing.galleryId, review.candidates[2]!.existing.galleryId];
    const history: DownloadOverlapAutomationHistoryItem = {
      reviewId: review.reviewId,
      incomingGalleryId: review.incoming.galleryId,
      title: review.incoming.title,
      occurredAt: review.updatedAt,
      reviewState: state,
      removeIncomingCount: state === "cancelled" ? 1 : 0,
      removeExistingCount: state === "cancelled" ? 0 : 2,
      removedGalleryIds,
    };
    const onRestore = vi.fn();
    const onAcknowledge = vi.fn();
    const onClose = vi.fn();
    const onDecision = vi.fn();
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false} review={review} previewWidth={220} thumbnailClient={client}
        automationHistoryItem={history} onRestoreAutomationExclusions={onRestore}
        onAcknowledgeAutomationHistory={onAcknowledge}
        onClose={onClose} onRetry={vi.fn()} onDecision={onDecision}
      />,
    ));
    const footer = container.querySelector('.download-overlap-history-actions')!;
    const buttons = [...footer.querySelectorAll<HTMLButtonElement>("button")];
    const restore = buttons.find((button) => button.textContent?.includes("목록에 복원"))!;
    const acknowledge = buttons.find((button) => button.textContent?.includes("확인 완료"))!;
    expect(restore).toBeEnabled();
    expect(acknowledge).toBeEnabled();
    expect(document.getElementById(restore.getAttribute("aria-describedby")!)).toHaveTextContent("격리된 실제 파일은 복원하지 않습니다");
    expect(document.getElementById(acknowledge.getAttribute("aria-describedby")!)).toHaveTextContent("앨범이나 파일은 변경하지 않습니다");
    expect(container.querySelector('.download-overlap-readonly-actions')).toBeNull();
    expect(container.querySelectorAll('button[aria-label="닫기"]')).toHaveLength(1);
    expect([...container.querySelectorAll(".review-actions button")].some((button) => button.textContent === "닫기")).toBe(false);
    expect(Boolean(container.querySelector(".download-overlap-actions"))).toBe(state === "pending");

    // Selecting another comparison must not replace the persisted removal targets.
    await act(async () => container.querySelectorAll<HTMLButtonElement>('[role="tab"]')[1]!.click());
    await act(async () => restore.click());
    await act(async () => acknowledge.click());
    expect(onRestore).toHaveBeenCalledExactlyOnceWith(review.reviewId, removedGalleryIds);
    expect(onAcknowledge).toHaveBeenCalledExactlyOnceWith(review.reviewId);
    expect(onDecision).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("navigates sequential evidence without acknowledging, including loading or failed items", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const previous = vi.fn();
    const next = vi.fn();
    const acknowledge = vi.fn();
    const review = { ...fixture(), state: "resolved" as const };
    const history: DownloadOverlapAutomationHistoryItem = {
      reviewId: review.reviewId, incomingGalleryId: review.incoming.galleryId,
      title: review.incoming.title, occurredAt: review.updatedAt, reviewState: review.state,
      removeIncomingCount: 0, removeExistingCount: 1, removedGalleryIds: [galleryId(100)],
    };
    const render = async (position: number, busy = false, loadState: "ready" | "loading" | "error" = "ready") => {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog open review={loadState === "ready" ? review : undefined}
          loading={loadState === "loading"} error={loadState === "error" ? "근거를 불러오지 못했습니다" : null}
          previewWidth={220} thumbnailClient={client} automationHistoryItem={history}
          automationHistoryPending={busy} onAcknowledgeAutomationHistory={acknowledge}
          automationReviewSequence={{ position, total: 3, canPrevious: position > 1, canNext: position < 3 }}
          onPreviousAutomationReview={previous} onNextAutomationReview={next}
          onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} />,
      ));
    };
    await render(2);
    const nav = container.querySelector('nav[aria-label="자동 분류 순차 검토"]')!;
    const previousButton = nav.querySelector<HTMLButtonElement>('[aria-label="이전 자동 분류"]')!;
    const nextButton = nav.querySelector<HTMLButtonElement>('[aria-label="다음 자동 분류"]')!;
    expect(nav).toHaveTextContent("순차 검토 · 2 / 3");
    const acknowledgeButton = [...container.querySelectorAll<HTMLButtonElement>(".download-overlap-history-actions button")]
      .find((button) => button.textContent?.includes("확인 완료"))!;
    expect(document.getElementById(acknowledgeButton.getAttribute("aria-describedby")!)).toHaveTextContent("다음 미확인 기록으로 이동");
    await act(async () => { previousButton.click(); nextButton.click(); });
    expect(previous).toHaveBeenCalledOnce();
    expect(next).toHaveBeenCalledOnce();
    expect(acknowledge).not.toHaveBeenCalled();
    await render(1);
    expect(previousButton).toBeDisabled();
    await render(3);
    expect(nextButton).toBeDisabled();
    await render(2, true);
    expect(previousButton).toBeDisabled();
    expect(nextButton).toBeDisabled();
    for (const state of ["loading", "error"] as const) {
      await render(2, false, state);
      expect(nav).toBeInTheDocument();
      expect(previousButton).toBeEnabled();
      expect(nextButton).toBeEnabled();
      expect(container.querySelector(".download-overlap-history-actions")).toBeNull();
    }
    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("guards history actions while busy and hides them for acknowledged or unrelated records", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    const history: DownloadOverlapAutomationHistoryItem = {
      reviewId: review.reviewId, incomingGalleryId: review.incoming.galleryId,
      title: review.incoming.title, occurredAt: review.updatedAt, reviewState: "pending",
      removeIncomingCount: 0, removeExistingCount: 1,
      removedGalleryIds: [review.candidates[0]!.existing.galleryId],
    };
    const onRestore = vi.fn();
    const onAcknowledge = vi.fn();
    const onDecision = vi.fn();
    const render = async (item = history, pending = false, loading = false) => {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog
          open={false} review={review} previewWidth={220} thumbnailClient={client}
          automationHistoryItem={item} automationHistoryPending={pending} loading={loading}
          onRestoreAutomationExclusions={onRestore} onAcknowledgeAutomationHistory={onAcknowledge}
          onClose={vi.fn()} onRetry={vi.fn()} onDecision={onDecision}
        />,
      ));
    };
    await render(history, true);
    expect(container.querySelector('.download-overlap-history-actions')).toHaveAttribute("aria-busy", "true");
    expect(container.querySelector('.download-overlap-history-status')).toHaveTextContent("처리 중");
    const blocked = [...container.querySelectorAll<HTMLButtonElement>(".download-overlap-history-actions button, .download-overlap-actions .danger-button")];
    for (const button of blocked) {
      expect(button).toBeDisabled();
      await act(async () => button.click());
    }
    expect(onRestore).not.toHaveBeenCalled();
    expect(onAcknowledge).not.toHaveBeenCalled();
    expect(onDecision).not.toHaveBeenCalled();
    await render(history, false, true);
    for (const button of container.querySelectorAll(".download-overlap-history-actions button")) expect(button).toBeDisabled();
    await render({ ...history, acknowledgedAt: "2026-09-10T00:00:00Z" });
    expect(container.querySelector('.download-overlap-history-actions')).toHaveTextContent("확인 완료한 자동 분류 기록");
    expect(container.querySelectorAll(".download-overlap-history-actions button")).toHaveLength(0);
    await render({ ...history, removedGalleryIds: [] });
    const onlyAction = container.querySelectorAll(".download-overlap-history-actions button");
    expect(onlyAction).toHaveLength(1);
    expect(onlyAction[0]).toHaveTextContent("확인 완료");
    await render({ ...history, reviewId: "different-review" });
    expect(container.querySelector('.download-overlap-history-actions')).toBeNull();
    await render({ ...history, incomingGalleryId: galleryId(999) });
    expect(container.querySelector('.download-overlap-history-actions')).toBeNull();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("makes the surviving edition unmistakable in processed multi-candidate evidence", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    review.incoming.title = "New edition [Decensored]";
    review.state = "resolved";
    review.resolvedAt = "2026-08-25T00:02:00.000Z";
    review.candidates = review.candidates.slice(0, 2).map((candidate, index) => ({
      ...candidate,
      decision: index === 0 ? "existing_removed" as const : "keep_both" as const,
    }));
    review.decisions = [{
      candidateId: review.candidates[0]!.candidateId,
      action: "remove_existing_continue",
      actor: "automation",
      reasonCode: "balanced_overlap_v3",
      ruleVersion: 3,
      featureSnapshotJson: JSON.stringify({
        candidateId: review.candidates[0]!.candidateId,
        incomingGalleryId: review.incoming.galleryId,
        existingGalleryId: review.candidates[0]!.existing.galleryId,
        winner: "incoming",
        preferenceReason: "uncensored",
        metrics: { pageDifference: 2 },
      }),
      createdAt: "2026-08-25T00:01:00.000Z",
    }];

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

    const tabs = [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    expect(tabs[0]).toHaveTextContent("A 제외");
    expect(tabs[1]).toHaveTextContent("둘 다 유지");
    expect(container.querySelector(".download-overlap-result")).toBeNull();
    let artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
    expect(artifacts[0]).toHaveClass("is-excluded");
    expect(artifacts[0]).toHaveTextContent("이 검토에서 제외");
    expect(artifacts[0]).toHaveTextContent("상대 판본의 무검열 표식 우선");
    expect(artifacts[1]).toHaveClass("is-kept");
    expect(artifacts[1]).toHaveTextContent("이 검토에서 보존");
    expect(artifacts[1]).toHaveTextContent("무검열 표식 우선 · 신뢰도 94%");
    expect(container.querySelector(".download-overlap-readonly-actions")).toHaveTextContent("읽기 전용 판정 기록");
    expect(container.querySelector(".download-overlap-readonly-actions")).toHaveTextContent("이 창에서는 판정을 바꾸거나 제외를 복구하지 않습니다");

    await act(async () => tabs[1]!.click());

    artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
    expect(artifacts[0]).toHaveClass("is-kept");
    expect(artifacts[1]).toHaveClass("is-kept");
    expect(artifacts[0]).toHaveTextContent("둘 다 보존 선택");
    expect(artifacts[1]).toHaveTextContent("둘 다 보존 선택");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("shows existing A as the survivor when incoming B was cancelled", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    review.state = "cancelled";
    review.resolvedAt = "2026-08-25T00:02:00.000Z";
    review.candidates = [review.candidates[0]!];
    review.decisions = [{
      action: "remove_incoming",
      actor: "human",
      createdAt: "2026-08-25T00:01:00.000Z",
    }];

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

    expect(container.querySelector(".download-overlap-result")).toBeNull();
    const artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
    expect(artifacts[0]).toHaveClass("is-kept");
    expect(artifacts[0]).toHaveAttribute("aria-label", expect.stringContaining("이 검토에서 보존"));
    expect(artifacts[0]).toHaveTextContent("신규 B 제거 선택");
    expect(artifacts[1]).toHaveClass("is-excluded");
    expect(artifacts[1]).toHaveAttribute("aria-label", expect.stringContaining("이 검토에서 제외"));
    expect(artifacts[1]).toHaveTextContent("수동 제거 선택");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it.each(["keep_both", "false_positive"] as const)(
    "shows a later incoming removal as the final result after %s",
    async (firstDecision) => {
      const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
      const container = document.createElement("div");
      document.body.append(container);
      const root = createRoot(container);
      const review = fixture();
      review.state = "cancelled";
      review.resolvedAt = "2026-08-25T00:03:00.000Z";
      review.candidates = review.candidates.slice(0, 2).map((candidate, index) => index === 0
        ? { ...candidate, decision: firstDecision }
        : candidate);
      review.decisions = [{
        candidateId: review.candidates[0]!.candidateId,
        action: firstDecision === "keep_both" ? "keep_both_continue" : "false_positive_continue",
        actor: "human",
        createdAt: "2026-08-25T00:01:00.000Z",
      }, {
        candidateId: review.candidates[1]!.candidateId,
        action: "remove_incoming",
        actor: "human",
        createdAt: "2026-08-25T00:02:00.000Z",
      }];

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

      const firstTab = container.querySelectorAll<HTMLButtonElement>('[role="tab"]')[0]!;
      expect(firstTab).toHaveTextContent("B 제외");
      await act(async () => firstTab.click());

      const artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
      expect(artifacts[0]).toHaveClass("is-kept");
      expect(artifacts[0]).toHaveTextContent("신규 B 최종 제외로 유지");
      expect(artifacts[1]).toHaveClass("is-excluded");
      expect(artifacts[1]).toHaveTextContent("다른 후보 판정으로 신규 B 최종 제외");
      expect(container.textContent).not.toContain("둘 다 보존 선택");
      expect(container.textContent).not.toContain("오탐 판정 · 둘 다 보존");

      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    },
  );

  it("keeps durable candidate results visible while a multi-candidate review continues", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    review.candidates = review.candidates.slice(0, 2).map((candidate, index) => index === 0
      ? { ...candidate, decision: "existing_removed" as const }
      : candidate);
    review.decisions = [{
      candidateId: review.candidates[0]!.candidateId,
      action: "remove_existing_continue",
      actor: "human",
      createdAt: "2026-08-25T00:01:00.000Z",
    }];

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

    const decidedTab = container.querySelectorAll<HTMLButtonElement>('[role="tab"]')[0]!;
    expect(decidedTab).toHaveTextContent("A 제외 · 계속 검토");
    expect(decidedTab.querySelector("small")).toHaveClass("is-pending");
    await act(async () => decidedTab.click());

    expect(container.querySelector(".download-overlap-result")).toBeNull();
    const artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
    expect(artifacts[0]).toHaveClass("is-excluded");
    expect(artifacts[0]).toHaveTextContent("수동 제거 선택");
    expect(artifacts[1]).toHaveClass("is-pending");
    expect(artifacts[1]).toHaveTextContent("검토 계속 중");
    expect(artifacts[1]).toHaveTextContent("남은 후보 검토 중");

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
    const actionButtons = ["검토 미루기", "기존 A 제거", "신규 B 제거", "오탐 판정", "둘 다 보존"]
      .map((label) => [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === label));
    expect(actionButtons.every((button) => Boolean(button?.getAttribute("aria-describedby")))).toBe(true);
    await click("기존 A 제거");
    await click("신규 B 제거");
    await click("오탐 판정");
    await click("둘 다 보존");
    await click("검토 미루기");
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(onDecision).toHaveBeenCalledWith({
      reviewId: "review-overlap",
      expectedRevision: 4,
      action: "remove_existing_continue",
      candidateId: "candidate-1",
    });
    expect(onDecision).toHaveBeenCalledWith({
      reviewId: "review-overlap",
      expectedRevision: 4,
      action: "remove_incoming",
    });
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
    expect(container.querySelector(".download-overlap-action-note")).toHaveTextContent("기존 제외를 복구하지 않고");

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
