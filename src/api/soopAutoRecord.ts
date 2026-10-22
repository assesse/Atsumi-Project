import { invoke } from "@tauri-apps/api/core";

export type SoopAutoStatus = "idle" | "starting" | "recording" | "stopping" | "waiting" | "skipped" | "failed";
export type SoopAutoChannel = {
  channelId: string; name: string; enabled: boolean;
  liveStatus: "live" | "offline" | "unknown";
  status: SoopAutoStatus; jobId: string | null; checkedAt: number | null; error: string | null;
};
export type SoopAutoSnapshot = { revision: number; channels: SoopAutoChannel[]; error: string | null; ready: boolean };
export interface SoopAutoRecordApi {
  snapshot(): Promise<SoopAutoSnapshot>;
  set(channelId: string, name: string, enabled: boolean, expectedRevision: number): Promise<SoopAutoSnapshot>;
}
export const soopAutoRecordApi: SoopAutoRecordApi = {
  snapshot: () => invoke("soop_auto_record_snapshot"),
  set: (channelId, name, enabled, expectedRevision) => invoke("soop_auto_record_set", { channelId, name, enabled, expectedRevision }),
};
