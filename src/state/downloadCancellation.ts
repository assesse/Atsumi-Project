import type { BackendClient } from "../api/backend";
import type { DownloadEntry } from "../api/contracts";
import type { DownloadState } from "../core/types";

export const runningDownloadStates: ReadonlySet<DownloadState> = new Set([
  "queued", "resolving_metadata", "downloading", "hashing", "verifying", "retry_wait",
]);

export const canCancelDownload = (state: DownloadState): boolean =>
  runningDownloadStates.has(state) || ["review_required", "interrupted", "failed"].includes(state);

type CancelResult = {
  cancelled: DownloadEntry[];
  skipped: string[];
  failed: Array<{ entryIds: string[]; message: string }>;
};

/** Keep the backend's atomic contract; remove only explicitly rejected stale IDs and retry. */
export async function cancelDownloads(
  backend: Pick<BackendClient, "downloadCancel">,
  entryIds: readonly string[],
  onApplied: (entries: DownloadEntry[]) => void = () => {},
): Promise<CancelResult> {
  const result: CancelResult = { cancelled: [], skipped: [], failed: [] };
  const uniqueIds = [...new Set(entryIds)];
  // The command accepts at most 200 IDs. Serial batches also avoid flooding SQLite.
  for (let offset = 0; offset < uniqueIds.length; offset += 200) {
    let pending = uniqueIds.slice(offset, offset + 200);
    while (pending.length) {
      let response;
      try {
        response = await backend.downloadCancel(pending);
      } catch {
        result.failed.push({ entryIds: pending, message: "취소 요청을 전달하지 못했습니다." });
        break;
      }
      if (response.ok) {
        result.cancelled.push(...response.data.filter((entry) => entry.state === "cancelled"));
        result.skipped.push(...response.data.filter((entry) => entry.state !== "cancelled").map((entry) => entry.entryId));
        onApplied(response.data);
        break;
      }
      const rejected = response.error.details?.entryId;
      const stale = response.error.code === "DOWNLOAD_ENTRY_NOT_FOUND"
        || (response.error.code === "INVALID_DOWNLOAD_STATE"
          && ["completed", "quarantined"].includes(String(response.error.details?.state)));
      if (stale && typeof rejected === "string" && pending.includes(rejected)) {
        result.skipped.push(rejected);
        pending = pending.filter((id) => id !== rejected);
        continue;
      }
      result.failed.push({ entryIds: pending, message: response.error.message });
      break;
    }
  }
  return result;
}
