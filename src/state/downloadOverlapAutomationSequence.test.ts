import { describe, expect, it, vi } from "vitest";
import type {
  ApiResult,
  DownloadOverlapAutomationHistoryItem,
  DownloadOverlapAutomationHistoryPage,
} from "../api/contracts";
import { galleryId } from "../core/types";
import { collectUnacknowledgedAutomationHistory } from "./downloadOverlapAutomationSequence";

const historyItem = (
  index: number,
  overrides: Partial<DownloadOverlapAutomationHistoryItem> = {},
): DownloadOverlapAutomationHistoryItem => ({
  reviewId: `review-${index}`,
  incomingGalleryId: galleryId(index + 1),
  title: `Automatic review ${index}`,
  occurredAt: new Date(Date.UTC(2026, 8, 4, 0, index)).toISOString(),
  reviewState: "resolved",
  removeIncomingCount: 1,
  removeExistingCount: 0,
  removedGalleryIds: [galleryId(index + 1)],
  ...overrides,
});

const historyPage = (
  page: number,
  items: DownloadOverlapAutomationHistoryItem[],
  totalItems = items.length,
): ApiResult<DownloadOverlapAutomationHistoryPage> => ({
  ok: true,
  data: {
    page,
    pageSize: 200,
    totalItems,
    unacknowledgedItems: totalItems,
    items,
  },
});

describe("collectUnacknowledgedAutomationHistory", () => {
  it("collects every page at size 200 and returns all items newest first", async () => {
    const items = Array.from({ length: 405 }, (_, index) => historyItem(index));
    const loadPage = vi.fn(async ({ page }: { page: number; pageSize: number }) =>
      historyPage(page, items.slice((page - 1) * 200, page * 200), items.length));

    const result = await collectUnacknowledgedAutomationHistory(loadPage, () => true);

    expect(loadPage.mock.calls).toEqual([
      [{ page: 1, pageSize: 200 }],
      [{ page: 2, pageSize: 200 }],
      [{ page: 3, pageSize: 200 }],
    ]);
    expect(result).toEqual([...items].reverse());
    expect(items[0]?.reviewId).toBe("review-0");
  });

  it("uses the latest duplicate across pages before filtering acknowledgements", async () => {
    const items = Array.from({ length: 200 }, (_, index) => historyItem(index));
    items[2] = historyItem(2, { acknowledgedAt: "2026-09-04T09:00:00Z" });
    const updated = historyItem(1, { title: "Updated history title" });
    const loadPage = vi.fn()
      .mockResolvedValueOnce(historyPage(1, items, 202))
      .mockResolvedValueOnce(historyPage(2, [
        historyItem(0, { acknowledgedAt: "2026-09-04T09:00:00Z" }),
        updated,
      ], 202));

    const result = await collectUnacknowledgedAutomationHistory(loadPage, () => true);

    expect(result).toHaveLength(198);
    expect(result?.some((item) => item.reviewId === "review-0")).toBe(false);
    expect(result?.some((item) => item.reviewId === "review-2")).toBe(false);
    expect(result?.filter((item) => item.reviewId === "review-1")).toEqual([updated]);
  });

  it("preserves collection order when timestamps tie, including duplicate updates", async () => {
    const occurredAt = "2026-09-04T00:00:00Z";
    const first = historyItem(1, { occurredAt });
    const second = historyItem(2, { occurredAt });
    const updated = { ...first, title: "Updated first review" };
    const loadPage = vi.fn().mockResolvedValue(historyPage(1, [first, second, updated]));

    await expect(collectUnacknowledgedAutomationHistory(loadPage, () => true))
      .resolves.toEqual([updated, second]);
  });

  it("returns null after cancellation without requesting another page", async () => {
    let current = true;
    let finishPage!: (result: ApiResult<DownloadOverlapAutomationHistoryPage>) => void;
    const loadPage = vi.fn(() => new Promise<ApiResult<DownloadOverlapAutomationHistoryPage>>(
      (resolve) => { finishPage = resolve; },
    ));

    const collection = collectUnacknowledgedAutomationHistory(loadPage, () => current);
    current = false;
    finishPage(historyPage(1, [historyItem(1)], 201));

    await expect(collection).resolves.toBeNull();
    expect(loadPage).toHaveBeenCalledTimes(1);
  });

  it("does not start an already cancelled collection", async () => {
    const loadPage = vi.fn();

    await expect(collectUnacknowledgedAutomationHistory(loadPage, () => false)).resolves.toBeNull();
    expect(loadPage).not.toHaveBeenCalled();
  });

  it("rejects backend errors after an earlier page instead of returning partial history", async () => {
    const loadPage = vi.fn()
      .mockResolvedValueOnce(historyPage(1, [historyItem(1)], 201))
      .mockResolvedValueOnce({
        ok: false,
        error: { code: "database_unavailable", message: "History could not be loaded", retryable: true },
      });

    await expect(collectUnacknowledgedAutomationHistory(loadPage, () => true))
      .rejects.toThrow("History could not be loaded");
    expect(loadPage).toHaveBeenCalledTimes(2);
  });

  it("propagates a thrown loader error", async () => {
    const error = new Error("History connection failed");
    const loadPage = vi.fn().mockRejectedValue(error);

    await expect(collectUnacknowledgedAutomationHistory(loadPage, () => true)).rejects.toBe(error);
  });

  it("returns an empty collection when the backend advertises no history", async () => {
    const loadPage = vi.fn().mockResolvedValue(historyPage(1, []));

    await expect(collectUnacknowledgedAutomationHistory(loadPage, () => true)).resolves.toEqual([]);
    expect(loadPage).toHaveBeenCalledTimes(1);
  });

  it("rejects an empty page that still advertises more history", async () => {
    const loadPage = vi.fn()
      .mockResolvedValueOnce(historyPage(1, [historyItem(1)], 401))
      .mockResolvedValueOnce(historyPage(2, [], 401));

    await expect(collectUnacknowledgedAutomationHistory(loadPage, () => true))
      .rejects.toThrow("페이지 정보가 올바르지 않습니다");
    expect(loadPage).toHaveBeenCalledTimes(2);
  });

  it.each([
    { page: 0 },
    { pageSize: 0 },
    { pageSize: 100 },
    { totalItems: -1 },
    { totalItems: Number.NaN },
    { totalItems: Number.POSITIVE_INFINITY },
  ])("rejects invalid pagination metadata %j", async (overrides) => {
    const loadPage = vi.fn().mockResolvedValue({
      ok: true,
      data: { page: 1, pageSize: 200, totalItems: 1, unacknowledgedItems: 1, items: [historyItem(1)], ...overrides },
    });

    await expect(collectUnacknowledgedAutomationHistory(loadPage, () => true))
      .rejects.toThrow("페이지 정보가 올바르지 않습니다");
    expect(loadPage).toHaveBeenCalledTimes(1);
  });
});
