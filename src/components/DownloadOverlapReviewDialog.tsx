import {
  cloneElement,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type CSSProperties,
  type ReactElement,
} from "react";
import { createPortal } from "react-dom";
import type {
  DownloadOverlapAutomationHistoryItem,
  DownloadOverlapCandidate,
  DownloadOverlapDecisionAudit,
  DownloadOverlapDecisionRequest,
  DownloadOverlapGalleryRef,
  DownloadOverlapMergeRequest,
  DownloadOverlapPagePair,
  DownloadOverlapReview,
  SettingsSnapshot,
} from "../api/contracts";
import type { GalleryId } from "../core/types";
import {
  buildDownloadOverlapAlignment,
  formatPageRanges,
  uniquePagesForSide,
} from "../downloadOverlap/alignment";
import { galleryPreviewPreset } from "../layout/galleryPreviewPresets";
import { buildStrictOverlapPlan, DOWNLOAD_OVERLAP_AUTO_HELP } from "../state/downloadOverlapAuto";
import {
  buildDownloadOverlapContainmentGroup,
  type DownloadOverlapContainmentGroup,
  type DownloadOverlapContainmentItem,
} from "../state/downloadOverlapContainment";
import { artifactPageThumbnailKey, type ThumbnailClient, type ThumbnailKey } from "../thumbnail";
import { FluentIcon } from "./FluentIcon";
import { GalleryThumbnail } from "./GalleryThumbnail";
import "./DownloadOverlapReviewDialog.css";

type Props = {
  completedPair?: boolean;
  open: boolean;
  review?: DownloadOverlapReview;
  loading?: boolean;
  error?: string | null;
  decisionPending?: boolean;
  automationHistoryItem?: DownloadOverlapAutomationHistoryItem;
  automationHistoryPending?: boolean;
  automationReviewSequence?: { position: number; total: number; canPrevious: boolean; canNext: boolean };
  containmentGroup?: DownloadOverlapContainmentGroup;
  containmentLoading?: boolean;
  batchProgress?: { completed: number; total: number };
  browserFixture?: boolean;
  autoMode?: SettingsSnapshot["downloadOverlapAutoMode"];
  previewWidth: number;
  thumbnailClient?: ThumbnailClient;
  onClose: () => void;
  onRetry: () => void;
  onRescan?: () => void;
  onDecision: (request: DownloadOverlapDecisionRequest) => void;
  onApplyContainmentBatch?: (itemKeys: string[]) => void;
  onMergePages?: (request: DownloadOverlapMergeRequest) => void;
  onAcknowledgeAutomationHistory?: (reviewId: string) => void;
  onRestoreAutomationExclusions?: (reviewId: string, galleryIds: GalleryId[]) => void;
  onPreviousAutomationReview?: () => void;
  onNextAutomationReview?: () => void;
};

const relationLabel: Record<DownloadOverlapCandidate["relation"], string> = {
  near_equivalent: "거의 같은 판본",
  incoming_contains_existing: "신규 앨범 B가 기존 앨범 A를 포함",
  existing_contains_incoming: "기존 앨범 A가 신규 앨범 B를 포함",
  partial_overlap: "강한 부분 겹침",
  translation_edition: "번역·가공 판본으로 보이는 일치",
};

const percent = (value: number) => `${Math.round(Math.max(0, Math.min(1, value)) * 100)}%`;

type ArtifactOutcome = "kept" | "excluded" | "pending" | "uncertain";

type ArtifactPresentation = {
  outcome: ArtifactOutcome;
  reason: string;
};

type ComparisonOutcome = {
  existing?: ArtifactPresentation;
  incoming?: ArtifactPresentation;
};

type AutomaticDecisionSnapshot = {
  candidateId?: string;
  incomingGalleryId?: number;
  existingGalleryId?: number;
  winner: "incoming" | "existing";
  preferenceReason:
    | "containment"
    | "complete_containment"
    | "omnibus_containment"
    | "uncensored"
    | "page_count"
    | "stable_existing";
  metrics?: {
    loserCoverage?: number;
    pageDifference?: number;
  };
};

const latestDecision = (
  review: DownloadOverlapReview,
  predicate: (decision: DownloadOverlapDecisionAudit) => boolean,
): DownloadOverlapDecisionAudit | undefined => {
  const decisions = review.decisions ?? [];
  for (let index = decisions.length - 1; index >= 0; index -= 1) {
    if (predicate(decisions[index]!)) return decisions[index];
  }
  return undefined;
};

const candidateDecisionAudit = (
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
): DownloadOverlapDecisionAudit | undefined => {
  const expectedAction = candidate.decision === "existing_removed"
    ? "remove_existing_continue"
    : candidate.decision === "keep_both"
      ? "keep_both_continue"
      : candidate.decision === "false_positive"
        ? "false_positive_continue"
        : null;
  if (!expectedAction) return undefined;
  return latestDecision(review, (decision) =>
    decision.candidateId === candidate.candidateId && decision.action === expectedAction);
};

const terminalIncomingAudit = (
  review: DownloadOverlapReview,
): DownloadOverlapDecisionAudit | undefined => latestDecision(review, (decision) =>
  decision.action === "remove_incoming");

const automaticDecisionSnapshot = (
  audit: DownloadOverlapDecisionAudit,
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
): AutomaticDecisionSnapshot | null => {
  if (audit.actor !== "automation" || !audit.featureSnapshotJson) return null;
  try {
    const parsed = JSON.parse(audit.featureSnapshotJson) as Record<string, unknown>;
    const winner = parsed.winner;
    const preferenceReason = parsed.preferenceReason;
    const expectedWinner = audit.action === "remove_existing_continue"
      ? "incoming"
      : audit.action === "remove_incoming"
        ? "existing"
        : null;
    if ((winner !== "incoming" && winner !== "existing")
      || winner !== expectedWinner
      || ![
        "containment",
        "complete_containment",
        "omnibus_containment",
        "uncensored",
        "page_count",
        "stable_existing",
      ].includes(String(preferenceReason))) return null;
    if (typeof parsed.candidateId === "string" && parsed.candidateId !== candidate.candidateId) return null;
    if (typeof parsed.incomingGalleryId === "number"
      && parsed.incomingGalleryId !== Number(review.incoming.galleryId)) return null;
    if (typeof parsed.existingGalleryId === "number"
      && parsed.existingGalleryId !== Number(candidate.existing.galleryId)) return null;
    const rawMetrics = parsed.metrics;
    const metrics = typeof rawMetrics === "object" && rawMetrics !== null && !Array.isArray(rawMetrics)
      ? rawMetrics as Record<string, unknown>
      : undefined;
    return {
      ...(typeof parsed.candidateId === "string" ? { candidateId: parsed.candidateId } : {}),
      ...(typeof parsed.incomingGalleryId === "number" ? { incomingGalleryId: parsed.incomingGalleryId } : {}),
      ...(typeof parsed.existingGalleryId === "number" ? { existingGalleryId: parsed.existingGalleryId } : {}),
      winner,
      preferenceReason: preferenceReason as AutomaticDecisionSnapshot["preferenceReason"],
      ...(metrics ? {
        metrics: {
          ...(typeof metrics.loserCoverage === "number" ? { loserCoverage: metrics.loserCoverage } : {}),
          ...(typeof metrics.pageDifference === "number" ? { pageDifference: metrics.pageDifference } : {}),
        },
      } : {}),
    };
  } catch {
    return null;
  }
};

const automaticArtifactReason = (
  snapshot: AutomaticDecisionSnapshot,
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
  side: "existing" | "incoming",
): string => {
  const isWinner = snapshot.winner === side;
  const pageDifference = Math.max(
    0,
    Math.round(snapshot.metrics?.pageDifference
      ?? Math.abs(candidate.existing.pageCount - review.incoming.pageCount)),
  );
  const loserCoverage = snapshot.metrics?.loserCoverage
    ?? (snapshot.winner === "incoming" ? candidate.existingCoverage : candidate.incomingCoverage);
  switch (snapshot.preferenceReason) {
    case "uncensored":
      return isWinner
        ? `무검열 표식 우선 · 신뢰도 ${percent(candidate.confidence)}`
        : "상대 판본의 무검열 표식 우선";
    case "page_count":
      return isWinner
        ? `추가 ${pageDifference}장 · 신뢰도 ${percent(candidate.confidence)}`
        : `상대 판본이 ${pageDifference}장 더 많음`;
    case "stable_existing":
      return isWinner ? "동일 조건 · 기존 보유본 우선" : "동일 조건 · 기존 보유본 우선";
    case "containment":
      return isWinner
        ? `상대 판본 ${percent(loserCoverage)} 포함`
        : `보존판에 ${percent(loserCoverage)} 포함`;
    case "complete_containment":
      return isWinner
        ? `완전 포함 · 상대 판본 ${percent(loserCoverage)} 포함`
        : `보존판에 모든 페이지 포함`;
    case "omnibus_containment":
      return isWinner
        ? `큰 합본 · 상대 판본 ${percent(loserCoverage)} 포함`
        : `큰 합본에 ${percent(loserCoverage)} 포함`;
  }
};

