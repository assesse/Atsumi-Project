import type { DownloadState, Gallery } from "../core/types";

export function linkedGalleryId(download: Gallery["download"]): number | undefined {
  const id = download?.linkedGalleryId;
  return download?.state === "cancelled" && download.errorCode === "SOURCE_GALLERY_LINKED"
    && id !== undefined && Number.isSafeInteger(id) && id > 0 ? id : undefined;
}

/** Shared by Explore, related albums, detail tabs, and the durable queue. */
export const downloadStatus: Record<DownloadState, { label: string; tone: string; symbol: string }> = {
  queued: { label: "대기 중", tone: "waiting", symbol: "◷" },
  resolving_metadata: { label: "정보 확인", tone: "active", symbol: "↻" },
  downloading: { label: "다운로드 중", tone: "active", symbol: "↓" },
  hashing: { label: "해시 중", tone: "processing", symbol: "#" },
  verifying: { label: "검증 중", tone: "processing", symbol: "↻" },
  retry_wait: { label: "재시도 대기", tone: "waiting", symbol: "◷" },
  review_required: { label: "검토 필요", tone: "review", symbol: "!" },
  interrupted: { label: "중단됨", tone: "error", symbol: "Ⅱ" },
  failed: { label: "실패", tone: "error", symbol: "×" },
  completed: { label: "완료", tone: "complete", symbol: "✓" },
  quarantined: { label: "격리됨", tone: "muted", symbol: "▣" },
  cancelled: { label: "취소됨", tone: "muted", symbol: "−" },
};

export type BackgroundOpenOptions = { background?: boolean };

export type GalleryProcessingOptions = { duplicateCount?: number; exclusionLabel?: string };
export type GalleryProcessingState = DownloadState | "duplicate" | "linked" | "excluded";

/** Local decisions take precedence over an older completed download record. */
export function galleryProcessingStatus(download: Gallery["download"], options: GalleryProcessingOptions = {}):
  { state: GalleryProcessingState; label: string; tone: string } | undefined {
  if (download?.state === "quarantined") return {
    state: "quarantined", tone: "muted",
    label: options.exclusionLabel ? "격리됨 · " + options.exclusionLabel : "격리됨",
  };
  if (options.exclusionLabel) return { state: "excluded", tone: "muted", label: options.exclusionLabel };
  if (download?.state === "review_required" && download.reviewKind === "gallery_duplicate") {
    return { state: "duplicate", tone: "review", label: "다운로드 판본 중복" };
  }
  if ((options.duplicateCount ?? 0) > 0) return {
    state: "duplicate", tone: "review", label: "중복 후보 " + options.duplicateCount + "개",
  };
  const target = linkedGalleryId(download);
  if (target) return { state: "linked", tone: "muted", label: "새 번호 #" + target + "에 연결됨" };
  return download ? { state: download.state, ...downloadStatus[download.state] } : undefined;
}
