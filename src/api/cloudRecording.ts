import { invoke } from '@tauri-apps/api/core';
import type { ReplaySession } from './replay';
import type { MediaChatAsset, MediaReplayChatPage } from '../features/platforms/platformMedia';

export type CloudRecordingProvider = 'chzzk' | 'soop' | 'youtube';
export type CloudSourceRef = { provider: CloudRecordingProvider; recordingId: string; partIndex?: number | null; planId?: string | null; unitId?: string | null };
export type CloudError = { code: string; message: string; retryable: boolean };
export type CloudAccount = { configured: boolean; connected: boolean; authorizing: boolean; channelId?: string | null; channelName?: string | null; error?: CloudError | null };
export type CloudJobState = 'queued' | 'validating' | 'uploading' | 'paused' | 'waiting_auth' | 'retry_wait' | 'processing' | 'ready' | 'failed';
export type CloudVisibility = 'private' | 'unlisted';
export type CloudPreparationProgress = { phase: 'checking' | 'preparing' | 'verifying'; completedBytes?: number | null; totalBytes?: number | null; completedSeconds?: number | null; totalSeconds?: number | null };
export type CloudRecordingJob = {
  id: string; source: CloudSourceRef; title: string; state: CloudJobState; visibility?: CloudVisibility;
  uploadedBytes: number; totalBytes: number; videoId?: string | null;
  durationSeconds: number; mediaOffsetSeconds: number; error?: CloudError | null;
  retryAt?: number | null; preparation?: CloudPreparationProgress | null; recordingPlan?: boolean; replayCohortId?: string | null; createdAt: number; updatedAt: number;
};
export type CloudAutomaticTarget = { kind: 'channel'; provider: 'chzzk' | 'soop'; channelId: string }
  | { kind: 'recording'; provider: 'chzzk' | 'soop'; recordingId: string };
export type CloudAutomaticStatus = 'disabled' | 'waiting_account' | 'watching' | 'waiting_recording' | 'waiting_processing' | 'queued' | 'validating' | 'uploading' | 'processing' | 'ready' | 'paused' | 'failed' | 'source_missing';
export type CloudAutomaticRule = { id: string; target: CloudAutomaticTarget; enabled: boolean; visibility?: CloudVisibility; generation: number; activatedAtMs: number; ownerChannelId?: string | null; status: CloudAutomaticStatus; error?: CloudError | null };
export type CloudAutomaticSnapshot = { revision: number; rules: CloudAutomaticRule[]; error?: CloudError | null };
export type CloudSnapshot = { account: CloudAccount; jobs: CloudRecordingJob[]; automatic?: CloudAutomaticSnapshot };
export type CloudSourceCoverage = { state: 'complete' | 'incomplete'; expectedSegments?: number | null; coveredSegments?: number | null; missingFromSegment?: number | null; reason?: string | null; captureWarning?: string | null };
export type CloudSourcePart = { partIndex: number | null; durationSeconds: number; mediaOffsetSeconds: number; bytes: number; source?: CloudSourceRef; unitIndex?: number; kind?: 'direct' | 'derived' };
export type CloudSourceDescription = {
  source: CloudSourceRef; title: string; planId?: string | null; coverage?: CloudSourceCoverage;
  preparationRequired?: boolean; durationSeconds?: number; bytes?: number;
  parts: CloudSourcePart[];
};
export interface CloudRecordingApi {
  snapshot(): Promise<CloudSnapshot>;
  describe(source: CloudSourceRef): Promise<CloudSourceDescription>;
  getPlan(planId: string): Promise<CloudSourceDescription>;
  importClient(): Promise<CloudSnapshot | null>;
  authorize(): Promise<CloudSnapshot>;
  disconnect(): Promise<CloudSnapshot>;
  enqueue(source: CloudSourceRef, visibility?: CloudVisibility): Promise<CloudSnapshot>;
  enqueueRecording(source: CloudSourceRef, visibility?: CloudVisibility): Promise<CloudSnapshot>;
  pause(jobId: string): Promise<CloudSnapshot>;
  resume(jobId: string): Promise<CloudSnapshot>;
  setAutomatic(target: CloudAutomaticTarget, enabled: boolean, expectedRevision: number, visibility?: CloudVisibility): Promise<CloudSnapshot>;
}
export const cloudRecordingApi: CloudRecordingApi = {
  snapshot: () => invoke('cloud_recording_snapshot'),
  describe: source => invoke('cloud_recording_describe', { source }),
  getPlan: planId => invoke('cloud_recording_get_plan', { planId }),
  importClient: () => invoke('cloud_recording_import_oauth_client'),
  authorize: () => invoke('cloud_recording_authorize'),
  disconnect: () => invoke('cloud_recording_disconnect'),
  enqueue: (source, visibility) => invoke('cloud_recording_enqueue', { source, visibility }),
  enqueueRecording: (source, visibility) => invoke('cloud_recording_enqueue_recording', { source, visibility }),
  pause: jobId => invoke('cloud_recording_pause', { jobId }),
  resume: jobId => invoke('cloud_recording_resume', { jobId }),
  setAutomatic: (target, enabled, expectedRevision, visibility) => invoke('cloud_recording_automatic_set', { target, enabled, expectedRevision, visibility }),
};