const artifactReason = (
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
  side: "existing" | "incoming",
  outcome: ArtifactOutcome,
): string => {
  if (outcome === "pending") return "남은 후보 검토 중";
  if (outcome === "uncertain") return "현재 상태 재확인 필요";

  const candidateAudit = candidateDecisionAudit(review, candidate);
  const incomingAudit = review.state === "cancelled"
    ? terminalIncomingAudit(review)
    : undefined;
  const terminalIncomingDecisionApplies = Boolean(incomingAudit)
    && review.state === "cancelled"
    && !(side === "existing" && outcome === "excluded");
  if (terminalIncomingDecisionApplies
    && incomingAudit?.candidateId
    && incomingAudit.candidateId !== candidate.candidateId) {
    return side === "incoming"
      ? "다른 후보 판정으로 신규 B 최종 제외"
      : "신규 B 최종 제외로 유지";
  }
  const audit = terminalIncomingDecisionApplies ? incomingAudit : candidateAudit;
  if (audit?.actor === "automation") {
    const snapshot = automaticDecisionSnapshot(audit, review, candidate);
    return snapshot
      ? automaticArtifactReason(snapshot, review, candidate, side)
      : `자동 판정 · 신뢰도 ${percent(candidate.confidence)}`;
  }
  if (audit?.action === "remove_existing_continue") {
    return side === "existing" ? "수동 제거 선택" : "기존 A 제거 후 계속";
  }
  if (audit?.action === "remove_incoming") {
    return side === "incoming" ? "수동 제거 선택" : "신규 B 제거 선택";
  }

  if (review.state !== "cancelled"
    && candidate.decision === "false_positive"
    && outcome === "kept") return "오탐 판정 · 둘 다 보존";
  if (review.state !== "cancelled"
    && candidate.decision === "keep_both"
    && outcome === "kept") return "둘 다 보존 선택";

  if (side === "existing" && candidate.decision === "existing_removed") {
    return "기존 제외 상태 반영";
  }
  if (side === "incoming" && review.state === "cancelled") return "신규 제거 판정";
  if (side === "existing" && review.state === "cancelled") return "신규 B 제거 후 유지";
  return outcome === "kept" ? "검토 완료" : "제외 판정";
};

const comparisonOutcome = (
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
): ComparisonOutcome | null => {
  const incomingOutcome: ArtifactOutcome | undefined = review.state === "resolved"
    ? "kept"
    : review.state === "cancelled"
      ? "excluded"
      : candidate.decision
        ? review.state === "pending" ? "pending" : "uncertain"
        : undefined;
  const existingOutcome: ArtifactOutcome | undefined = candidate.decision === "existing_removed"
    ? "excluded"
    : candidate.decision === "keep_both" || candidate.decision === "false_positive"
      ? "kept"
      : review.state === "cancelled"
        ? "kept"
        : undefined;

  if (!existingOutcome && !incomingOutcome) return null;
  return {
    ...(existingOutcome ? {
      existing: {
        outcome: existingOutcome,
        reason: artifactReason(review, candidate, "existing", existingOutcome),
      },
    } : {}),
    ...(incomingOutcome ? {
      incoming: {
        outcome: incomingOutcome,
        reason: artifactReason(review, candidate, "incoming", incomingOutcome),
      },
    } : {}),
  };
};

const candidateOutcomeLabel = (
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
): { label: string; tone: "kept" | "excluded" | "pending" } | null => {
  const outcome = comparisonOutcome(review, candidate);
  if (!outcome) return null;
  if (outcome.existing?.outcome === "excluded" && outcome.incoming?.outcome === "kept") {
    return { label: "A 제외", tone: "excluded" };
  }
  if (outcome.existing?.outcome === "kept" && outcome.incoming?.outcome === "excluded") {
    return { label: "B 제외", tone: "excluded" };
  }
  if (outcome.existing?.outcome === "kept" && outcome.incoming?.outcome === "kept") {
    return {
      label: candidate.decision === "false_positive" ? "오탐 · 둘 다 유지" : "둘 다 유지",
      tone: "kept",
    };
  }
  if (outcome.existing?.outcome === "excluded" && outcome.incoming?.outcome === "excluded") {
    return { label: "둘 다 제외", tone: "excluded" };
  }
  if (outcome.incoming?.outcome === "pending") {
    return {
      label: outcome.existing?.outcome === "excluded" ? "A 제외 · 계속 검토" : "A 유지 · 계속 검토",
      tone: "pending",
    };
  }
  if (outcome.incoming?.outcome === "uncertain") {
    return {
      label: outcome.existing?.outcome === "excluded" ? "A 제외 · 상태 확인" : "A 유지 · 상태 확인",
      tone: "pending",
    };
  }
  return outcome.incoming?.outcome === "kept" ? { label: "B 유지", tone: "kept" } : null;
};

type PageHoverPreview = {
  anchor: HTMLElement;
  portalHost: HTMLDialogElement;
  thumbnailKey: ThumbnailKey;
  alt: string;
  caption: string;
  identity: string;
};

type PreviewDimensions = { width: number; height: number };

const HOVER_PREVIEW_GAP = 12;
const HOVER_PREVIEW_MARGIN = 10;

const hoverPreviewLayout = (
  preview: PageHoverPreview,
  desiredWidth: number,
  intrinsic: PreviewDimensions,
) => {
  const viewport = {
    left: 0,
    top: 0,
    right: Math.max(1, window.innerWidth),
    bottom: Math.max(1, window.innerHeight),
  };
  const dialogRect = preview.portalHost.getBoundingClientRect();
  const hasDialogBounds = dialogRect.width > 0 && dialogRect.height > 0;
  const bounds = hasDialogBounds ? {
    left: Math.max(viewport.left, dialogRect.left),
    top: Math.max(viewport.top, dialogRect.top),
    right: Math.min(viewport.right, dialogRect.right),
    bottom: Math.min(viewport.bottom, dialogRect.bottom),
  } : viewport;
  const availableWidth = Math.max(1, bounds.right - bounds.left - HOVER_PREVIEW_MARGIN * 2);
  const availableHeight = Math.max(1, bounds.bottom - bounds.top - HOVER_PREVIEW_MARGIN * 2);
  const aspectRatio = intrinsic.width > 0 && intrinsic.height > 0
    ? intrinsic.width / intrinsic.height
    : 2 / 3;
  let width = Math.min(desiredWidth, availableWidth);
  let height = width / aspectRatio;
  if (height > availableHeight) {
    height = availableHeight;
    width = height * aspectRatio;
  }

  const anchor = preview.anchor.getBoundingClientRect();
  const minimumLeft = bounds.left + HOVER_PREVIEW_MARGIN;
  const maximumLeft = bounds.right - HOVER_PREVIEW_MARGIN - width;
  const preferredRight = anchor.right + HOVER_PREVIEW_GAP;
  const preferredLeft = anchor.left - HOVER_PREVIEW_GAP - width;
  const left = preferredRight + width <= bounds.right - HOVER_PREVIEW_MARGIN
    ? preferredRight
    : preferredLeft >= minimumLeft
      ? preferredLeft
      : Math.max(minimumLeft, Math.min(anchor.left + anchor.width / 2 - width / 2, maximumLeft));
  const minimumTop = bounds.top + HOVER_PREVIEW_MARGIN;
  const maximumTop = bounds.bottom - HOVER_PREVIEW_MARGIN - height;
  const top = Math.max(
    minimumTop,
    Math.min(anchor.top + anchor.height / 2 - height / 2, maximumTop),
  );

  return { left, top, width, height };
};

function PageHoverPreviewLayer({ preview, previewWidth, thumbnailClient }: {
  preview: PageHoverPreview;
  previewWidth: number;
  thumbnailClient?: ThumbnailClient;
}) {
  const [intrinsic, setIntrinsic] = useState<PreviewDimensions>({ width: 2, height: 3 });
  const [, setLayoutRevision] = useState(0);
  const normalizedPreviewWidth = galleryPreviewPreset(previewWidth).width;
  const layout = hoverPreviewLayout(preview, normalizedPreviewWidth, intrinsic);
  const handleTerminalSnapshot = useCallback((snapshot: {
    status: "resolved" | "error";
    width?: number;
    height?: number;
  }) => {
    const width = snapshot.width;
    const height = snapshot.height;
    if (snapshot.status !== "resolved" || !width || !height) return;
    setIntrinsic((current) => current.width === width && current.height === height
      ? current
      : { width, height });
  }, []);

  useLayoutEffect(() => {
    const refresh = () => setLayoutRevision((revision) => revision + 1);
    window.addEventListener("resize", refresh);
    document.addEventListener("scroll", refresh, true);
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(refresh) : null;
    observer?.observe(preview.anchor);
    observer?.observe(preview.portalHost);
    return () => {
      window.removeEventListener("resize", refresh);
      document.removeEventListener("scroll", refresh, true);
      observer?.disconnect();
    };
  }, [preview.anchor, preview.portalHost]);

  return (
    <GalleryThumbnail
      className="download-overlap-page-hover-preview"
      thumbnailKey={preview.thumbnailKey}
      consumer="review"
      priority="critical"
      client={thumbnailClient}
      alt={preview.alt}
      aria-hidden="true"
      data-preview-width={normalizedPreviewWidth}
      onTerminalSnapshot={handleTerminalSnapshot}
      style={{
        position: "fixed",
        left: layout.left,
        top: layout.top,
        width: layout.width,
        height: layout.height,
      }}
    >
      <span className="download-overlap-page-hover-caption">{preview.caption}</span>
    </GalleryThumbnail>
  );
}

