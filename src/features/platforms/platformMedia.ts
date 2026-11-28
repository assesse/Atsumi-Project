import { invoke } from '@tauri-apps/api/core';
import type { MediaCaptureOutcome } from './mediaCaptureOutcome';

export type MediaProvider = 'youtube' | 'soop';
export interface MediaRef { provider: MediaProvider; kind: 'video' | 'live'; externalId: string }
export type MediaJobState = 'queued' | 'resolving' | 'downloading' | 'recording' | 'reconnecting' | 'finalizing' | 'verifying' | 'completed' | 'interrupted' | 'failed' | 'cancelled' | 'deleted';
export interface MediaChatArchive { state: 'complete' | 'partial' | 'unknown'; reasons: ('archive_limit' | 'write_failed' | 'flush_failed' | 'interrupted' | 'status_unknown')[]; originalsRetained: boolean; assetsIncomplete: boolean }
export interface MediaLibraryItem {
  broadcastId?: string | null;
  libraryId: string; revision: number; reference: MediaRef; title: string; channelName: string;
  bytes: number; duration: number; sha256: string; completedAt: number;
  chatArchive?: MediaChatArchive | null;
  captureOutcome?: MediaCaptureOutcome | null;
}
export interface MediaJob {
  broadcastId?: string | null;
  jobId: string; requestId: string; reference: MediaRef; revision: number; state: MediaJobState;
  title: string; channelName: string; createdAt: number; receivedBytes: number; totalBytes?: number | null;
  error?: string | null; library?: MediaLibraryItem | null;
}
export interface MediaPlayback { token: string; url: string; item: MediaLibraryItem }
export interface MediaLibraryPage { items: MediaLibraryItem[]; cursor?: string | null }
export interface MediaRecordingGroup {
  type: 'recordingGroup'; key: string; groupRevision: string; representative: MediaLibraryItem;
  fileCount: number; totalBytes: number; latestCompletedAt: number;
}
export type MediaLibraryEntry = { type: 'video'; key: string; item: MediaLibraryItem } | MediaRecordingGroup;
export type MediaGroupsPage = { status: 'ready'; catalogRevision: string; entries: MediaLibraryEntry[]; cursor: string | null }
  | { status: 'changed'; catalogRevision: string };
export type MediaGroupMembersPage = { status: 'ready'; group: MediaRecordingGroup; items: MediaLibraryItem[]; cursor: string | null }
  | { status: 'changed'; groupRevision: string } | { status: 'missing' };
export type MediaChatPart = { type: 'text'; text: string } | { type: 'emoji'; assetId: string; alt: string };
export interface MediaChatAsset { state: 'pending' | 'ready' | 'missing'; dataUrl: string | null }
export interface SoopDisplayValues {
  theme: 'light' | 'dark' | null; fontLevel: number | null;
  randomNicknameColor: boolean | null; fanMessageColor: boolean | null;
  showEmoticons: boolean | null; animateEmoticons: boolean | null; smallOgq: boolean | null;
}
export type SoopPaletteRole = 'normal' | 'streamer' | 'manager' | 'vip' | 'subscribe' | 'fan' | 'support' | 'nightbot';
export interface SoopDisplayPalette {
  version: 1; basis: 'computed-css-variable';
  nickname: (string | null)[];
  roles: Record<SoopPaletteRole, string | null>;
}
export interface SoopDisplayState {
  version: 1; values: SoopDisplayValues;
  basis: Record<keyof SoopDisplayValues, 'dom' | 'saved-option' | null>;
  palette?: SoopDisplayPalette | null;
  paletteIncomplete?: boolean;
  surface?: SoopDisplaySurface | null;
  surfaceIncomplete?: boolean;
  documentEpoch: number; sequence: number; observedAtMs: number | null; timeMs: number | null;
  timingQuality: 'receive-time-approximate' | 'unmapped';
}
export interface SoopDisplaySurface {
  version: 1; basis: 'computed-dom'; foreground: string | null; background: string | null;
}
export type SoopRowDisplay = Pick<SoopDisplayValues, 'fanMessageColor' | 'showEmoticons' | 'smallOgq'>;
export interface SoopChatMetadata {
  flag1: number; flag2: number;
  role: 'streamer' | 'manager' | 'vip' | 'subscribe' | 'fan' | 'support' | 'nightbot' | 'normal';
  badge: 'streamer' | 'manager' | 'vip' | 'fan' | 'support' | 'nightbot' | 'normal';
  subscriptionTier: 1 | 2 | 3 | null;
  subscriptionMonths: number | null; accumulatedSubscriptionMonths: number | null;
  messageColor?: string | null;
  nicknameColorKey?: string | null;
  messageType?: 'staff' | 'police' | null;
  personalcon?: SoopPersonalconMetadata | null;
  avatar?: SoopAvatarMetadata | null;
}
export interface SoopAvatarMetadata { imageAssetId: string; kind: 'broadcaster'; source: 'connectionMetadata' }
export interface SoopPersonalconMetadata {
  imageAssetId: string; tierNickname?: string | null;
  tier: 1 | 2 | 3; months: number; source: 'connectionMetadata';
}
export interface SoopEmoticonMetadata {
  kind: 'ogq' | 'gem'; animated: boolean;
  staticImageAssetId?: string | null; smallStaticImageAssetId?: string | null;
  animatedImageAssetId?: string | null; smallAnimatedImageAssetId?: string | null;
}
export type SoopMissionNotice = { result:'draw' } | { result:'winner'; winner:string }
  | { result:'rank'; rank:number; team:string } | { result:'challenge'; title:string; successful:boolean };
