import { createContext, useContext, useEffect, useMemo, type ReactNode } from "react";
import type { BackendClient } from "../api/backend";
import type { DetailOriginalPrepared } from "../api/contracts";
import type { Gallery } from "../core/types";

type Entry = { id: string; page: number; promise: Promise<DetailOriginalPrepared>; resolve(media: DetailOriginalPrepared): void; reject(error: Error): void; started: boolean; expired?: boolean; image?: HTMLImageElement };
/** A preview-window lease, not an application-wide original-image cache.
 * At most two reads and the visible spread +/- 3 pages survive. */
export class OriginalPagePool {
  private entries = new Map<number, Entry>();
  private running = 0;
  constructor(private backend: BackendClient, private gallery: Gallery) {}
  setRange(pages: number[], count: number) {
    const wanted = new Set(pages);
    for (let distance = 1; distance <= 3; distance++) for (const page of pages) {
      if (page + distance <= count) wanted.add(page + distance);
      if (page - distance >= 1) wanted.add(page - distance);
    }
    for (const [page, entry] of this.entries) if (!wanted.has(page)) this.remove(page, entry);
    for (const page of wanted) if (!this.entries.has(page)) {
      let resolve!: Entry["resolve"], reject!: Entry["reject"];
      const promise = new Promise<DetailOriginalPrepared>((yes, no) => { resolve = yes; reject = no; });
      void promise.catch(() => {}); // speculative pages have no visible consumer yet
      this.entries.set(page, { id: crypto.randomUUID(), page, promise, resolve, reject, started: false });
    }
    // Current pages precede speculative neighbours even after a long jump.
    this.entries = new Map([...wanted].map(page => [page, this.entries.get(page)!]));
    this.pump();
  }
  get(page: number) { return this.entries.get(page)?.promise; }
  private remove(page: number, entry: Entry) {
    this.entries.delete(page);
    entry.expired = true;
    entry.reject(new Error("preview closed"));
    if (entry.image) entry.image.src = "";
    void this.backend.detailOriginalDispose(entry.id).catch(() => {});
  }
  dispose() { for (const [page, entry] of this.entries) this.remove(page, entry); }
  private pump() {
    for (const entry of this.entries.values()) {
      if (this.running >= 2) break;
      if (entry.started) continue;
      entry.started = true; this.running++;
      const entryId = this.gallery.download?.state === "completed" ? this.gallery.download.entryId : undefined;
      const timeout = setTimeout(() => { if (this.entries.get(entry.page) === entry) { entry.expired = true; entry.reject(new Error("original timeout")); void this.backend.detailOriginalDispose(entry.id).catch(() => {}); } }, 60_000);
      void this.backend.detailOriginalPrepare({ requestId: entry.id, galleryId: this.gallery.id, sourcePage: entry.page, entryId }).then(result => {
        if (entry.expired || this.entries.get(entry.page) !== entry) { void this.backend.detailOriginalDispose(entry.id).catch(() => {}); return; }
        if (!result.ok) throw new Error(result.error.code);
        const media = result.data;
        if (media.requestId !== entry.id || media.galleryId !== this.gallery.id || media.sourcePage !== entry.page) throw new Error("invalid original response");
        // Browser cache is warm before the next turn; very large images are not
        // speculatively decoded. Closing the preview releases every lease.
        if (media.width * media.height <= 4_000_000) { entry.image = new Image(); entry.image.src = media.mediaUrl; }
        entry.resolve(media);
      }).catch(error => { entry.reject(error); void this.backend.detailOriginalDispose(entry.id).catch(() => {}); })
        .finally(() => { clearTimeout(timeout); this.running--; this.pump(); });
    }
  }
}
const Context = createContext<OriginalPagePool | null>(null);
export const useOriginalPagePool = () => useContext(Context);
export function OriginalPageWindow({ gallery, pages, count, backend, children }: { gallery: Gallery; pages: number[]; count: number; backend: BackendClient; children: ReactNode }) {
  const pool = useMemo(() => new OriginalPagePool(backend, gallery), [backend, gallery.id, gallery.download?.state, gallery.download?.entryId, gallery.download?.revision]);
  // Child effects run before parent effects: their subscription is deferred one
  // microtask; establish this bounded range in the parent effect, not render.
  const key = pages.join(":");
  useEffect(() => { pool.setRange(pages, count); }, [pool, key, count]);
  useEffect(() => () => pool.dispose(), [pool]);
  return <Context.Provider value={pool}>{children}</Context.Provider>;
}
