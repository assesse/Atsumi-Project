import { invoke } from '@tauri-apps/api/core';

export type YouTubeAutoChannel = {
  channelId: string; name: string; enabled: boolean;
  liveStatus: 'live' | 'offline' | 'unknown';
  status: 'idle' | 'starting' | 'recording' | 'stopping' | 'waiting' | 'skipped' | 'failed';
  jobId: string | null; checkedAt: number | null; error: string | null; recordingCount: number;
};
export type YouTubeAutoSnapshot = { revision: number; channels: YouTubeAutoChannel[]; ready: boolean; error: string | null };
export interface YouTubeAutoRecordApi {
  snapshot(): Promise<YouTubeAutoSnapshot>;
  resolve(input: string): Promise<{ channelId: string; name: string }>;
  set(channelId: string, name: string, enabled: boolean, expectedRevision: number): Promise<YouTubeAutoSnapshot>;
}
export const youtubeAutoRecordApi: YouTubeAutoRecordApi = {
  snapshot: () => invoke('youtube_auto_record_snapshot'),
  resolve: input => invoke('youtube_auto_record_resolve', { input }),
  set: (channelId, name, enabled, expectedRevision) => invoke('youtube_auto_record_set', { channelId, name, enabled, expectedRevision }),
};
