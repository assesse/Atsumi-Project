import { useEffect, useRef, useState } from "react";
import type { DownloadOverlapAutomationHistoryItem } from "../api/contracts";
import type { DownloadState, Gallery, GalleryId } from "../core/types";
import { FluentIcon } from "./FluentIcon";

type ActivityDrawerProps = {
  open: boolean;
  galleries: Gallery[];
  sessionDownloads: SessionDownloadActivity[];
  automaticOverlapActivities?: AutomaticOverlapActivity[];
  automationHistory?: DownloadOverlapAutomationHistoryItem[];
  automationHistoryLoading?: boolean;
  automationHistoryError?: string | null;
  automationHistoryTotalItems?: number;
  automationHistoryUnacknowledgedItems?: number;
  automationHistoryPendingReviewIds?: ReadonlySet<string>;
  danbooruActivities?: DanbooruSessionActivity[];
  duplicateExcludedGalleryIds?: ReadonlySet<GalleryId>;
  onClose: () => void;
  onReview: (id: GalleryId) => void;
  onReviewOverlap?: (reviewId: string, galleryId: GalleryId) => void;
  onAcknowledgeAutomationHistory?: (reviewId: string) => void;
  onRestoreAutomationExclusions?: (reviewId: string, galleryIds: GalleryId[]) => void;
  onRetryAutomationHistory?: () => void;
  onLoadMoreAutomationHistory?: () => void;
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

export type DanbooruSessionActivity = {
  id: string;
  postId: number;
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

const duplicateProcessedDetail = "중복 처리 완료 · 목록에서 제외";

const stateDetail: Partial<Record<NonNullable<Gallery["download"]>["state"], string>> = {
  queued: "대기 중",
  resolving_metadata: "정보 확인 중",
  downloading: "다운로드 중",
  hashing: "해시 확인 중",
  verifying: "검증 중",
  retry_wait: "재시도 대기",
  review_required: "검토 필요",
  interrupted: "중단됨",
  failed: "실패",
  completed: "완료",
  quarantined: "격리됨",
  cancelled: "취소됨",
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

const automationReviewStateLabel: Record<DownloadOverlapAutomationHistoryItem["reviewState"], string> = {
  pending: "직접 검토 필요",
  resolved: "분류 완료",
  cancelled: "신규 판본 취소",
  stale: "상태 변경됨",
};

const automaticHistoryDetail = (item: DownloadOverlapAutomationHistoryItem): string => {
  const decisions = [
    item.removeExistingCount > 0 ? `기존 판본 ${item.removeExistingCount}개 제외` : "",
    item.removeIncomingCount > 0 ? `신규 판본 ${item.removeIncomingCount}개 제외` : "",
  ].filter(Boolean);
  return decisions.length > 0 ? `자동 분류 · ${decisions.join(" · ")}` : "자동 판본 분류 기록";
};

const formatOccurredAt = (occurredAt: string): string => {
  const timestamp = Date.parse(occurredAt);
  if (!Number.isFinite(timestamp)) return occurredAt;
  return new Intl.DateTimeFormat("ko-KR", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(timestamp);
};

export function ActivityDrawer({
  open,
  galleries,
  sessionDownloads,
  automaticOverlapActivities = [],
  automationHistory = [],
  automationHistoryLoading = false,
  automationHistoryError = null,
  automationHistoryTotalItems = 0,
  automationHistoryUnacknowledgedItems = 0,
  automationHistoryPendingReviewIds = new Set(),
  danbooruActivities = [],
  duplicateExcludedGalleryIds = new Set(),
  onClose,
  onReview,
  onReviewOverlap,
  onAcknowledgeAutomationHistory,
  onRestoreAutomationExclusions,
  onRetryAutomationHistory,
  onLoadMoreAutomationHistory,
  onRetry,
  onCancel,
  pendingEntryIds = new Set(),
}: ActivityDrawerProps) {
  const closeButton = useRef<HTMLButtonElement>(null);
  const [activeSection, setActiveSection] = useState<"session" | "automation">("session");

  useEffect(() => {
    if (open) window.requestAnimationFrame(() => closeButton.current?.focus());
  }, [open]);

  if (!open) return null;
  const galleryById = new Map(galleries.map((gallery) => [gallery.id, gallery]));
  const downloadActivities = sessionDownloads.flatMap(({ galleryId, occurredAt }) => {
    const gallery = galleryById.get(galleryId);
    return gallery?.download ? [{ kind: "download" as const, gallery, occurredAt }] : [];
  });
  const latestAutomaticActivities = [...automaticOverlapActivities]
    .sort((left, right) => right.occurredAt - left.occurredAt)
    .filter((activity, index, activities) =>
      activities.findIndex((candidate) => candidate.reviewId === activity.reviewId) === index);
  const feed = [
    ...downloadActivities,
    ...latestAutomaticActivities.map((activity) => ({
      kind: "automatic-overlap" as const,
      activity,
      occurredAt: activity.occurredAt,
    })),
    ...danbooruActivities.map((activity) => ({
      kind: "danbooru" as const,
      activity,
      occurredAt: activity.occurredAt,
    })),
  ]
    .sort((left, right) => right.occurredAt - left.occurredAt);
  const liveAutomaticByReviewId = new Map(latestAutomaticActivities.map((activity) => [activity.reviewId, activity]));
  const persistedReviewIds = new Set(automationHistory.map((item) => item.reviewId));
  const automationReviewRows = [
    ...automationHistory.map((item) => ({
      kind: "persisted" as const,
      item,
      live: liveAutomaticByReviewId.get(item.reviewId),
      occurredAt: Date.parse(item.occurredAt) || 0,
    })),
    ...latestAutomaticActivities
      .filter((activity) => !persistedReviewIds.has(activity.reviewId))
      .map((activity) => ({ kind: "live" as const, activity, occurredAt: activity.occurredAt })),
  ].sort((left, right) => right.occurredAt - left.occurredAt);

  return (
    <aside
      id="activity-panel"
      className="activity-panel"
      aria-label="활동 기록"
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
          if (item.kind === "danbooru") return `${item.activity.title}: ${item.activity.detail}`;
          const gallery = item.gallery;
          const processed = duplicateExcludedGalleryIds.has(gallery.id)
            && !runningDownloadStates.has(gallery.download!.state);
          return `${gallery.title}: ${processed ? duplicateProcessedDetail : downloadDetail(gallery.download!)}`;
        }).join(", ")}
      </div>
      <header>
        <div>
          <span className="eyebrow">ACTIVITY</span>
          <h2>활동 기록</h2>
        </div>
        <button ref={closeButton} type="button" className="icon-button small" title="닫기" aria-label="활동 기록 닫기" onClick={onClose}>
          <FluentIcon glyph="\uE711" />
        </button>
      </header>
      <nav className="activity-section-tabs" role="tablist" aria-label="활동 기록 분류">
        <button
          type="button"
          role="tab"
          aria-selected={activeSection === "session"}
          aria-controls="activity-session-panel"
          className={`mini-command${activeSection === "session" ? " is-active" : ""}`}
          onClick={() => setActiveSection("session")}
        >이번 실행</button>
        <button
          type="button"
          role="tab"
          aria-selected={activeSection === "automation"}
          aria-controls="activity-automation-panel"
          className={`mini-command${activeSection === "automation" ? " is-active" : ""}`}
          onClick={() => setActiveSection("automation")}
        >
          자동분류 검토{automationHistoryUnacknowledgedItems > 0 ? ` ${automationHistoryUnacknowledgedItems}` : ""}
        </button>
      </nav>
      {activeSection === "session" ? <div id="activity-session-panel" role="tabpanel" className="activity-list">
        {feed.map((item) => {
          if (item.kind === "danbooru") {
            const { activity } = item;
            return (
              <article key={activity.id} className={`activity-item${activity.state === "completed" ? " complete" : " warning"}`}>
                <span className="activity-icon"><FluentIcon glyph={activity.state === "completed" ? "\uE73E" : "\uE7BA"} /></span>
                <div>
                  <strong>{activity.title}</strong>
                  <span>{activity.detail}</span>
                  <small>Danbooru #{activity.postId}</small>
                </div>
              </article>
            );
          }
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
            <strong>기록이 없습니다.</strong>
            <span>다운로드와 자동 처리가 여기에 표시됩니다.</span>
          </div>
        ) : null}
      </div> : (
        <div id="activity-automation-panel" role="tabpanel" className="activity-list">
          <p className="activity-history-note" title="탐색·목록 제외만 해제하며 격리된 실제 파일은 복원하지 않습니다.">
            목록 복원은 탐색·목록 제외만 해제합니다. 격리된 실제 파일은 복원하지 않습니다.
          </p>
          {automationReviewRows.map((row) => {
            if (row.kind === "live") {
              const { activity } = row;
              return (
                <article key={`live:${activity.reviewId}`} className={`activity-item automatic-overlap${activity.state === "completed" ? " complete" : " warning"}`}>
                  <span className="activity-icon"><FluentIcon glyph={activity.state === "completed" ? "\uE73E" : "\uE7BA"} /></span>
                  <div>
                    <strong>{activity.title}</strong>
                    <span>{activity.detail}</span>
                    <small>이번 실행 · 영구 기록 반영 대기</small>
                  </div>
                  <div className="activity-actions">
                    <button type="button" className="mini-command" onClick={() => onReviewOverlap?.(activity.reviewId, activity.galleryId)}>근거 보기</button>
                  </div>
                </article>
              );
            }
            const { item, live } = row;
            const pending = automationHistoryPendingReviewIds.has(item.reviewId);
            const acknowledged = Boolean(item.acknowledgedAt);
            return (
              <article key={item.reviewId} className={`activity-item automatic-overlap activity-history-item${item.reviewState === "pending" || item.reviewState === "stale" ? " warning" : " complete"}`}>
                <span className="activity-icon"><FluentIcon glyph={item.reviewState === "pending" || item.reviewState === "stale" ? "\uE7BA" : "\uE73E"} /></span>
                <div>
                  <strong>{item.title}</strong>
                  <span>{live?.detail ?? automaticHistoryDetail(item)}</span>
                  <small>{automationReviewStateLabel[item.reviewState]} · {formatOccurredAt(item.occurredAt)}</small>
                </div>
                <div className="activity-actions automation-history-actions">
                  {acknowledged ? <b className="activity-resolution">확인 완료</b> : null}
                  <button type="button" className="mini-command" disabled={pending} onClick={() => onReviewOverlap?.(item.reviewId, item.incomingGalleryId)}>근거 보기</button>
                  {!acknowledged && item.removedGalleryIds.length > 0 ? (
                    <button
                      type="button"
                      className="mini-command"
                      title="탐색·목록 제외만 해제합니다. 격리된 실제 파일은 복원하지 않습니다."
                      disabled={pending}
                      onClick={() => onRestoreAutomationExclusions?.(item.reviewId, item.removedGalleryIds)}
                    >목록에 복원</button>
                  ) : null}
                  {!acknowledged ? (
                    <button type="button" className="mini-command" disabled={pending} onClick={() => onAcknowledgeAutomationHistory?.(item.reviewId)}>확인 완료</button>
                  ) : null}
                </div>
              </article>
            );
          })}
          {automationHistoryLoading && automationReviewRows.length === 0 ? (
            <div className="activity-empty" role="status"><span className="spinner" /><strong>자동분류 기록을 불러오는 중입니다.</strong></div>
          ) : null}
          {automationHistoryError ? (
            <div className="activity-empty" role="alert">
              <FluentIcon glyph="\uE7BA" />
              <strong>자동분류 기록을 불러오지 못했습니다.</strong>
              <span>{automationHistoryError}</span>
              <button type="button" className="mini-command" onClick={onRetryAutomationHistory}>다시 시도</button>
            </div>
          ) : null}
          {!automationHistoryLoading && !automationHistoryError && automationReviewRows.length === 0 ? (
            <div className="activity-empty">
              <FluentIcon glyph="\uE823" />
              <strong>검토할 자동분류 기록이 없습니다.</strong>
              <span>자동 판본 분류 결과가 생기면 여기에 영구 보관됩니다.</span>
            </div>
          ) : null}
          {automationHistory.length < automationHistoryTotalItems ? (
            <button type="button" className="mini-command activity-history-more" disabled={automationHistoryLoading} onClick={onLoadMoreAutomationHistory}>
              {automationHistoryLoading ? "불러오는 중" : "더 보기"}
            </button>
          ) : null}
        </div>
      )}
    </aside>
  );
}
