import { useEffect, useRef } from "react";
import type { DownloadState, Gallery, GalleryId } from "../core/types";
import { FluentIcon } from "./FluentIcon";

type ActivityDrawerProps = {
  open: boolean;
  galleries: Gallery[];
  sessionDownloads: SessionDownloadActivity[];
  automaticOverlapActivities?: AutomaticOverlapActivity[];
  duplicateExcludedGalleryIds?: ReadonlySet<GalleryId>;
  onClose: () => void;
  onReview: (id: GalleryId) => void;
  onReviewOverlap?: (reviewId: string, galleryId: GalleryId) => void;
  onRetry: (id: GalleryId) => void;
  onCancel: (id: GalleryId) => void;
  pendingEntryIds?: ReadonlySet<string>;
};

export type SessionDownloadActivity = {
  galleryId: GalleryId;
  occurredAt: number;
  state?: DownloadState;
};

export type AutomaticOverlapActivity = {
  id: string;
  reviewId: string;
  galleryId: GalleryId;
  title: string;
  detail: string;
  occurredAt: number;
  state: "completed" | "failed";
};

const runningDownloadStates = new Set([
  "queued",
  "resolving_metadata",
  "downloading",
  "hashing",
  "verifying",
  "retry_wait",
]);

const duplicateProcessedDetail = "중복 검토 완료 · 탐색 및 다운로드 목록에서 제외됨";

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
  sessionDownloads,
  automaticOverlapActivities = [],
  duplicateExcludedGalleryIds = new Set(),
  onClose,
  onReview,
  onReviewOverlap,
  onRetry,
  onCancel,
  pendingEntryIds = new Set(),
}: ActivityDrawerProps) {
  const closeButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) window.requestAnimationFrame(() => closeButton.current?.focus());
  }, [open]);

  if (!open) return null;
  const galleryById = new Map(galleries.map((gallery) => [gallery.id, gallery]));
  const downloadActivities = sessionDownloads.flatMap(({ galleryId, occurredAt }) => {
    const gallery = galleryById.get(galleryId);
    return gallery?.download ? [{ kind: "download" as const, gallery, occurredAt }] : [];
  });
  const feed = [
    ...downloadActivities,
    ...automaticOverlapActivities.map((activity) => ({
      kind: "automatic-overlap" as const,
      activity,
      occurredAt: activity.occurredAt,
    })),
  ]
    .sort((left, right) => right.occurredAt - left.occurredAt);

  return (
    <aside
      id="activity-panel"
      className="activity-panel"
      aria-label="이번 실행 활동"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          onClose();
        }
      }}
    >
      <div className="sr-only" role="status" aria-live="polite">
        {feed.map((item) => {
          if (item.kind === "automatic-overlap") {
            return `${item.activity.title}: ${item.activity.detail}`;
          }
          const gallery = item.gallery;
          const processed = duplicateExcludedGalleryIds.has(gallery.id)
            && !runningDownloadStates.has(gallery.download!.state);
          return `${gallery.title}: ${processed ? duplicateProcessedDetail : downloadDetail(gallery.download!)}`;
        }).join(", ")}
      </div>
      <header>
        <div>
          <span className="eyebrow">ACTIVITY</span>
          <h2>이번 실행 활동</h2>
        </div>
        <button ref={closeButton} type="button" className="icon-button small" title="닫기" aria-label="활동 알림 닫기" onClick={onClose}>
          <FluentIcon glyph="\uE711" />
        </button>
      </header>
      <div className="activity-list">
        {feed.map((item) => {
          if (item.kind === "automatic-overlap") {
            const { activity } = item;
            return (
              <article
                key={activity.id}
                className={`activity-item automatic-overlap${activity.state === "completed" ? " complete" : " warning"}`}
              >
                <span className="activity-icon">
                  <FluentIcon glyph={activity.state === "completed" ? "\uE73E" : "\uE7BA"} />
                </span>
                <div>
                  <strong>{activity.title}</strong>
                  <span>{activity.detail}</span>
                  <small>자동 판본 분류 · 이번 실행</small>
                </div>
                <div className="activity-actions">
                  <button
                    type="button"
                    className="mini-command"
                    onClick={() => onReviewOverlap?.(activity.reviewId, activity.galleryId)}
                  >근거 보기</button>
                </div>
              </article>
            );
          }
          const gallery = item.gallery;
          const download = gallery.download!;
          const running = runningDownloadStates.has(download.state);
          const duplicateProcessed = duplicateExcludedGalleryIds.has(gallery.id) && !running;
          const complete = download.state === "completed" || duplicateProcessed;
          const warning = !duplicateProcessed && ["review_required", "failed", "interrupted"].includes(download.state);
          const retryable = !duplicateProcessed && ["failed", "interrupted", "cancelled"].includes(download.state);
          const cancellable = !duplicateProcessed && (running || warning);
          const pending = pendingEntryIds.has(download.entryId);
          const progress = displayedProgress(download);
          return (
            <article
              key={download.entryId}
              className={`activity-item${warning ? " warning" : ""}${complete ? " complete" : ""}${duplicateProcessed ? " duplicate-resolved" : ""}`}
            >
              <span className={`activity-icon${running ? " is-running" : ""}`}>
                {running ? <span className="spinner" /> : <FluentIcon glyph={complete ? "\uE73E" : "\uE7BA"} />}
              </span>
              <div>
                <strong>{gallery.title}</strong>
                <span>{duplicateProcessed ? duplicateProcessedDetail : downloadDetail(download)}</span>
                {!duplicateProcessed && (download.attempt || download.errorCode) ? (
                  <small>
                    {download.attempt ? `시도 ${download.attempt}` : ""}
                    {download.attempt && download.errorCode ? " · " : ""}
                    {download.errorCode ?? ""}
                  </small>
                ) : null}
              </div>
              <div className="activity-actions">
                {duplicateProcessed ? <b className="activity-resolution">처리 완료</b> : null}
                {!duplicateProcessed && download.state === "review_required" ? (
                  <button type="button" className="mini-command" disabled={pending} onClick={() => onReview(gallery.id)}>검토</button>
                ) : null}
                {retryable ? (
                  <button type="button" className="mini-command" disabled={pending} onClick={() => onRetry(gallery.id)}>재시도</button>
                ) : null}
                {cancellable ? (
                  <button type="button" className="mini-command" disabled={pending} onClick={() => onCancel(gallery.id)}>취소</button>
                ) : null}
                {!duplicateProcessed && !warning && !retryable ? (
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
        {feed.length === 0 ? (
          <div className="activity-empty">
            <FluentIcon glyph="\uE823" />
            <strong>이번 실행의 활동이 없습니다.</strong>
            <span>새 다운로드와 자동 판본 분류가 여기에 쌓입니다.</span>
          </div>
        ) : null}
      </div>
    </aside>
  );
}
