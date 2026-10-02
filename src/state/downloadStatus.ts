import type { DownloadState } from "../core/types";

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
