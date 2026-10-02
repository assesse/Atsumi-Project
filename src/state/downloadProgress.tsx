import { createContext, useCallback, useContext, useSyncExternalStore } from "react";
import type { DownloadChangedEvent } from "../api/contracts";
import type { Gallery, GalleryId } from "../core/types";
import { uiCounters } from "../diagnostics/uiDiagnostics";

type Download = Gallery["download"];
const STRUCTURAL_FIELDS = ["entryId", "state", "attempt", "errorCode", "errorMessage", "reviewKind", "reviewId"] as const;
const MAX_ENTRIES = 4096;

/** Progress is not gallery metadata. Notify only subscribers to the changed ID,
 * and coalesce visual progress for 100ms; structural transitions remain immediate. */
export class DownloadProgressStore {
  private values = new Map<number, DownloadChangedEvent>();
  private listeners = new Map<number, Set<() => void>>();
  private dirty = new Set<number>();
  private timer?: ReturnType<typeof setTimeout>;
  readonly counters = { received: 0, stale: 0, progressOnly: 0, notifications: 0 };

  read(id: number, fallback: Download): Download {
    const value = this.values.get(id);
    return value && value.entryId === fallback?.entryId && value.revision > (fallback.revision ?? -1)
      ? value : fallback;
  }

  apply(event: DownloadChangedEvent, fallback: Download): { applied: boolean; structural: boolean } {
    this.counters.received++;
    uiCounters.downloadReceived++;
    const previous = this.read(event.galleryId, fallback) ?? this.values.get(event.galleryId);
    if (previous?.entryId === event.entryId && (previous.revision ?? -1) >= event.revision) {
      this.counters.stale++; return { applied: false, structural: false };
    }
    const structural = !previous || STRUCTURAL_FIELDS.some((field) => previous[field] !== event[field]);
    this.values.delete(event.galleryId);
    this.values.set(event.galleryId, event);
    while (this.values.size > MAX_ENTRIES) {
      const oldest = [...this.values.keys()].find((id) => !this.listeners.has(id));
      if (oldest === undefined) break;
      this.values.delete(oldest); this.dirty.delete(oldest);
    }
    if (structural) { this.dirty.delete(event.galleryId); this.notify(event.galleryId); }
    else {
      this.counters.progressOnly++;
      this.dirty.add(event.galleryId);
      this.timer ??= setTimeout(() => this.flush(), 100);
    }
    return { applied: true, structural };
  }

  subscribe(id: number, listener: () => void): () => void {
    const listeners = this.listeners.get(id) ?? new Set();
    listeners.add(listener); this.listeners.set(id, listeners);
    return () => { listeners.delete(listener); if (!listeners.size) this.listeners.delete(id); };
  }
  flush(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    const changed = [...this.dirty]; this.dirty.clear();
    changed.forEach((id) => this.notify(id));
  }
  clear(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined; this.dirty.clear(); this.values.clear();
  }
  private notify(id: number): void {
    for (const listener of this.listeners.get(id) ?? []) { this.counters.notifications++; uiCounters.downloadNotifications++; listener(); }
  }
}

export const DownloadProgressContext = createContext<DownloadProgressStore | null>(null);
export function useGalleryDownload(id: GalleryId, fallback: Download): Download {
  const store = useContext(DownloadProgressContext);
  const subscribe = useCallback((listener: () => void) => store?.subscribe(id, listener) ?? (() => undefined), [store, id]);
  const snapshot = useCallback(() => store?.read(id, fallback) ?? fallback, [store, id, fallback]);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function DownloadProgressLabel({ gallery }: { gallery: Gallery }) {
  const download = useGalleryDownload(gallery.id, gallery.download);
  const raw = download?.state === "completed" ? 100 : download?.progress ?? 0;
  const progress = Number.isFinite(raw) ? Math.floor(Math.min(100, Math.max(0, raw))) : 0;
  return <b role="progressbar" aria-label={`${gallery.title} 진행률`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress}>{progress}%</b>;
}
