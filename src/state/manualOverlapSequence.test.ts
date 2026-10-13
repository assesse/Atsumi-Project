import { describe, expect, it, vi } from "vitest";
import { galleryId } from "../core/types";
import { collectManualOverlapReviews } from "./manualOverlapSequence";
import type { DownloadEntry } from "../api/contracts";

const entry = (i: number, state: DownloadEntry["state"] = "review_required"): DownloadEntry => ({
  entryId: `entry-${i}`, galleryId: galleryId(i + 1), reviewId: `review-${i}`,
  reviewKind: "gallery_duplicate", revision: 1, progress: 100, state,
});
describe("manual overlap queue", () => {
  it("collects every pending page, deduplicates, and excludes terminal jobs", async () => {
    const entries = Array.from({ length: 200 }, (_, i) => entry(i));
    const downloadEntriesList = vi.fn().mockResolvedValueOnce({ ok: true, data: { page: 1, totalItems: 204, entries } })
      .mockResolvedValueOnce({ ok: true, data: { page: 2, totalItems: 204, entries: [entry(0), entry(201, "quarantined"), entry(202, "completed"), entry(203)] } });
    const rows = await collectManualOverlapReviews({ downloadEntriesList }, () => true);
    expect(rows).toHaveLength(201);
    expect(rows?.at(-1)?.reviewId).toBe("review-203");
    expect(rows?.some(row => ["review-201", "review-202"].includes(row.reviewId))).toBe(false);
    expect(downloadEntriesList).toHaveBeenLastCalledWith({ state: "review_required", page: 2, pageSize: 200 });
  });
  it("ignores results after cancellation", async () => {
    const downloadEntriesList = vi.fn().mockResolvedValue({ ok: true, data: { page: 1, totalItems: 1, entries: [entry(0)] } });
    expect(await collectManualOverlapReviews({ downloadEntriesList }, () => false)).toBeNull();
  });
  it("does not loop over repeated or incomplete backend pages", async () => {
    const downloadEntriesList = vi.fn().mockImplementation(async ({ page }) => ({ ok: true, data: { page, totalItems: 999, entries: [entry(0)] } }));
    await expect(collectManualOverlapReviews({ downloadEntriesList }, () => true)).rejects.toThrow("반복");
    downloadEntriesList.mockResolvedValue({ ok: true, data: { page: 1, totalItems: 999, entries: [] } });
    await expect(collectManualOverlapReviews({ downloadEntriesList }, () => true)).rejects.toThrow("끝까지");
  });
});
