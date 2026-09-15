import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";

export type ReplayIndexState = "building" | "ready" | "failed";
export type ReplaySyncQuality = "receive_time_approximate" | "observed_media";
export type ReplaySession = {
  token: string; recordingId: string; title: string; recordedAt?: number; channelName?: string | null; channelProfileImage?: string | null; durationSeconds: number; mimeType: string;
  chatStatus: string; indexState: ReplayIndexState; syncQuality: ReplaySyncQuality;
  manualOffsetSeconds: number; warnings: string[];
};
export type ReplayMessage = {
  sequence: number; sender: string; text: string; serverTime: number | null; receivedAt: number;
  offsetSeconds: number; broadcastOffsetSeconds: number | null; mediaTimeSeconds: number;
  syncQuality: ReplaySyncQuality; senderKey?: string | null;
  assetIds?: Record<string, string>;
  rich?: { nicknameColor?: string | null; textColor?: string | null; profileUrl?: string | null; badges: { kind: string; title?: string | null; imageUrl: string }[]; emojis: { id: string; imageUrl: string }[] } | null;
};
export type ReplayPage = {
  generation: number; items: ReplayMessage[]; previousCursor?: string | null; nextCursor?: string | null;
  indexState: ReplayIndexState; syncQuality: ReplaySyncQuality; warnings: string[];
};
export type ReplayTimeline = {
  bucketSeconds: number; buckets: { startSeconds: number; chatCount: number; uniqueSenderCount: number | null; viewerCount: number | null; viewerSampleCount?: number; viewerCoverageSeconds?: number }[];
  viewerMetricStatus: "not_recorded" | "recorded" | "partial"; indexState: ReplayIndexState;
};
export interface ReplayApi {
  open(recordingId: string): Promise<ApiResult<ReplaySession>>;
  chatAt(token: string, mediaTime: number, generation: number): Promise<ApiResult<ReplayPage>>;
  chatPage(token: string, cursor: string | null, generation: number): Promise<ApiResult<ReplayPage>>;
  chatSearch?(token: string, query: string, field: string, cursor: string | null, generation: number): Promise<ApiResult<ReplayPage>>;
  timeline(token: string, bucketSeconds?: number): Promise<ApiResult<ReplayTimeline>>;
  openProfile?(token: string, sequence: number): Promise<ApiResult<void>>;
  setOffset(token: string, offsetSeconds: number): Promise<ApiResult<number>>;
  close(token: string): Promise<ApiResult<void>>;
  mediaUrl(token: string): string;
}
const unavailable = <T>(): ApiResult<T> => ({ ok: false, error: { code: "REPLAY_DESKTOP_REQUIRED", message: "저장 영상 다시보기는 데스크톱 앱에서 사용할 수 있습니다.", retryable: false } });
async function call<T>(command: string, args: Record<string, unknown>): Promise<ApiResult<T>> {
  try { return await invoke<ApiResult<T>>(command, args); }
  catch { return { ok: false, error: { code: "REPLAY_TRANSPORT_ERROR", message: "저장 영상을 읽지 못했습니다. 다시 열어 주세요.", retryable: true } }; }
}
export function createReplayApi(runtime: "tauri" | "browser-mock"): ReplayApi {
  const request = <T>(command: string, args: Record<string, unknown>): Promise<ApiResult<T>> => runtime === "tauri" ? call<T>(command, args) : Promise.resolve(unavailable<T>());
  return {
    open: (recordingId) => request("replay_open", { recordingId }),
    chatAt: (token, mediaTime, generation) => request("replay_chat_at", { token, mediaTime, generation, limit: 200 }),
    chatPage: (token, cursor, generation) => request("replay_chat_page", { token, cursor, generation, limit: 200 }),
    chatSearch: (token, query, field, cursor, generation) => request("replay_chat_search", { token, query, field, cursor, generation, limit: 200 }),
    timeline: (token, bucketSeconds) => request("replay_timeline", { token, ...(bucketSeconds === undefined ? {} : { bucketSeconds }) }),
    openProfile: (token, sequence) => request("replay_open_profile", { token, sequence }),
    setOffset: (token, offsetSeconds) => request("replay_set_offset", { token, offsetSeconds }),
    close: (token) => request("replay_close", { token }),
    mediaUrl: (token) => convertFileSrc(token, "atsumi-replay"),
  };
}