export interface SoopMissionMetadata {
  kind:'battle'|'challenge'; event:'gift'|'settle'|'notice'; count?:number|null; relay?:boolean|null;
  fanOrder?:number|null; topFan?:boolean|null; notice?:SoopMissionNotice|null; imageAssetId?:string|null;
}
export interface SoopGiftMetadata {
  kind: 'balloon' | 'videoBalloon';
  count: number; relay: boolean; isDefault: boolean;
  fanOrder?: number | null; topFan?: boolean | null;
  imageAssetId?: string | null;
}
export interface SoopSubscriptionMetadata {
  event: 'new' | 'renewal'; displayTier: 1 | 2;
  tierNickname?: string | null;
  fromVod?: boolean | null; productMonths?: number | null;
  months?: number | null;
  imageAssetId?: string | null; fallbackAssetId?: string | null;
}
export interface MediaReplayChatItem { id: string; timeMs: number | null; kind: string; author: string; text: string; parts?: MediaChatPart[]; amount: string | null; badges: string[]; color: string | null; soop?: SoopChatMetadata; soopGift?: SoopGiftMetadata; soopMission?: SoopMissionMetadata; soopEmoticon?: SoopEmoticonMetadata; soopSubscription?: SoopSubscriptionMetadata; soopDisplay?: SoopRowDisplay }
export interface MediaReplayChatPage { items: MediaReplayChatItem[]; state: 'missing' | 'empty' | 'unmapped' | 'ready' | 'indexing'; preparation?: { scannedBytes: number; totalBytes: number }; timelineQuality: string; nextAtMs: number | null; cursor: string | null; previousCursor: string | null; total: number; malformedLines: number; unsupportedItems: number; reordered: number; duplicates: number; partialFile: boolean; displayMetadataUnavailable?: boolean; soopDisplay?: SoopDisplayState; soopDisplayIncomplete?: boolean }
export interface MediaLiveSnapshot { token: string; url: string; reference: MediaRef; state: 'queued' | 'resolving' | 'playing' | 'ended' | 'failed'; title: string; channelName: string; receivedBytes: number; recordingJob: string | null; recordingStopping: boolean; chatStatus: string; error: string | null }
export interface PlatformLiveApi {
  open(reference: MediaRef, requestId: string): Promise<MediaLiveSnapshot>;
  snapshot(token: string): Promise<MediaLiveSnapshot>;
  record(token: string, enabled: boolean): Promise<MediaLiveSnapshot>;
  close(token: string): Promise<void>;
  liveProfile?(token: string): Promise<string | null>;
  chat?(token: string, after: number, limit: number): Promise<MediaLiveChatPage>;
  chatAsset?(token: string, assetId: string): Promise<MediaChatAsset>;
}
export interface MediaLiveChatRow { sequence: number; kind: string; author: string; text: string; parts?: MediaChatPart[]; amount: string | null; badges: string[]; color?: string | null }
export interface MediaLiveChatPage { items: MediaLiveChatRow[]; cursor: number; status: string; ended: boolean; gap: boolean }
export const platformLiveApi: PlatformLiveApi = {
  open: (reference, requestId) => invoke('platform_media_live_open', { reference, requestId }),
  snapshot: token => invoke('platform_media_live_snapshot', { token }),
  record: (token, enabled) => invoke('platform_media_live_record', { token, enabled }),
  close: token => invoke('platform_media_live_close', { token }),
  liveProfile: token => invoke('platform_media_live_profile', { token }),
  chat: (token, after, limit) => invoke('platform_media_live_chat', { token, after, limit }),
  chatAsset: (token, assetId) => invoke('platform_media_live_chat_asset', { token, assetId }),
};
export interface PlatformMediaApi {
  download(reference: MediaRef, requestId: string): Promise<MediaJob>;
  downloadBatch?(references: MediaRef[], requestId: string): Promise<MediaJob[]>;
  jobs(provider: MediaProvider): Promise<MediaJob[]>;
  cancel(jobId: string, revision: number): Promise<MediaJob>;
  retry(jobId: string, revision: number): Promise<MediaJob>;
  library(provider: MediaProvider, cursor?: string): Promise<MediaLibraryPage>;
  groups?(provider: MediaProvider, cursor?: string): Promise<MediaGroupsPage>;
  groupMembers?(provider: MediaProvider, groupKey: string, cursor?: string): Promise<MediaGroupMembersPage>;
  open(libraryId: string): Promise<MediaPlayback>;
  thumbnail?(libraryId: string): Promise<string | null>;
  profile?(libraryId: string): Promise<string | null>;
  currentProfile?(libraryId: string): Promise<string | null>;
  replayChat?(libraryId: string, positionMs: number, cursor?: string, limit?: number): Promise<MediaReplayChatPage>;
  replayChatAsset?(libraryId: string, assetId: string): Promise<MediaChatAsset>;
  close(token: string): Promise<void>;
  openFolder(libraryId: string): Promise<void>;
  delete(libraryId: string, revision: number): Promise<void>;
}
export const platformMediaApi: PlatformMediaApi = {
  download: (reference, requestId) => invoke('platform_media_download', { reference, requestId }),
  downloadBatch: (references, requestId) => invoke('platform_media_download_batch', { references, requestId }),
  jobs: provider => invoke('platform_media_jobs', { provider }),
  cancel: (jobId, revision) => invoke('platform_media_cancel', { jobId, revision }),
  retry: (jobId, revision) => invoke('platform_media_retry', { jobId, revision }),
  library: (provider, cursor) => invoke('platform_media_library', { provider, cursor: cursor ?? null, limit: 100 }),
  groups: (provider, cursor) => invoke('platform_media_library_groups', { provider, cursor: cursor ?? null, limit: 100 }),
  groupMembers: (provider, groupKey, cursor) => invoke('platform_media_library_group_members', { provider, groupKey, cursor: cursor ?? null, limit: 100 }),
  open: libraryId => invoke('platform_media_open', { libraryId }),
  thumbnail: libraryId => invoke('platform_media_thumbnail', { libraryId }),
  profile: libraryId => invoke('platform_media_profile', { libraryId }),
  currentProfile: libraryId => invoke('platform_media_profile', { libraryId, current: true }),
  replayChat: (libraryId, positionMs, cursor, limit = 100) => invoke('platform_media_replay_chat', { libraryId, positionMs, cursor: cursor ?? null, limit }),
  replayChatAsset: (libraryId, assetId) => invoke('platform_media_replay_chat_asset', { libraryId, assetId }),
  close: token => invoke('platform_media_close', { token }),
  openFolder: libraryId => invoke('platform_media_open_folder', { libraryId }),
  delete: (libraryId, revision) => invoke('platform_media_delete', { libraryId, revision }),
};
export const mediaStateLabel: Record<MediaJobState, string> = {
  queued: '대기', resolving: '준비 중', downloading: '다운로드 중', recording: '녹화 중', reconnecting: '재연결 중',
  finalizing: '보관 중', verifying: '검증 중', completed: '완료', interrupted: '중단됨', failed: '실패', cancelled: '취소됨', deleted: '삭제됨',
};
export function mediaJobActive(job: MediaJob): boolean {
  return ['queued', 'resolving', 'downloading', 'recording', 'reconnecting', 'finalizing', 'verifying'].includes(job.state);
}
