import type { CloudLiveOperation } from '../../api/cloudRecording';
export function canReplayLiveArchive(value: CloudLiveOperation) {
  const archive = value.archiveVerification;
  return value.receiptPersisted && !value.outcomeUncertain && value.managerJoined && value.workerJoined && value.sidecarJoined
    && archive?.state === 'metadata_verified' && archive.embeddable === true && !!archive.videoId
    && archive.videoId === value.broadcastId && Number.isFinite(archive.durationSeconds) && archive.durationSeconds! > 0;
}