function ArtifactSummary({ gallery, label, page, presentation, thumbnailClient }: {
  gallery: DownloadOverlapGalleryRef;
  label: string;
  page: number;
  presentation?: ArtifactPresentation;
  thumbnailClient?: ThumbnailClient;
}) {
  const outcome = presentation?.outcome;
  const outcomeLabel = outcome === "kept"
    ? "이 검토에서 보존"
    : outcome === "excluded"
      ? "이 검토에서 제외"
      : outcome === "pending"
        ? "검토 계속 중"
        : outcome === "uncertain"
          ? "상태 확인 필요"
          : null;
  return (
    <article
      className={`download-overlap-artifact${outcome ? ` is-${outcome}` : ""}`}
      aria-label={`${label}${outcomeLabel ? ` · ${outcomeLabel}` : ""}${presentation?.reason ? ` · 근거 ${presentation.reason}` : ""}`}
    >
      <GalleryThumbnail
        className="download-overlap-cover"
        thumbnailKey={artifactPageThumbnailKey(gallery.entryId, page, Number(gallery.galleryId) % 6)}
        consumer="review"
        priority="critical"
        client={thumbnailClient}
        alt={`${gallery.title} ${page}페이지`}
      />
      <div>
        <div className="download-overlap-artifact-label-row">
          <strong className="download-overlap-edition-label">{label}</strong>
          {outcomeLabel ? (
            <span className={`download-overlap-artifact-outcome is-${outcome}`}>
              <FluentIcon glyph={outcome === "kept" ? "\uE73E" : outcome === "excluded" ? "\uE711" : "\uE7BA"} />
              {outcomeLabel}
            </span>
          ) : null}
        </div>
        {presentation?.reason ? (
          <p className="download-overlap-artifact-reason">
            <span>근거</span>
            {presentation.reason}
          </p>
        ) : null}
        <strong className="download-overlap-artifact-title">{gallery.title}</strong>
        <span>{gallery.artists.join(", ") || "작가 정보 없음"}</span>
        <span>#{gallery.galleryId} · {gallery.pageCount}p</span>
      </div>
    </article>
  );
}

type MergePageSide = DownloadOverlapMergeRequest["sourceSide"];

type PageMergeSelection = {
  reviewId: string;
  reviewRevision: number;
  candidateId: string;
  sourceSide: MergePageSide;
  sourcePages: ReadonlySet<number>;
};

const MAX_PAGE_MERGE_SELECTION = 200;

type SourceMappingValidation = {
  valid: boolean;
  missingPages: number;
  reason?: "bounds" | "duplicate" | "incomplete";
};

const validateMergeSourceMapping = (
  candidate: DownloadOverlapCandidate,
  incoming: DownloadOverlapGalleryRef,
  sourceSide: MergePageSide,
): SourceMappingValidation => {
  const existingPageCount = Math.max(0, Math.floor(candidate.existing.pageCount));
  const incomingPageCount = Math.max(0, Math.floor(incoming.pageCount));
  const sourcePageCount = sourceSide === "existing" ? existingPageCount : incomingPageCount;
  const sourcePages = new Set<number>();
  const targetPages = new Set<number>();
  for (const pair of candidate.pagePairs) {
    const existingInBounds = Number.isInteger(pair.existingSourcePage)
      && pair.existingSourcePage >= 1
      && pair.existingSourcePage <= existingPageCount;
    const incomingInBounds = Number.isInteger(pair.incomingSourcePage)
      && pair.incomingSourcePage >= 1
      && pair.incomingSourcePage <= incomingPageCount;
    if (!existingInBounds || !incomingInBounds) {
      return { valid: false, missingPages: sourcePageCount, reason: "bounds" };
    }
    const sourcePage = sourceSide === "existing" ? pair.existingSourcePage : pair.incomingSourcePage;
    const targetPage = sourceSide === "existing" ? pair.incomingSourcePage : pair.existingSourcePage;
    if (sourcePages.has(sourcePage) || targetPages.has(targetPage)) {
      return { valid: false, missingPages: Math.max(0, sourcePageCount - sourcePages.size), reason: "duplicate" };
    }
    sourcePages.add(sourcePage);
    targetPages.add(targetPage);
  }
  const missingPages = Math.max(0, sourcePageCount - sourcePages.size);
  return missingPages === 0
    ? { valid: true, missingPages: 0 }
    : { valid: false, missingPages, reason: "incomplete" };
};

const mergeSourceMappingMessage = (
  side: MergePageSide,
  validation: SourceMappingValidation,
): string => {
  const sideLabel = side === "existing" ? "기존 A" : "신규 B";
  if (validation.reason === "bounds") {
    return "저장된 페이지 대응 정보가 실제 판본 범위를 벗어납니다. 최신 검토를 다시 불러온 뒤 확인해 주세요.";
  }
  if (validation.reason === "duplicate") {
    return "저장된 페이지 대응이 일대일 관계가 아니어서 교체 위치를 안전하게 정할 수 없습니다. 최신 검토를 다시 불러와 주세요.";
  }
  return `${sideLabel} 전체 페이지 중 ${validation.missingPages}장의 대응을 확인할 수 없어 자동 제외를 전제로 한 병합이 불가능합니다. 반대 판본을 원본으로 선택하거나 직접 검토해 주세요.`;
};

function PageCell({ entryId, page, side, pair, index, mergeEnabled, mergeDisabled, mergeSourceBlocked, mergeSelection, thumbnailClient, onPreviewOpen, onPreviewClose, onMergeToggle }: {
  entryId: string;
  page?: number;
  side: MergePageSide;
  pair?: DownloadOverlapPagePair;
  index: number;
  mergeEnabled?: boolean;
  mergeDisabled?: boolean;
  mergeSourceBlocked?: boolean;
  mergeSelection?: PageMergeSelection | null;
  thumbnailClient?: ThumbnailClient;
  onPreviewOpen: (preview: PageHoverPreview) => void;
  onPreviewClose: () => void;
  onMergeToggle?: (side: MergePageSide, page: number, pair?: DownloadOverlapPagePair) => void;
}) {
  if (!page) {
    return <div className="download-overlap-page-cell is-gap" aria-label="이 판본에는 대응 페이지 없음"><span>—</span></div>;
  }
  const matched = Boolean(pair);
  const matchLabel = pair
    ? pair.exactSha256 ? "SHA-256 일치" : `시각 ${percent(pair.visualSimilarity)}`
    : "이 판본에만 있음";
  const sideLabel = side === "existing" ? "기존 A" : "신규 B";
  const thumbnailKey = artifactPageThumbnailKey(entryId, page, index);
  const selectedSourcePage = pair && mergeSelection
    ? mergeSelection.sourceSide === "existing"
      ? pair.existingSourcePage
      : pair.incomingSourcePage
    : undefined;
  const selectedPair = selectedSourcePage !== undefined
    && mergeSelection?.sourcePages.has(selectedSourcePage);
  const mergeState = selectedPair
    ? side === mergeSelection?.sourceSide ? "source" : "target"
    : undefined;
  const sideLocked = Boolean(mergeSelection && mergeSelection.sourceSide !== side);
  const mergeLabel = mergeState === "source"
    ? " · 병합 원본으로 선택됨"
    : mergeState === "target"
      ? " · 이 페이지가 교체됨"
      : mergeEnabled && mergeSourceBlocked
        ? " · 이 판본에만 있는 페이지가 있어 병합 원본 선택 불가"
        : mergeEnabled && !pair
        ? " · 대응 페이지가 없어 병합 선택 불가"
        : mergeEnabled && sideLocked
          ? ` · ${mergeSelection?.sourceSide === "existing" ? "기존 A" : "신규 B"} 원본 선택을 먼저 해제해야 함`
          : mergeEnabled
            ? " · Ctrl+클릭하여 병합 원본으로 선택"
            : "";
  return (
    <GalleryThumbnail
      className={`download-overlap-page-cell ${matched ? "is-matched" : "is-unique"}${mergeEnabled && pair && !mergeSourceBlocked ? " can-merge" : ""}${mergeEnabled && mergeSourceBlocked ? " is-merge-source-blocked" : ""}${sideLocked ? " is-merge-side-locked" : ""}${mergeState ? ` is-merge-${mergeState}` : ""}`}
      thumbnailKey={thumbnailKey}
      consumer="review"
      priority={index < 6 ? "visible" : "prefetch"}
      client={thumbnailClient}
      alt={`${sideLabel} ${page}페이지`}
      aria-label={`${sideLabel} ${page}페이지 · ${matchLabel}${mergeLabel}`}
      data-merge-state={mergeState}
      onClick={(event) => {
        if (!event.ctrlKey || event.button !== 0 || !mergeEnabled || mergeDisabled) return;
        event.preventDefault();
        event.stopPropagation();
        onMergeToggle?.(side, page, pair);
      }}
      onMouseEnter={(event) => {
        const portalHost = event.currentTarget.closest("dialog");
        if (!portalHost) return;
        onPreviewOpen({
          anchor: event.currentTarget,
          portalHost,
          thumbnailKey,
          alt: `${sideLabel} ${page}페이지 확대 미리보기`,
          caption: `${sideLabel} ${page}p · ${matchLabel}`,
          identity: `${side}:${entryId}:${page}`,
        });
      }}
      onMouseLeave={onPreviewClose}
    >
      {mergeState ? <span className="download-overlap-page-merge-state">{mergeState === "source" ? "원본" : "교체"}</span> : null}
      <span className="download-overlap-page-number">{sideLabel} {page}p</span>
      <span className="download-overlap-page-status">{matched ? "일치" : "추가"}</span>
    </GalleryThumbnail>
  );
}

