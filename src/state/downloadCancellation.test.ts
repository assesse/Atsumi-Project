import { describe, expect, it, vi } from "vitest";
import type { ApiResult, DownloadEntry } from "../api/contracts";
import { galleryId } from "../core/types";
import { cancelDownloads, canCancelDownload, runningDownloadStates } from "./downloadCancellation";

const success = (ids: string[]): ApiResult<DownloadEntry[]> => ({
  ok: true, data: ids.map((entryId) => ({ entryId, galleryId: galleryId(1), revision: 2, state: "cancelled" })),
});

describe("download cancellation", () => {
  it("allows all running phases but never treats completed files as cancellable", () => {
    expect([...runningDownloadStates]).toEqual(["queued", "resolving_metadata", "downloading", "hashing", "verifying", "retry_wait"]);
    for (const state of runningDownloadStates) expect(canCancelDownload(state)).toBe(true);
    for (const state of ["review_required", "interrupted", "failed"] as const) expect(canCancelDownload(state)).toBe(true);
    for (const state of ["completed", "quarantined", "cancelled"] as const) expect(canCancelDownload(state)).toBe(false);
  });

  it("deduplicates and sends serial batches within the backend limit", async () => {
    const downloadCancel = vi.fn(async (ids: string[]) => success(ids));
    const onApplied = vi.fn();
    const ids = Array.from({ length: 405 }, (_, index) => `entry-${index}`);
    const result = await cancelDownloads({ downloadCancel }, [...ids, ids[0]!], onApplied);
    expect(downloadCancel.mock.calls.map(([batch]) => batch.length)).toEqual([200, 200, 5]);
    expect(result.cancelled.map((entry) => entry.entryId)).toEqual(ids);
    expect(result.skipped).toEqual([]);
    expect(result.failed).toEqual([]);
    expect(onApplied).toHaveBeenCalledTimes(3);
  });

  it.each(["completed", "quarantined"])("does not roll back other cancellations when an item becomes %s", async (state) => {
    const downloadCancel = vi.fn<(ids: string[]) => Promise<ApiResult<DownloadEntry[]>>>()
      .mockResolvedValueOnce({ ok: false, error: { code: "INVALID_DOWNLOAD_STATE", message: "changed", retryable: false,
        details: { entryId: "changed", state, operation: "cancel" } } })
      .mockImplementation(async (ids) => success(ids));
    const result = await cancelDownloads({ downloadCancel }, ["first", "changed", "last"]);
    expect(downloadCancel.mock.calls).toEqual([[["first", "changed", "last"]], [["first", "last"]]]);
    expect(result.cancelled.map((entry) => entry.entryId)).toEqual(["first", "last"]);
    expect(result.skipped).toEqual(["changed"]);
    expect(result.failed).toEqual([]);
  });

  it("skips a removed entry and stops once there are no remaining targets", async () => {
    const downloadCancel = vi.fn<(ids: string[]) => Promise<ApiResult<DownloadEntry[]>>>()
      .mockResolvedValue({ ok: false, error: { code: "DOWNLOAD_ENTRY_NOT_FOUND", message: "gone", retryable: false,
        details: { entryId: "gone" } } });
    expect(await cancelDownloads({ downloadCancel }, ["gone"])).toEqual({ cancelled: [], skipped: ["gone"], failed: [] });
    expect(downloadCancel).toHaveBeenCalledTimes(1);
  });

  it("does not loop or pretend success for an unrelated rejection", async () => {
    const downloadCancel = vi.fn<(ids: string[]) => Promise<ApiResult<DownloadEntry[]>>>()
      .mockResolvedValue({ ok: false, error: { code: "INVALID_DOWNLOAD_STATE", message: "unknown target", retryable: false,
        details: { entryId: "outside-selection", state: "completed" } } });
    const result = await cancelDownloads({ downloadCancel }, ["first"]);
    expect(result.failed).toEqual([{ entryIds: ["first"], message: "unknown target" }]);
    expect(result.cancelled).toEqual([]);
    expect(downloadCancel).toHaveBeenCalledTimes(1);
  });

  it("reports a transport failure without preventing a later batch from cancelling", async () => {
    const downloadCancel = vi.fn<(ids: string[]) => Promise<ApiResult<DownloadEntry[]>>>()
      .mockRejectedValueOnce(new Error("offline"))
      .mockImplementation(async (ids) => success(ids));
    const ids = Array.from({ length: 201 }, (_, index) => `entry-${index}`);
    const result = await cancelDownloads({ downloadCancel }, ids);
    expect(result.failed[0]?.entryIds).toHaveLength(200);
    expect(result.cancelled[0]?.entryId).toBe("entry-200");
  });

  it("does not call the backend for an empty selection", async () => {
    const downloadCancel = vi.fn();
    expect(await cancelDownloads({ downloadCancel }, [])).toEqual({ cancelled: [], skipped: [], failed: [] });
    expect(downloadCancel).not.toHaveBeenCalled();
  });
});
