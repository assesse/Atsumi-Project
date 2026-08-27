import { useEffect, useRef } from "react";
import type { Gallery, GalleryId } from "../core/types";
import { FluentIcon } from "./FluentIcon";

type ActivityDrawerProps = {
  open: boolean;
  galleries: Gallery[];
  onClose: () => void;
  onReview: (id: GalleryId) => void;
  onRetry: (id: GalleryId) => void;
  onCancel: (id: GalleryId) => void;
  pendingEntryIds?: ReadonlySet<string>;
};

const stateDetail: Partial<Record<NonNullable<Gallery["download"]>["state"], string>> = {
  queued: "다운로드 대기",
  resolving_metadata: "원본 정보를 확인하는 중",
  downloading: "이미지를 받는 중",
  hashing: "다운로드 파일의 해시를 만드는 중",
  verifying: "파일 수와 무결성을 확인하는 중",
  retry_wait: "서버 cooldown 후 다시 시도",
  review_required: "유사 작품 검토 필요",
  interrupted: "이전 실행에서 작업이 중단됨",
  failed: "원본 사이트 응답 시간이 초과됨",
  completed: "다운로드와 파일 검증 완료",
  quarantined: "복구 가능한 격리 상태",
  cancelled: "사용자가 다운로드를 취소함",
};

const downloadDetail = (download: NonNullable<Gallery["download"]>): string => {
  if (download.errorMessage) return download.errorMessage;
  if (download.state === "failed") return "다운로드 작업이 실패했습니다.";
  if (download.state === "interrupted") return "다운로드 작업이 중단되었습니다.";
  return stateDetail[download.state] ?? "다운로드 상태를 확인하고 있습니다.";
};

const displayedProgress = (download: NonNullable<Gallery["download"]>): number => {
  const rawProgress = download.state === "completed" ? 100 : download.progress ?? 0;
  return Math.floor(Math.min(100, Math.max(0, Number.isFinite(rawProgress) ? rawProgress : 0)));
};

export function ActivityDrawer({
  open,
  galleries,
  onClose,
  onReview,
  onRetry,
  onCancel,
  pendingEntryIds = new Set(),
}: ActivityDrawerProps) {
  const closeButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) window.requestAnimationFrame(() => closeButton.current?.focus());
  }, [open]);

  if (!open) return null;
  const activities = galleries
    .filter((gallery) => gallery.download)
    .sort((left, right) => {
      const order = { failed: 0, review_required: 1, downloading: 2, queued: 3, completed: 4 } as const;
      return (order[left.download?.state as keyof typeof order] ?? 3) - (order[right.download?.state as keyof typeof order] ?? 3);
    });

  return (
    <aside
      id="activity-panel"
      className="activity-panel"
      aria-label="작업 상태"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          onClose();
        }
      }}
    >
      <div className="sr-only" role="status" aria-live="polite">
        {activities.map((gallery) => `${gallery.title}: ${downloadDetail(gallery.download!)}`).join(", ")}
      </div>
      <header>
        <div>
          <span className="eyebrow">ACTIVITY</span>
          <h2>작업 상태</h2>
        </div>
        <button ref={closeButton} type="button" className="icon-button small" title="닫기" aria-label="작업 상태 닫기" onClick={onClose}>
          <FluentIcon glyph="\uE711" />
        </button>
      </header>
      <div className="activity-list">
        {activities.map((gallery) => {
          const download = gallery.download!;
          const running = ["queued", "resolving_metadata", "downloading", "hashing", "verifying", "retry_wait"].includes(download.state);
          const complete = download.state === "completed";
          const warning = ["review_required", "failed", "interrupted"].includes(download.state);
          const retryable = ["failed", "interrupted", "cancelled"].includes(download.state);
          const cancellable = running || warning;
          const pending = pendingEntryIds.has(download.entryId);
          const progress = displayedProgress(download);
          return (
            <article
              key={download.entryId}
              className={`activity-item${warning ? " warning" : ""}${complete ? " complete" : ""}`}
            >
              <span className={`activity-icon${running ? " is-running" : ""}`}>
                {running ? <span className="spinner" /> : <FluentIcon glyph={complete ? "\uE73E" : "\uE7BA"} />}
              </span>
              <div>
                <strong>{gallery.title}</strong>
                <span>{downloadDetail(download)}</span>
                {download.attempt || download.errorCode ? (
                  <small>
                    {download.attempt ? `시도 ${download.attempt}` : ""}
                    {download.attempt && download.errorCode ? " · " : ""}
                    {download.errorCode ?? ""}
                  </small>
                ) : null}
              </div>
              <div className="activity-actions">
                {download.state === "review_required" ? (
                  <button type="button" className="mini-command" disabled={pending} onClick={() => onReview(gallery.id)}>검토</button>
                ) : null}
                {retryable ? (
                  <button type="button" className="mini-command" disabled={pending} onClick={() => onRetry(gallery.id)}>재시도</button>
                ) : null}
                {cancellable ? (
                  <button type="button" className="mini-command" disabled={pending} onClick={() => onCancel(gallery.id)}>취소</button>
                ) : null}
                {!warning && !retryable ? (
                  <b
                    role="progressbar"
                    aria-label={`${gallery.title} 진행률`}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={progress}
                  >
                    {progress}%
                  </b>
                ) : null}
              </div>
            </article>
          );
        })}
      </div>
    </aside>
  );
}