function PageAlignment({ candidate, incoming, previewWidth, mergeEnabled = false, mergeDisabled = false, mergeSelection, mergeHint, thumbnailClient, onMergeToggle }: {
  candidate: DownloadOverlapCandidate;
  incoming: DownloadOverlapGalleryRef;
  previewWidth: number;
  mergeEnabled?: boolean;
  mergeDisabled?: boolean;
  mergeSelection?: PageMergeSelection | null;
  mergeHint?: string | null;
  thumbnailClient?: ThumbnailClient;
  onMergeToggle?: (side: MergePageSide, page: number, pair?: DownloadOverlapPagePair) => void;
}) {
  const [hoverPreview, setHoverPreview] = useState<PageHoverPreview | null>(null);
  const columns = useMemo(
    () => buildDownloadOverlapAlignment(candidate, incoming.pageCount),
    [candidate, incoming.pageCount],
  );
  const existingUnique = uniquePagesForSide(columns, "existing");
  const incomingUnique = uniquePagesForSide(columns, "incoming");
  const existingSourceMapping = validateMergeSourceMapping(candidate, incoming, "existing");
  const incomingSourceMapping = validateMergeSourceMapping(candidate, incoming, "incoming");
  const gridStyle = { "--overlap-page-columns": columns.length } as CSSProperties;

  useEffect(() => setHoverPreview(null), [candidate.candidateId, incoming.entryId]);

  return (
    <>
      <section className="download-overlap-page-map" aria-labelledby="download-overlap-page-map-title">
        <header>
          <div>
            <strong id="download-overlap-page-map-title">판본 페이지 정렬 · {candidate.pagePairs.length}쌍 일치</strong>
            <span>
              기존 A에만 {formatPageRanges(existingUnique)} · {existingUnique.length}장
              <b aria-hidden="true"> / </b>
              신규 B에만 {formatPageRanges(incomingUnique)} · {incomingUnique.length}장
            </span>
          </div>
          <div className="download-overlap-page-legend" aria-label="페이지 표시 범례">
            <span className="is-matched">일치</span>
            <span className="is-unique">한쪽에만 있음</span>
            <span className="is-gap">대응 없음</span>
          </div>
        </header>
        {mergeEnabled ? (
          <div className={`download-overlap-merge-guide${mergeHint ? " has-message" : ""}`} role={mergeHint ? "alert" : "note"}>
            <FluentIcon glyph={mergeHint ? "\uE7BA" : "\uE946"} />
            <span>
              {mergeHint ?? (mergeSelection
                ? `${mergeSelection.sourceSide === "existing" ? "기존 A" : "신규 B"}가 병합 원본입니다. 반대쪽의 대응 페이지만 교체되고 대상의 추가 페이지는 유지됩니다. 성공하면 원본 앨범은 제외하되 파일은 보존합니다.`
                : "교체에 사용할 원본 페이지를 Ctrl+클릭하세요. 모든 원본 페이지가 대응되는 판본만 병합할 수 있으며, 대응쌍이 없는 페이지의 순서는 임의로 정하지 않습니다.")}
            </span>
          </div>
        ) : null}
        <div className="download-overlap-alignment-scroll" data-thumbnail-scroll-root tabIndex={0} aria-label="기존 A와 신규 B 페이지 정렬표">
          <div className="download-overlap-alignment-grid" style={gridStyle}>
            <strong className="download-overlap-row-label">기존 A</strong>
            {columns.map((column, index) => (
              <PageCell key={`existing:${column.key}`} entryId={candidate.existing.entryId} page={column.existingPage} side="existing" pair={column.pair} index={index} mergeEnabled={mergeEnabled} mergeDisabled={mergeDisabled} mergeSourceBlocked={!existingSourceMapping.valid || candidate.existingUniquePages > 0} mergeSelection={mergeSelection} thumbnailClient={thumbnailClient} onPreviewOpen={setHoverPreview} onPreviewClose={() => setHoverPreview(null)} onMergeToggle={onMergeToggle} />
            ))}
            <strong className="download-overlap-row-label">신규 B</strong>
            {columns.map((column, index) => (
              <PageCell key={`incoming:${column.key}`} entryId={incoming.entryId} page={column.incomingPage} side="incoming" pair={column.pair} index={index} mergeEnabled={mergeEnabled} mergeDisabled={mergeDisabled} mergeSourceBlocked={!incomingSourceMapping.valid || candidate.incomingUniquePages > 0} mergeSelection={mergeSelection} thumbnailClient={thumbnailClient} onPreviewOpen={setHoverPreview} onPreviewClose={() => setHoverPreview(null)} onMergeToggle={onMergeToggle} />
            ))}
          </div>
        </div>
      </section>
      {hoverPreview ? createPortal(
        <PageHoverPreviewLayer
          key={hoverPreview.identity}
          preview={hoverPreview}
          previewWidth={previewWidth}
          thumbnailClient={thumbnailClient}
        />,
        hoverPreview.portalHost,
      ) : null}
    </>
  );
}

const pairedPages = (
  item: DownloadOverlapContainmentItem,
  side: "keeper" | "excluded",
): number[] => {
  const useIncoming = side === "keeper"
    ? item.keeperIsIncoming
    : !item.keeperIsIncoming;
  return item.candidate.pagePairs.map((pair) => useIncoming
    ? pair.incomingSourcePage
    : pair.existingSourcePage);
};

const containmentCoverage = (item: DownloadOverlapContainmentItem): number => (
  item.keeperIsIncoming
    ? item.candidate.existingCoverage
    : item.candidate.incomingCoverage
);

function ContainmentCandidateRow({ item, keeper, selected, processed, disabled, previewWidth, thumbnailClient, onSelected }: {
  item: DownloadOverlapContainmentItem;
  keeper: DownloadOverlapGalleryRef;
  selected: boolean;
  processed: boolean;
  disabled: boolean;
  previewWidth: number;
  thumbnailClient?: ThumbnailClient;
  onSelected: (selected: boolean) => void;
}) {
  const [evidenceOpen, setEvidenceOpen] = useState(false);
  const excludedPages = pairedPages(item, "excluded");
  const keeperPages = pairedPages(item, "keeper");
  const previewPage = excludedPages[0] ?? 1;
  const actionLabel = item.action === "remove_existing_continue"
    ? "기존 A 제외 · 완료본은 격리"
    : "신규 B 제외 · 검토 staging 취소";

  return (
    <article className={`download-overlap-containment-item${selected ? " is-selected" : ""}${processed ? " is-processed" : ""}`}>
      <label className="download-overlap-containment-check">
        <input
          type="checkbox"
          checked={selected}
          disabled={disabled || processed}
          onChange={(event) => onSelected(event.currentTarget.checked)}
        />
        <span className="download-overlap-containment-checkmark" aria-hidden="true"><FluentIcon glyph="\uE73E" /></span>
        <span className="sr-only">#{item.excluded.galleryId} 제외 대상으로 선택</span>
      </label>
      <GalleryThumbnail
        className="download-overlap-containment-cover"
        thumbnailKey={artifactPageThumbnailKey(item.excluded.entryId, previewPage, Number(item.excluded.galleryId) % 6)}
        consumer="review"
        priority="visible"
        client={thumbnailClient}
        alt={`${item.excluded.title} 대표 페이지`}
      />
      <div className="download-overlap-containment-copy">
        <div className="download-overlap-containment-heading">
          <span className="download-overlap-containment-exclude"><FluentIcon glyph="\uE711" /> 제외 대상</span>
          <strong>{item.excluded.title}</strong>
          <small>#{item.excluded.galleryId}</small>
        </div>
        <div className="download-overlap-containment-facts" aria-label={`포함률 ${percent(containmentCoverage(item))}, 제외 ${item.excluded.pageCount}페이지, 보존 ${keeper.pageCount}페이지`}>
          <span><b>완전 포함 검증</b> {percent(containmentCoverage(item))}</span>
          <span><b>분량</b> {item.excluded.pageCount}p → 합본 {keeper.pageCount}p</span>
          <span><b>대응</b> 제외본 {formatPageRanges(excludedPages)} · 합본 {formatPageRanges(keeperPages)}</span>
        </div>
        <div className="download-overlap-containment-row-actions">
          <span>{processed ? "처리 완료" : actionLabel}</span>
          <button
            type="button"
            className="text-button"
            aria-expanded={evidenceOpen}
            onClick={() => setEvidenceOpen((current) => !current)}
          >
            {evidenceOpen ? "페이지 근거 접기" : "페이지 근거 보기"}
          </button>
        </div>
      </div>
      {evidenceOpen ? (
        <div className="download-overlap-containment-evidence">
          <PageAlignment
            candidate={item.candidate}
            incoming={item.review.incoming}
            previewWidth={previewWidth}
            thumbnailClient={thumbnailClient}
          />
        </div>
      ) : null}
    </article>
  );
}

