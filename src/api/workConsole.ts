import { invoke } from "@tauri-apps/api/core";
import { backend } from "./backend";
import type { DownloadState, GalleryId } from "../core/types";
import { runningDownloadStates } from "../state/downloadCancellation";
import { readNavigationCheckpoint, writeNavigationCheckpoint, type NavigationCheckpoint } from "../state/navigationCheckpoint";

export type QueueQuery = { sequence?: number; after?: boolean; page?: number; offset?: number; includeSettled?: boolean; cancellationPreview?: boolean; observedSince?: string };
export type QueueRow = { entryId: string; galleryId: GalleryId; title: string; artist: string; state: DownloadState; progress: number; sequence: number | null; updatedAt: string; errorCode: string | null };
export type QueueBatch = { sequence: number; requestId: string; requested: number; owned: number; active: number; createdAt: string | null; label: string };
export type QueueSnapshot = {
  queriedAt: string; counts: Partial<Record<DownloadState, number>>; globalActive: number;
  totalRows: number; page: number; pageSize: number; offset?: number; items: QueueRow[]; batches: QueueBatch[];
  etaSeconds: number | null; recentCompleted: number; lastProgressAt: string | null;
};

export async function getQueueSnapshot(query: QueueQuery): Promise<QueueSnapshot> {
  if (backend.runtime === "tauri") return invoke("work_queue_snapshot", { query });
  const response = await backend.downloadEntriesList({ page: 1, pageSize: 200 });
  if (!response.ok) throw new Error(response.error.message);
  const counts: QueueSnapshot["counts"] = {};
  const rows = response.data.entries.map((entry): QueueRow => {
    if (!query.observedSince || runningDownloadStates.has(entry.state) || (entry.updatedAt ?? "") >= query.observedSince) counts[entry.state] = (counts[entry.state] ?? 0) + 1;
    return { ...entry, title: `앨범 #${entry.galleryId}`, artist: "", sequence: 1, errorCode: entry.errorCode ?? null, progress: entry.progress ?? 0, updatedAt: entry.updatedAt ?? "" };
  });
  const active = rows.filter((row) => runningDownloadStates.has(row.state));
  const filtered = query.after ? [] : query.includeSettled && !query.cancellationPreview ? rows : active;
  const page = query.page ?? 1;
  const offset = query.offset ?? (page - 1) * 100;
  return { queriedAt: new Date().toISOString(), counts: query.after ? {} : counts, globalActive: active.length, totalRows: filtered.length, page, pageSize: 100,
    items: query.cancellationPreview ? filtered : filtered.slice(offset, offset + 100), offset, batches: [{ sequence: 1, requestId: "browser-preview", requested: rows.length, owned: rows.length, active: active.length, label: "미리보기 큐", createdAt: null }], etaSeconds: null, recentCompleted: 0, lastProgressAt: null };
}

/** Freeze explicit IDs for the confirmation dialog. Jobs added after preview are not cancelled. */
export async function previewQueueCancellation(query: QueueQuery): Promise<QueueRow[]> {
  const snapshot = await getQueueSnapshot({ ...query, page: 1, includeSettled: false, cancellationPreview: true });
  if (snapshot.totalRows > 10_000 || snapshot.items.length !== snapshot.totalRows) throw new Error("한 번에 10,000개까지 미리 볼 수 있습니다. 요청 묶음을 좁혀 주세요.");
  return snapshot.items;
}

let checkpointPending = false;
let nextCheckpoint: NavigationCheckpoint | null = null;
export function persistNativeCheckpoint(checkpoint: NavigationCheckpoint): void {
  if (backend.runtime !== "tauri") return;
  nextCheckpoint = checkpoint;
  if (checkpointPending) return;
  checkpointPending = true;
  void (async () => {
    try {
      while (nextCheckpoint) {
        const current = nextCheckpoint; nextCheckpoint = null;
        await invoke("work_checkpoint_save", { checkpoint: current });
      }
    } catch { /* Keep session checkpoint; diagnostic channel reports the last durable timestamp. */ }
    finally { checkpointPending = false; }
  })();
}

/** Restore before React mounts; a broken bridge cannot prevent the shell from opening. */
export async function restoreNativeCheckpoint(): Promise<void> {
  if (backend.runtime !== "tauri") return;
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    const checkpoint = await Promise.race([
      invoke<NavigationCheckpoint | null>("work_checkpoint_get"),
      new Promise<null>((resolve) => { timeout = setTimeout(() => resolve(null), 1500); }),
    ]);
    const current = readNavigationCheckpoint();
    if (checkpoint && (!current || checkpoint.savedAt > current.savedAt)) writeNavigationCheckpoint(checkpoint);
  } catch { /* Session-only restore remains available. */ }
  finally { clearTimeout(timeout); }
}
