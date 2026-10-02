import { invoke } from "@tauri-apps/api/core";
import type { DownloadChangedEvent, JobEvent } from "./contracts";
type Events = { "job:changed": JobEvent; "download:changed": DownloadChangedEvent; "download:resync": boolean };
type Batch = { items: { job: JobEvent; download: DownloadChangedEvent }[]; resync: boolean };
/** Pull/ack by completion: at most one small batch exists in the JS bridge.
 * The Rust side replaces older revisions while the renderer is unresponsive. */
export class DownloadEventPoller {
  private listeners = { "job:changed": new Set<(e: JobEvent) => void>(), "download:changed": new Set<(e: DownloadChangedEvent) => void>(), "download:resync": new Set<(e: boolean) => void>() };
  private timer?: ReturnType<typeof setTimeout>;
  private pending = false;
  private epoch = "";
  constructor(private call = invoke) {}
  subscribe<K extends keyof Events>(event: K, handler: (value: Events[K]) => void): () => void {
    const listeners = this.listeners[event] as Set<(value: Events[K]) => void>;
    listeners.add(handler); this.schedule();
    return () => { listeners.delete(handler); if (!this.active() && this.timer) { clearTimeout(this.timer); this.timer=undefined; } };
  }
  private active() { return Object.values(this.listeners).some((set) => set.size>0); }
  private schedule() {
    if (this.pending || this.timer || !this.active()) return;
    this.timer = setTimeout(() => { this.timer=undefined; void this.tick(); }, 250);
  }
  private async tick() {
    if (!this.active()) return;
    this.pending=true;
    try {
      this.epoch ||= await this.call<string>("ui_diagnostics_session");
      const batch=await this.call<Batch>("download_events_take", { epoch:this.epoch });
      for (const row of batch.items) {
        this.emit("job:changed",row.job); this.emit("download:changed",row.download);
      }
      if (batch.resync) this.emit("download:resync",true);
    } catch { this.epoch=""; }
    finally { this.pending=false; this.schedule(); }
  }
  private emit<K extends keyof Events>(event:K, value:Events[K]) {
    for (const handler of this.listeners[event] as Set<(value:Events[K]) => void>) {
      try { handler(value); } catch { /* A failed consumer cannot block others. */ }
    }
  }
}