function ContainmentBatchReview({ group, ambiguousCandidates, loading, disabled, error, progress, previewWidth, thumbnailClient, selectionOverrides, onSelectionOverride, onReviewCandidate, onApply }: {
  group?: DownloadOverlapContainmentGroup;
  ambiguousCandidates: DownloadOverlapCandidate[];
  loading: boolean;
  disabled: boolean;
  error?: string | null;
  progress?: { completed: number; total: number };
  previewWidth: number;
  thumbnailClient?: ThumbnailClient;
  selectionOverrides: ReadonlyMap<string, boolean>;
  onSelectionOverride: (key: string, selected: boolean) => void;
  onReviewCandidate: (candidateId: string) => void;
  onApply?: (itemKeys: string[]) => void;
}) {
  const [submittedKeys, setSubmittedKeys] = useState<string[]>([]);
  const keeperKey = group?.keeper.entryId ?? null;
  useEffect(() => setSubmittedKeys([]), [keeperKey]);

  if (loading && !group) {
    return (
      <section className="download-overlap-containment" aria-labelledby="download-overlap-containment-title" aria-busy="true">
        <header><div><span className="eyebrow">COMBINED EDITION</span><h3 id="download-overlap-containment-title">합본 포함 검토</h3></div></header>
        <div className="download-overlap-containment-loading" role="status"><span className="spinner" /> 포함 후보를 모으는 중</div>
      </section>
    );
  }
  if (!group || group.items.length < 2) return null;

  const completed = Math.max(0, Math.min(progress?.completed ?? 0, progress?.total ?? 0));
  const total = Math.max(0, progress?.total ?? 0);
  const processedKeys = new Set(
    progress?.total === submittedKeys.length
      ? submittedKeys.slice(0, completed)
      : [],
  );
  const selectedKeys = group.items
    .filter((item) => !processedKeys.has(item.key) && selectionOverrides.get(item.key) !== false)
    .map((item) => item.key);
  const first = group.items[0];
  const keeperPage = first ? pairedPages(first, "keeper")[0] ?? 1 : 1;
  const inProgress = Boolean(progress && progress.total > 0 && progress.completed < progress.total && disabled);
  const partialFailure = Boolean(error && progress && progress.total > 0 && progress.completed < progress.total);

  return (
    <section className="download-overlap-containment" aria-labelledby="download-overlap-containment-title" aria-busy={loading || inProgress}>
      <header>
        <div>
          <span className="eyebrow">COMBINED EDITION</span>
          <h3 id="download-overlap-containment-title">합본 포함 검토</h3>
          <p>한 합본에 완전히 들어 있는 작은 판본을 한 화면에서 확인하고 함께 정리합니다.</p>
        </div>
        <span className="download-overlap-containment-count">완전 포함 {group.items.length}개</span>
      </header>

      <article className="download-overlap-containment-keeper">
        <GalleryThumbnail
          className="download-overlap-containment-keeper-cover"
          thumbnailKey={artifactPageThumbnailKey(group.keeper.entryId, keeperPage, Number(group.keeper.galleryId) % 6)}
          consumer="review"
          priority="critical"
          client={thumbnailClient}
          alt={`${group.keeper.title} 대표 페이지`}
        />
        <div>
          <span><FluentIcon glyph="\uE73E" /> 보존 합본</span>
          <strong>{group.keeper.title}</strong>
          <small>#{group.keeper.galleryId} · {group.keeper.pageCount}p · 이 판본은 유지됩니다</small>
        </div>
      </article>

      <div className="download-overlap-containment-list" aria-label="완전 포함 제외 후보">
        {group.items.map((item) => (
          <ContainmentCandidateRow
            key={item.key}
            item={item}
            keeper={group.keeper}
            selected={!processedKeys.has(item.key) && selectionOverrides.get(item.key) !== false}
            processed={processedKeys.has(item.key)}
            disabled={disabled || loading}
            previewWidth={previewWidth}
            thumbnailClient={thumbnailClient}
            onSelected={(selected) => onSelectionOverride(item.key, selected)}
          />
        ))}
      </div>

      {ambiguousCandidates.length ? (
        <section className="download-overlap-containment-ambiguous" aria-labelledby="download-overlap-ambiguous-title">
          <div>
            <strong id="download-overlap-ambiguous-title">별도 직접 검토 {ambiguousCandidates.length}개</strong>
            <span>완전 포함이 확인되지 않아 일괄 제외에 포함하지 않았습니다.</span>
          </div>
          <div>
            {ambiguousCandidates.map((candidate) => (
              <button
                type="button"
                className="text-button"
                key={candidate.candidateId}
                disabled={disabled}
                onClick={() => onReviewCandidate(candidate.candidateId)}
              >
                후보 {candidate.rank} · #{candidate.existing.galleryId} · {relationLabel[candidate.relation]}
              </button>
            ))}
          </div>
        </section>
      ) : null}

      <footer>
        <div className="download-overlap-containment-status" role="status" aria-live="polite">
          {inProgress ? (
            <><span className="spinner" /> 선택 항목 처리 중 · {completed}/{total}</>
          ) : partialFailure ? (
            <><FluentIcon glyph="\uE7BA" /> 일부 처리 후 중단 · {completed}/{total} 완료 · 처리되지 않은 선택은 유지됩니다.</>
          ) : loading ? (
            <><span className="spinner" /> 포함 후보를 다시 확인하는 중</>
          ) : (
            <><FluentIcon glyph="\uE946" /> 체크된 완전 포함 판본만 제외되며, 보존 합본은 변경되지 않습니다.</>
          )}
        </div>
        <button
          type="button"
          className="primary-button download-overlap-containment-apply"
          disabled={disabled || loading || selectedKeys.length === 0 || !onApply}
          onClick={() => {
            setSubmittedKeys(selectedKeys);
            onApply?.(selectedKeys);
          }}
        >
          {inProgress
            ? `제외 처리 중 ${completed}/${total}`
            : `선택한 ${selectedKeys.length}개 제외 · 합본 보존`}
        </button>
      </footer>
    </section>
  );
}

function ReviewAction({ help, children }: {
  help: string;
  children: ReactElement<ButtonHTMLAttributes<HTMLButtonElement>>;
}) {
  const tooltipId = useId();
  return (
    <div className="download-overlap-action-with-help">
      {cloneElement(children, { "aria-describedby": tooltipId })}
      <span id={tooltipId} role="tooltip">{help}</span>
    </div>
  );
}

