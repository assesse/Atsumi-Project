import type { AutoFindCutoffEvidence, AutoFindHistoryMode } from "../api/contracts";

export const autoFindHistoryModeLabel = (mode: AutoFindHistoryMode): string => {
  if (mode === "include_all_history") return "전체 기록 포함";
  if (mode === "newer_than_oldest_downloaded") return "가장 오래된 소유 작품 이후 (이전 기준)";
  return "가장 최근 소유 작품 이후";
};

export const autoFindCutoffDescription = (
  evidence: AutoFindCutoffEvidence,
  mode: AutoFindHistoryMode,
): string => {
  if (mode === "include_all_history") return "전체 범위 탐색";
  const legacy = mode === "newer_than_oldest_downloaded";
  const cutoff = legacy ? evidence.oldestOwnedGalleryId : evidence.latestOwnedGalleryId;
  if (cutoff === undefined) return "검증 완료·격리 소유 작품 없음 · 전체 범위 탐색";
  return legacy
    ? `이전 기준 · 검증된 소유본 ${evidence.qualifiedOwnedCount}개 중 가장 오래된 #${cutoff} 이후를 검색`
    : `검증된 소유본 ${evidence.qualifiedOwnedCount}개 중 가장 최근 작품 #${cutoff} 이후를 검색 (작품번호 기준)`;
};
