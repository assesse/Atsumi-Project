import { invoke } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";
import type { AutoRecordingEntry } from "./autoRecording";
export type LiveFavorite = { channelId: string; channelName: string };
export type LiveChannelProfile = { channelName: string; image: string | null };
export type LiveChannel = LiveFavorite & { favorite: boolean; scheduled?: AutoRecordingEntry };
export interface LiveChannelsApi {
  snapshot(): Promise<ApiResult<LiveFavorite[]>>;
  set(input: string, favorite: boolean): Promise<ApiResult<LiveFavorite[]>>;
  profile(channelId: string): Promise<ApiResult<LiveChannelProfile>>;
}
export function createLiveChannelsApi(runtime: "tauri" | "browser-mock"): LiveChannelsApi {
  async function call<T>(command: string, args?: Record<string, unknown>): Promise<ApiResult<T>> {
    if (runtime !== "tauri") return { ok: false, error: { code: "DESKTOP_REQUIRED", message: "즐겨찾기는 데스크톱 앱에서 사용할 수 있습니다.", retryable: false } };
    try { return await invoke(command, args); }
    catch { return { ok: false, error: { code: "LIVE_CHANNELS_TRANSPORT", message: "즐겨찾기에 연결하지 못했습니다.", retryable: true } }; }
  }
  return { snapshot: () => call("chzzk_live_favorites"), set: (input, favorite) => call("chzzk_live_favorite_set", { input, favorite }),
    profile: (channelId) => call("chzzk_live_channel_profile", { channelId }) };
}
export function mergeLiveChannels(favorites: LiveFavorite[], scheduled: AutoRecordingEntry[]): LiveChannel[] {
  const rows = new Map<string, LiveChannel>();
  for (const row of favorites) rows.set(row.channelId, { ...row, favorite: true });
  for (const row of scheduled) {
    const previous = rows.get(row.channelId);
    rows.set(row.channelId, { channelId: row.channelId, channelName: row.channelName || previous?.channelName || row.channelId, favorite: previous?.favorite ?? false, scheduled: row });
  }
  return [...rows.values()].sort((a, b) => Number(b.scheduled?.status === "recording") - Number(a.scheduled?.status === "recording")
    || Number(b.favorite) - Number(a.favorite) || a.channelName.localeCompare(b.channelName, "ko"));
}
