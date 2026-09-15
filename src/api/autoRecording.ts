import { invoke } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";

export type AutoRecordingEntry = {
  channelId: string; channelName: string; enabled: boolean; checkedAt: number;
  status: string; recordingId: string | null; message: string | null;
  lastFailure?: { occurredAt: number; liveKey: string; stage: string; message: string } | null;
};
export type AutoRecordingSnapshot = { channels: AutoRecordingEntry[]; captureChat: boolean; error: string | null };
export type AutoRecordingChange = { action: "enabled"; enabled: boolean } | { action: "remove" | "stop" };
export interface AutoRecordingApi {
  snapshot(): Promise<ApiResult<AutoRecordingSnapshot>>;
  add(input: string): Promise<ApiResult<AutoRecordingSnapshot>>;
  update(channelId: string, change: AutoRecordingChange): Promise<ApiResult<AutoRecordingSnapshot>>;
}
export function createAutoRecordingApi(runtime: "tauri" | "browser-mock"): AutoRecordingApi {
  async function call(command: string, args?: Record<string, unknown>): Promise<ApiResult<AutoRecordingSnapshot>> {
    if (runtime !== "tauri") return { ok: false, error: { code: "DESKTOP_REQUIRED", message: "자동 녹화는 데스크톱 앱에서 사용할 수 있습니다.", retryable: false } };
    try { return await invoke(command, args); }
    catch { return { ok: false, error: { code: "AUTO_RECORD_TRANSPORT", message: "자동 녹화 상태를 확인하지 못했습니다.", retryable: true } }; }
  }
  return {
    snapshot: () => call("chzzk_auto_record_snapshot"),
    add: (input) => call("chzzk_auto_record_add", { input }),
    update: (channelId, change) => call("chzzk_auto_record_update", { channelId, change }),
  };
}