/** Native receipt authorization supplies only saved chat; this token cannot open local video. */
export type CloudReplayDescriptor = {
  provider: CloudRecordingProvider; token: string; cohortId: string;
  description: CloudSourceDescription; session?: ReplaySession | null;
};
export interface CloudRecordingReplayApi {
  describe(jobId: string): Promise<CloudSourceDescription>;
  open(jobId: string): Promise<CloudReplayDescriptor>;
  close(provider: CloudRecordingProvider, token: string): Promise<void>;
  page(token: string, positionMs: number, cursor: string | null, limit: number): Promise<MediaReplayChatPage>;
  asset(token: string, assetId: string): Promise<MediaChatAsset>;
}
export const cloudRecordingReplayApi: CloudRecordingReplayApi = {
  describe: jobId => invoke('cloud_recording_replay_describe', { jobId }),
  open: jobId => invoke('cloud_recording_replay_open', { jobId }),
  close: (provider, token) => invoke('cloud_recording_replay_close', { provider, token }),
  page: (token, positionMs, cursor, limit) => invoke('cloud_recording_replay_page', { token, positionMs, cursor, limit }),
  asset: (token, assetId) => invoke('cloud_recording_replay_asset', { token, assetId }),
};

export function cloudErrorText(error: unknown): string {
  if (error && typeof error === 'object' && 'message' in error && typeof error.message === 'string') return error.message;
  // Native errors are bounded user-facing messages, never provider response bodies.
  return typeof error === 'string' ? error : 'YouTube 보관 상태를 확인하지 못했습니다.';
}

export type CloudLiveProvider = 'chzzk' | 'soop';
export type CloudLivePlan = { title: string; scheduledStartMs: number; madeForKids: boolean };
export type CloudLiveArchiveVerification = {
  state: 'checking' | 'pending' | 'needs_check' | 'metadata_verified' | 'inconclusive' | 'rejected';
  reason?: string | null; checkedAt?: number | null; deadlineAt: number; retryAt?: number | null; rounds: number;
  videoId?: string | null; durationSeconds?: number | null; embeddable?: boolean | null;
  processingInfoUnavailable: boolean; playbackVerified: false; remoteTimeOrigin: 'unknown'; synchronizedReplayReady: false;
};
export type CloudLiveOperation = {
  selectionId: string; revision: number; provider: CloudLiveProvider; channelId: string; broadcastKey?: string | null;
  state: string; preparationStage?: string | null; destinationChannelId?: string | null;
  streamId?: string | null; broadcastId?: string | null; outcomeUncertain: boolean; runId?: string | null;
  stopRequested: boolean; interrupted: boolean; segmentAcks: number; playlistAcks: number; endListAcked: boolean;
  managerJoined: boolean; workerJoined: boolean; sidecarJoined: boolean; archiveReady: boolean; receiptPersisted: boolean; error?: CloudError | null;
  sidecarTextState?: string | null; assetsIncomplete: boolean; timingIncomplete: boolean; clockQuality?: string | null;
  archiveVerification?: CloudLiveArchiveVerification | null;
};
export type CloudLiveSnapshot = { account: CloudAccount & { generation: number }; operations: CloudLiveOperation[] };
/** Native receipts contain public status only. Destination credentials and source
 * capabilities never cross this interface. Reading status starts no API request. */
export interface CloudLiveApi {
  snapshot(): Promise<CloudLiveSnapshot>;
  sources(provider: CloudLiveProvider): Promise<string[]>;
  importClient(): Promise<CloudLiveSnapshot>;
  authorize(): Promise<CloudLiveSnapshot>;
  disconnect(): Promise<CloudLiveSnapshot>;
  select(provider: CloudLiveProvider, channelId: string): Promise<CloudLiveSnapshot>;
  start(selectionId: string, expectedRevision: number, plan: CloudLivePlan): Promise<CloudLiveSnapshot>;
  stop(selectionId: string, expectedRevision: number): Promise<CloudLiveSnapshot>;
  verifyArchive(selectionId: string, expectedRevision: number): Promise<CloudLiveSnapshot>;
}
export const cloudLiveApi: CloudLiveApi = {
  snapshot: () => invoke('cloud_live_snapshot'),
  sources: provider => invoke('cloud_live_sources', { provider }),
  importClient: async () => { await cloudRecordingApi.importClient(); return invoke('cloud_live_snapshot'); },
  authorize: () => invoke('cloud_live_authorize'),
  disconnect: () => invoke('cloud_live_disconnect'),
  select: (provider, channelId) => invoke('cloud_live_select', { provider, channelId }),
  start: (selectionId, expectedRevision, plan) => invoke('cloud_live_start', { selectionId, expectedRevision, plan }),
  stop: (selectionId, expectedRevision) => invoke('cloud_live_stop', { selectionId, expectedRevision }),
  verifyArchive: (selectionId, expectedRevision) => invoke('cloud_live_verify_archive', { selectionId, expectedRevision }),
};