function AutoPlanHelp() {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [tooltip, setTooltip] = useState<HTMLSpanElement | null>(null);
  const [position, setPosition] = useState({ left: -10_000, top: -10_000 });
  const open = hovered || focused;
  const host = trigger.current?.closest("dialog") ?? document.body;

  useLayoutEffect(() => {
    if (!open || !tooltip || !trigger.current) return;
    const place = () => {
      if (!trigger.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      const bounds = host.getBoundingClientRect();
      const inset = 8;
      const leftEdge = bounds.width > 0 ? Math.max(inset, bounds.left + inset) : inset;
      const rightEdge = bounds.width > 0 ? Math.min(window.innerWidth - inset, bounds.right - inset) : window.innerWidth - inset;
      const topEdge = bounds.height > 0 ? Math.max(inset, bounds.top + inset) : inset;
      const bottomEdge = bounds.height > 0 ? Math.min(window.innerHeight - inset, bounds.bottom - inset) : window.innerHeight - inset;
      const box = tooltip.getBoundingClientRect();
      const preferredTop = anchor.bottom + inset + box.height <= bottomEdge
        ? anchor.bottom + inset : anchor.top - inset - box.height;
      setPosition({
        left: Math.max(leftEdge, Math.min(anchor.right - box.width, rightEdge - box.width)),
        top: Math.max(topEdge, Math.min(preferredTop, bottomEdge - box.height)),
      });
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [host, open, tooltip]);

  return <>
    <button
      ref={trigger}
      type="button"
      className="setting-help-trigger"
      aria-label="자동 판정 기준"
      aria-describedby={open ? id : undefined}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        event.stopPropagation();
        setHovered(false);
        setFocused(false);
        event.currentTarget.blur();
      }}
    ><FluentIcon glyph="\uE946" /></button>
    {open ? createPortal(
      <span ref={setTooltip} id={id} role="tooltip" className="setting-help-tooltip" style={{ ...position, whiteSpace: "pre-line" }}>
        {DOWNLOAD_OVERLAP_AUTO_HELP}
      </span>, host,
    ) : null}
  </>;
}

export function DownloadOverlapReviewDialog({ open, review, loading = false, error = null, decisionPending: reviewDecisionPending = false, automationHistoryItem, automationHistoryPending = false, automationReviewSequence, containmentGroup, containmentLoading = false, batchProgress, browserFixture = false, autoMode = "off", previewWidth, thumbnailClient, onClose, onRetry, onRescan, onDecision, onApplyContainmentBatch, onMergePages, onAcknowledgeAutomationHistory, onRestoreAutomationExclusions, onPreviousAutomationReview, onNextAutomationReview, completedPair = false }: Props) {
  const decisionPending = reviewDecisionPending || automationHistoryPending;
  const historyItem = automationHistoryItem?.reviewId === review?.reviewId
    && automationHistoryItem?.incomingGalleryId === review?.incoming.galleryId
    ? automationHistoryItem : undefined;
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const [candidateId, setCandidateId] = useState<string | null>(null);
  const [containmentSelectionOverrides, setContainmentSelectionOverrides] = useState<Map<string, boolean>>(() => new Map());
  const containmentSelectionScope = useRef<string | null>(null);
  const [pageMergeSelection, setPageMergeSelection] = useState<PageMergeSelection | null>(null);
  const [pageMergeHint, setPageMergeHint] = useState<string | null>(null);
  const previousOpen = useRef(open);

  const pendingCandidates = useMemo(
    () => review?.candidates.filter((candidate) => candidate.decision === undefined) ?? [],
    [review],
  );
  const candidate = review?.candidates.find((item) => item.candidateId === candidateId)
    ?? pendingCandidates[0]
    ?? review?.candidates[0];
  const activePageMergeSelection = pageMergeSelection
    && pageMergeSelection.reviewId === review?.reviewId
    && pageMergeSelection.reviewRevision === review?.revision
    && pageMergeSelection.candidateId === candidate?.candidateId
    ? pageMergeSelection
    : null;
  const autoPlan = useMemo(() => review ? buildStrictOverlapPlan(review) : null, [review]);
  const fallbackContainmentGroup = useMemo(
    () => review ? buildDownloadOverlapContainmentGroup(review.incoming.galleryId, [review]) ?? undefined : undefined,
    [review],
  );
  const resolvedContainmentGroup = containmentGroup ?? fallbackContainmentGroup;
  const containmentScopeKey = resolvedContainmentGroup?.keeper.entryId ?? null;
  useEffect(() => {
    if (!containmentScopeKey || containmentSelectionScope.current === containmentScopeKey) return;
    containmentSelectionScope.current = containmentScopeKey;
    setContainmentSelectionOverrides(new Map());
  }, [containmentScopeKey]);
  const groupedCurrentCandidateIds = useMemo(() => new Set(
    resolvedContainmentGroup?.items
      .filter((item) => item.review.reviewId === review?.reviewId)
      .map((item) => item.candidate.candidateId) ?? [],
  ), [resolvedContainmentGroup, review?.reviewId]);
  const ambiguousContainmentCandidates = useMemo(
    () => review?.candidates.filter((item) =>
      item.decision === undefined && !groupedCurrentCandidateIds.has(item.candidateId)) ?? [],
    [groupedCurrentCandidateIds, review],
  );
  const outcome = useMemo(
    () => review && candidate ? comparisonOutcome(review, candidate) : null,
    [candidate, review],
  );
  const reviewPending = review?.state === "pending";
  const pageMergeAvailable = Boolean(reviewPending && candidate && !candidate.decision && onMergePages);
  const pageMergeCount = activePageMergeSelection?.sourcePages.size ?? 0;
  const mergeSourceLabel = activePageMergeSelection?.sourceSide === "existing" ? "기존 A" : "신규 B";
  const mergeTargetLabel = activePageMergeSelection?.sourceSide === "existing" ? "신규 B" : "기존 A";
  const remainingAfterCandidate = pendingCandidates.filter((item) =>
    item.candidateId !== candidate?.candidateId).length;

  useEffect(() => {
    setCandidateId(review?.candidates.find((item) => item.decision === undefined)?.candidateId ?? review?.candidates[0]?.candidateId ?? null);
  }, [review?.reviewId, review?.revision]);

  useEffect(() => {
    setPageMergeSelection(null);
    setPageMergeHint(null);
  }, [candidate?.candidateId, review?.reviewId, review?.revision]);

  useEffect(() => {
    if (previousOpen.current && !open) {
      setPageMergeSelection(null);
      setPageMergeHint(null);
    }
    previousOpen.current = open;
  }, [open]);

  useEffect(() => {
    if (!open) return;
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!dialog.current?.open) {
      if (typeof dialog.current?.showModal === "function") dialog.current.showModal();
      else dialog.current?.setAttribute("open", "");
    }
    window.requestAnimationFrame(() => closeButton.current?.focus());
    return () => {
      if (dialog.current?.open) {
        if (typeof dialog.current.close === "function") dialog.current.close();
        else dialog.current.removeAttribute("open");
      }
      if (opener.current?.isConnected) opener.current.focus();
      opener.current = null;
    };
  }, [open]);

  const titleId = useId();
  const safetyId = useId();
  const decide = (action: DownloadOverlapDecisionRequest["action"], currentCandidateId?: string) => {
    if (!review || !reviewPending || decisionPending) return;
    if (completedPair && ["remove_existing_continue", "remove_incoming"].includes(action) && !window.confirm(`완료 앨범 ${action === "remove_incoming" ? "B" : "A"}를 목록에서 제외하고 파일을 제외 폴더로 이동할까요? 반대쪽 완료 앨범은 보존하며, 파일은 영구 삭제하지 않습니다.`)) return;
    if (!completedPair && action === "remove_existing_continue" && !window.confirm(`기존 앨범 A를 제거 처리할까요? 완료 앨범은 영구 삭제하지 않고 격리 영역으로 이동하며, 다른 중복 검토에 멈춘 staging이면 그 다운로드만 취소합니다.${candidate?.existingUniquePages ? ` A에만 있는 ${candidate.existingUniquePages}장도 해당 처리에 포함됩니다.` : ""} 신규 앨범 B는 남은 후보 검토 후 완료됩니다.`)) return;
    if (!completedPair && action === "remove_incoming" && !window.confirm(`신규 앨범 B 다운로드를 취소할까요?${candidate?.incomingUniquePages ? ` B에만 있는 ${candidate.incomingUniquePages}장도 완료되지 않습니다.` : ""} 기존 앨범 A와 다른 보유 파일은 변경하지 않습니다.`)) return;
    onDecision({
      reviewId: review.reviewId,
      expectedRevision: review.revision,
      action,
      ...(completedPair && candidate ? { candidateId: candidate.candidateId } : currentCandidateId ? { candidateId: currentCandidateId } : {}),
    });
  };

  const toggleMergePage = (side: MergePageSide, page: number, pair?: DownloadOverlapPagePair) => {
    if (!review || !candidate || !pageMergeAvailable || decisionPending) return;
    if (!pair) {
      setPageMergeHint("이 페이지는 반대 판본의 대응 위치가 없어 병합할 수 없습니다. 첫 버전에서는 순서를 임의로 정하지 않습니다.");
      return;
    }
    const pairedSourcePage = side === "existing" ? pair.existingSourcePage : pair.incomingSourcePage;
    if (pairedSourcePage !== page) {
      setPageMergeHint("저장된 페이지 대응 정보가 달라졌습니다. 최신 검토를 다시 불러와 주세요.");
      return;
    }
    if (activePageMergeSelection && activePageMergeSelection.sourceSide !== side) {
      setPageMergeHint(`현재 ${mergeSourceLabel}를 병합 원본으로 선택 중입니다. 방향을 바꾸려면 먼저 전체 선택을 해제하세요.`);
      return;
    }
    const sourceMapping = validateMergeSourceMapping(candidate, review.incoming, side);
    if (!sourceMapping.valid) {
      setPageMergeHint(mergeSourceMappingMessage(side, sourceMapping));
      return;
    }
    const sourceUniquePages = side === "existing"
      ? candidate.existingUniquePages
      : candidate.incomingUniquePages;
    if (sourceUniquePages > 0) {
      setPageMergeHint(`${side === "existing" ? "기존 A" : "신규 B"}에만 있는 페이지가 ${sourceUniquePages}장 있어 자동 제외를 전제로 한 병합이 불가능합니다. 반대 판본을 원본으로 선택하거나 직접 검토해 주세요.`);
      return;
    }
    const sourcePages = new Set(activePageMergeSelection?.sourcePages ?? []);
    if (sourcePages.has(page)) sourcePages.delete(page);
    else {
      if (sourcePages.size >= MAX_PAGE_MERGE_SELECTION) {
        setPageMergeHint(`한 번에 최대 ${MAX_PAGE_MERGE_SELECTION}장까지 병합할 수 있습니다. 일부를 해제한 뒤 다시 선택하세요.`);
        return;
      }
      sourcePages.add(page);
    }
    setPageMergeSelection(sourcePages.size ? {
      reviewId: review.reviewId,
      reviewRevision: review.revision,
      candidateId: candidate.candidateId,
      sourceSide: side,
      sourcePages,
    } : null);
    setPageMergeHint(null);
  };

  const clearPageMergeSelection = () => {
    setPageMergeSelection(null);
    setPageMergeHint("병합 페이지 선택을 모두 해제했습니다.");
  };

  const applyPageMerge = () => {
    if (!review || !candidate || !activePageMergeSelection || !onMergePages || decisionPending) return;
    const sourcePages = [...activePageMergeSelection.sourcePages].sort((left, right) => left - right);
    if (!sourcePages.length) return;
    onMergePages({
      reviewId: review.reviewId,
      expectedRevision: review.revision,
      candidateId: candidate.candidateId,
      sourceSide: activePageMergeSelection.sourceSide,
      sourcePages,
    });
  };

  if (!open && !review) return null;

  return (
    <dialog className="review-dialog download-overlap-dialog" ref={dialog} aria-labelledby={titleId} aria-describedby={review ? safetyId : undefined} aria-busy={loading || decisionPending || containmentLoading} onCancel={(event) => { event.preventDefault(); onClose(); }} onClose={onClose}>
      <div className="review-form">
        <header className="dialog-header">
          <div>
            <span className="eyebrow">DOWNLOAD OVERLAP REVIEW</span>
            <h2 id={titleId}>다운로드 판본 중복 검토</h2>
            {automationReviewSequence ? (
              <nav className="download-overlap-sequence" aria-label="자동 분류 순차 검토">
                <button type="button" className="text-button" aria-label="이전 자동 분류" title="확인 처리하지 않고 이전 기록으로 이동합니다." disabled={decisionPending || !automationReviewSequence.canPrevious || !onPreviousAutomationReview} onClick={onPreviousAutomationReview}>
                  <FluentIcon glyph="\uE76B" /> 이전
                </button>
                <span role="status">순차 검토 · {automationReviewSequence.position} / {automationReviewSequence.total}</span>
                <button type="button" className="text-button" aria-label="다음 자동 분류" title="확인 처리하지 않고 다음 기록으로 이동합니다." disabled={decisionPending || !automationReviewSequence.canNext || !onNextAutomationReview} onClick={onNextAutomationReview}>
                  다음 <FluentIcon glyph="\uE76C" />
                </button>
              </nav>
            ) : null}
          </div>
          <button ref={closeButton} type="button" className="icon-button small" title="닫기" aria-label="닫기" disabled={decisionPending} onClick={onClose}><FluentIcon glyph="\uE711" /></button>
        </header>

        {completedPair && onRescan && <button type="button" className="text-button" disabled={loading || decisionPending} onClick={onRescan}>두 앨범 다시 대조</button>}
        {loading && !review ? (
          <div className="review-loading" role="status"><span className="spinner" /> 판본 겹침 근거를 불러오는 중</div>
        ) : error && !review ? (
          <div className="review-loading" role="alert">
            <FluentIcon glyph="\uE7BA" />
            <strong>다운로드 검토를 불러오지 못했습니다.</strong>
            <span>{error}</span>
            <button type="button" className="text-button" onClick={onRetry}>다시 불러오기</button>
          </div>
        ) : review && candidate ? (
          <div className="review-scroll">
            {error ? <div className="inline-error review-inline-error" role="alert">{error}</div> : null}
            <div className="review-summary">
              <span className="review-signal">{completedPair ? "완료 앨범 A/B 수동 대조" : reviewPending ? "완료 전 일시 정지" : review.state === "stale" ? "보유 목록 재검사 중" : "처리된 판본 검토"}</span>
              <strong>{relationLabel[candidate.relation]} · 신뢰도 {percent(candidate.confidence)}</strong>
              <span id={safetyId}>
                {completedPair ? "A와 B 모두 이미 다운로드가 완료된 앨범입니다. 보존·오탐 판정은 재다운로드하지 않으며, 제거는 선택한 앨범만 목록에서 제외하고 파일을 제외 폴더로 이동합니다. 병합은 검증 후 제공 앨범을 제외합니다." : reviewPending
                  ? "신규 B 파일은 검증됐지만 아직 완료 manifest를 만들지 않았습니다. 제거는 영구 삭제가 아니며, 완료된 기존 A는 격리 영역으로 이동하고 검토 중 staging A와 신규 B는 취소 상태로 보존합니다."
                  : historyItem
                    ? "자동 분류 당시의 비교 근거입니다. 목록 복원은 격리된 실제 파일을 복원하지 않습니다."
                    : "이 화면은 판정 당시의 A/B 비교 근거와 선택을 읽기 전용으로 보여줍니다. 탐색·목록 제외는 활동 기록이나 설정에서 해제할 수 있지만, 격리된 실제 파일은 이 판정 기록에서 복원되지 않습니다."}
                {browserFixture ? " · 브라우저 검토 fixture" : ""}
              </span>
            </div>

            {review.state === "stale" ? (
              <div className="download-overlap-stale-note" role="status">
                <FluentIcon glyph="\uE895" />
                <div>
                  <strong>이미 제외된 상대를 정리하고 현재 보유 목록으로 다시 검사 중입니다.</strong>
                  <span>이전 비교 근거는 확인용으로만 남아 있으며, 재검사가 끝나기 전에는 이 기록에서 추가 판정을 적용하지 않습니다.</span>
                </div>
              </div>
            ) : null}

            {autoMode !== "off" && autoPlan ? (
              <div className={`download-overlap-auto-recommendation${autoMode === "strict_quarantine" ? " is-automatic" : ""}`} role="status">
                <FluentIcon glyph={autoPlan.winner === "incoming" ? "\uE73A" : "\uE74D"} />
                <div>
                  <div className="download-overlap-auto-heading">
                    <strong>{autoMode === "strict_quarantine" ? "자동 정리 예정" : "추천"}</strong>
                    <span>{autoPlan.summary}</span>
                  </div>
                  <small>{autoMode === "strict_quarantine" ? "닫으면 재검증 후 적용 · 영구 삭제 없음" : "직접 선택해야 적용됩니다."}</small>
                </div>
                <AutoPlanHelp />
              </div>
            ) : null}

            <ContainmentBatchReview
              group={resolvedContainmentGroup}
              ambiguousCandidates={ambiguousContainmentCandidates}
              loading={containmentLoading}
              disabled={decisionPending || pageMergeCount > 0}
              error={error}
              progress={batchProgress}
              previewWidth={previewWidth}
              thumbnailClient={thumbnailClient}
              selectionOverrides={containmentSelectionOverrides}
              onSelectionOverride={(key, selected) => {
                setContainmentSelectionOverrides((current) => {
                  const next = new Map(current);
                  next.set(key, selected);
                  return next;
                });
              }}
              onReviewCandidate={setCandidateId}
              onApply={onApplyContainmentBatch}
            />

            {review.candidates.length > 1 ? (
              <div className="download-overlap-candidate-tabs" role="tablist" aria-label="겹침 후보">
                {review.candidates.map((item) => {
                  const tabOutcome = candidateOutcomeLabel(review, item);
                  return (
                    <button type="button" role="tab" aria-selected={item.candidateId === candidate.candidateId} className={item.candidateId === candidate.candidateId ? "is-active" : ""} key={item.candidateId} disabled={decisionPending} onClick={() => setCandidateId(item.candidateId)}>
                      후보 {item.rank} · #{item.existing.galleryId}
                      {tabOutcome ? <small className={`is-${tabOutcome.tone}`}>{tabOutcome.label}</small> : null}
                    </button>
                  );
                })}
              </div>
            ) : null}

            <div className="download-overlap-artifacts">
              <ArtifactSummary gallery={candidate.existing} label={completedPair ? "완료 앨범 A" : "기존 앨범 A"} page={candidate.pagePairs[0]?.existingSourcePage ?? 1} presentation={outcome?.existing} thumbnailClient={thumbnailClient} />
              <ArtifactSummary gallery={review.incoming} label={completedPair ? "완료 앨범 B" : "신규 앨범 B"} page={candidate.pagePairs[0]?.incomingSourcePage ?? 1} presentation={outcome?.incoming} thumbnailClient={thumbnailClient} />
            </div>

            <dl className="download-overlap-metrics">
              <div><dt>일치 페이지</dt><dd>{candidate.matchedPages}장</dd></div>
              <div><dt>SHA-256 / 시각</dt><dd>{candidate.exactPages} / {candidate.visualPages}</dd></div>
              <div><dt>기존 A 범위</dt><dd>{percent(candidate.existingCoverage)}</dd></div>
              <div><dt>신규 B 범위</dt><dd>{percent(candidate.incomingCoverage)}</dd></div>
              <div><dt>연속 일치</dt><dd>{candidate.longestAlignedRun}장</dd></div>
              <div><dt>고유 페이지</dt><dd>기존 A {candidate.existingUniquePages} · 신규 B {candidate.incomingUniquePages}</dd></div>
            </dl>

            <PageAlignment
              candidate={candidate}
              incoming={review.incoming}
              previewWidth={previewWidth}
              mergeEnabled={pageMergeAvailable}
              mergeDisabled={decisionPending}
              mergeSelection={activePageMergeSelection}
              mergeHint={pageMergeHint}
              thumbnailClient={thumbnailClient}
              onMergeToggle={toggleMergePage}
            />
          </div>
        ) : null}

        {reviewPending ? (
          <div className="review-actions download-overlap-actions">
            <div className={`download-overlap-action-note${pageMergeCount ? " is-page-merge" : ""}`} role="note">
              <FluentIcon glyph={pageMergeCount ? "\uE8B7" : "\uE946"} />
              {pageMergeCount ? (
                <>
                  <span>
                    <strong>{mergeSourceLabel} {pageMergeCount}장 → {mergeTargetLabel} 대응 {pageMergeCount}장 교체</strong>
                    {' '}선택한 원본 페이지를 복사하며 {mergeTargetLabel}의 추가 페이지는 그대로 둡니다. 교체 전 대상 파일은 백업합니다. 병합 검증이 성공하면 {mergeSourceLabel} 앨범은 제외하되 원본 파일은 보존합니다.
                    {decisionPending ? <span role="status"> 병합 중… 대상 앨범 전체의 백업과 무결성 검증을 진행합니다. 큰 합본은 선택한 장수보다 전체 용량에 따라 오래 걸릴 수 있습니다. 완료 후 바뀐 페이지만 해시를 갱신합니다.</span> : null}
                  </span>
                  <button type="button" className="text-button download-overlap-merge-clear" disabled={decisionPending} onClick={clearPageMergeSelection}>전체 선택 해제</button>
                </>
              ) : (
                <span>
                  <strong>현재 후보에만 적용됩니다.</strong>
                  {' '}`둘 다 보존`은 기존 제외를 복구하지 않고 이 A/B를 유지로 확정합니다.
                  {!candidate?.decision
                    ? remainingAfterCandidate > 0
                      ? ` 선택 후 남은 후보 ${remainingAfterCandidate}개를 계속 검토합니다.`
                      : completedPair ? " 판정 후 두 앨범을 다시 다운로드하지 않습니다." : " 마지막 후보이면 검토를 완료하고 신규 B 다운로드를 재개합니다."
                    : " 이 후보는 이미 처리됐으므로 미처리 후보 탭을 선택해 주세요."}
                </span>
              )}
            </div>
            {pageMergeCount ? (
              <ReviewAction help={`선택한 ${mergeSourceLabel} ${pageMergeCount}장을 ${mergeTargetLabel}의 저장된 대응 페이지에 복사합니다. 대상의 추가 페이지는 유지하고 교체 전 파일은 백업합니다. 검증 성공 뒤 ${mergeSourceLabel} 앨범은 목록에서 제외하되 원본 파일은 보존합니다.`}>
                <button type="button" className="primary-button download-overlap-merge-apply" disabled={decisionPending || !onMergePages} onClick={applyPageMerge}>선택한 {pageMergeCount}장 병합</button>
              </ReviewAction>
            ) : (
              <ReviewAction help={autoMode === "strict_quarantine" ? "창을 닫으면 자동 기준을 충족한 검토가 재검증 후 처리될 수 있습니다. 자동 처리를 막으려면 설정에서 ‘추천만 표시’ 또는 ‘사용 안 함’을 선택하세요." : "아무 판정도 저장하지 않고 검토 창만 닫습니다. 다음에 같은 검토를 다시 열 수 있습니다."}><button type="button" className="text-button" onClick={onClose}>검토 미루기</button></ReviewAction>
            )}
            <ReviewAction help={completedPair ? "완료 앨범 A만 목록에서 제외하고 파일을 제외 폴더로 이동합니다. B는 완료 상태로 보존합니다." : `현재 후보의 기존 앨범 A를 제거 처리합니다. 완료본은 영구 삭제하지 않고 격리 영역으로 이동하며, 다른 중복 검토에 멈춘 staging이면 그 staging 다운로드와 자체 검토만 취소합니다. 남은 후보 검토 또는 신규 B 완료 절차는 계속됩니다.${candidate?.existingUniquePages ? ` A에만 있는 ${candidate.existingUniquePages}장도 해당 처리에 포함됩니다.` : ""}`}><button type="button" className="text-button danger-button" disabled={!candidate || decisionPending || pageMergeCount > 0 || Boolean(candidate.decision)} onClick={() => candidate && decide("remove_existing_continue", candidate.candidateId)}>{completedPair ? "A 제외" : "기존 A 제거"}</button></ReviewAction>
            <ReviewAction help={completedPair ? "완료 앨범 B만 목록에서 제외하고 파일을 제외 폴더로 이동합니다. A는 완료 상태로 보존합니다." : `신규 앨범 B 다운로드 전체를 취소합니다. 기존 A와 다른 보유 앨범은 변경하지 않습니다.${candidate?.incomingUniquePages ? ` B에만 있는 ${candidate.incomingUniquePages}장도 완료되지 않습니다.` : ""}`}><button type="button" className="text-button danger-button" disabled={decisionPending || pageMergeCount > 0} onClick={() => decide("remove_incoming")}>{completedPair ? "B 제외" : "신규 B 제거"}</button></ReviewAction>
            <ReviewAction help="현재 A/B 후보가 중복이 아니라고 기록하고 둘 다 보존합니다. 같은 판본 지문 쌍은 다음 탐지에서 제외되며, 기존 제외나 격리를 복구하지 않습니다."><button type="button" className="text-button" disabled={!candidate || decisionPending || pageMergeCount > 0 || Boolean(candidate.decision)} onClick={() => candidate && decide("false_positive_continue", candidate.candidateId)}>오탐 판정</button></ReviewAction>
            <ReviewAction help={completedPair ? "현재 A/B를 둘 다 보존으로 확정합니다. 완료 상태를 유지하며 다시 다운로드하거나 기존 제외를 복구하지 않습니다." : "현재 A/B 후보를 둘 다 보존으로 확정합니다. 기존에 제외·격리된 앨범을 복구하는 기능은 아닙니다. 남은 후보가 있으면 계속 검토하고, 마지막 후보이면 신규 B 완료 절차를 재개합니다."}><button type="button" className="primary-button" disabled={!candidate || decisionPending || pageMergeCount > 0 || Boolean(candidate.decision)} onClick={() => candidate && decide("keep_both_continue", candidate.candidateId)}>둘 다 보존</button></ReviewAction>
          </div>
        ) : !historyItem ? (
          <div className="review-actions download-overlap-readonly-actions">
            <div role="note">
              <FluentIcon glyph="\uE8A5" />
              <span>
                <strong>읽기 전용 판정 기록</strong>
                이 창에서는 판정을 바꾸거나 제외를 복구하지 않습니다. 탐색·목록 제외는 활동 기록이나 설정에서 해제할 수 있지만, 격리된 실제 파일은 이 판정 기록에서 복원되지 않습니다.
              </span>
            </div>
          </div>
        ) : null}
        {historyItem ? (
          <div className="review-actions download-overlap-history-actions" aria-label="자동 분류 기록 처리" aria-busy={automationHistoryPending}>
            <div className="download-overlap-history-status" role="status">
              <FluentIcon glyph={historyItem.acknowledgedAt ? "\uE73E" : "\uE8A5"} />
              <span>{automationHistoryPending ? "처리 중…" : historyItem.acknowledgedAt ? "확인 완료한 자동 분류 기록" : `자동 분류 · 제외 ${historyItem.removedGalleryIds.length}개`}</span>
            </div>
            {!historyItem.acknowledgedAt ? (
              <div className="download-overlap-history-commands">
                {historyItem.removedGalleryIds.length > 0 ? (
                  <ReviewAction help="이 자동 분류로 제외된 앨범의 탐색·목록 제외를 해제하고 확인 완료합니다. 격리된 실제 파일은 복원하지 않습니다.">
                    <button type="button" className="text-button" disabled={loading || decisionPending || !onRestoreAutomationExclusions} onClick={() => onRestoreAutomationExclusions?.(historyItem.reviewId, historyItem.removedGalleryIds)}>
                      <FluentIcon glyph="\uE777" /> 목록에 복원
                    </button>
                  </ReviewAction>
                ) : null}
                <ReviewAction help={automationReviewSequence ? "자동 분류 결과를 유지하고 확인 완료한 뒤 다음 미확인 기록으로 이동합니다. 모두 확인하면 창을 닫습니다. 앨범이나 파일은 변경하지 않습니다." : "자동 분류 결과를 유지하고 기록을 확인 완료한 뒤 창을 닫습니다. 앨범이나 파일은 변경하지 않습니다."}>
                  <button type="button" className="primary-button" disabled={loading || decisionPending || !onAcknowledgeAutomationHistory} onClick={() => onAcknowledgeAutomationHistory?.(historyItem.reviewId)}>
                    <FluentIcon glyph="\uE73E" /> 확인 완료
                  </button>
                </ReviewAction>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </dialog>
  );
}
