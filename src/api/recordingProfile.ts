import { invoke, isTauri } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";

export type RecordingProfile = { name: string | null; image: string | null };
const cache = new Map<string, { at: number; value: Promise<RecordingProfile> }>();
export function recordingProfile(id: string): Promise<RecordingProfile> {
  if (!isTauri()) return Promise.resolve({ name: null, image: null });
  const previous = cache.get(id);
  if (previous && Date.now() - previous.at < 30_000) return previous.value;
  const value = invoke<ApiResult<RecordingProfile>>("chzzk_recording_profile", { recordingId: id }).then(r => r.ok ? r.data : { name: null, image: null }).catch(() => ({ name: null, image: null }));
  if (cache.size >= 256) cache.delete(cache.keys().next().value!);
  cache.set(id, { at: Date.now(), value });
  return value;
}
export async function openRecordingChannel(id: string) {
  if (!isTauri()) return;
  const result = await invoke<ApiResult<void>>("chzzk_recording_open_channel", { recordingId: id });
  if (!result.ok) throw new Error(result.error.message);
}
