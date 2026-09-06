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
  DownloadOverlapCandidate,
  DownloadOverlapDecisionAudit,
  DownloadOverlapDecisionRequest,
  DownloadOverlapGalleryRef,
  DownloadOverlapPagePair,
  DownloadOverlapReview,
  SettingsSnapshot,
} from "../api/contracts";
import {
  buildDownloadOverlapAlignment,
  formatPageRanges,
  uniquePagesForSide,
} from "../downloadOverlap/alignment";
import { galleryPreviewPreset } from "../layout/galleryPreviewPresets";
import { buildStrictOverlapPlan } from "../state/downloadOverlapAuto";
import { artifactPageThumbnailKey, type ThumbnailClient, type ThumbnailKey } from "../thumbnail";
import { FluentIcon } from "./FluentIcon";
import { GalleryThumbnail } from "./GalleryThumbnail";

type Props = {
  open: boolean;
  review?: DownloadOverlapReview;
  loading?: boolean;
  error?: string | null;
  decisionPending?: boolean;
  browserFixture?: boolean;
  autoMode?: SettingsSnapshot["downloadOverlapAutoMode"];
  previewWidth: number;
  thumbnailClient?: ThumbnailClient;
  onClose: () => void;
  onRetry: () => void;
  onDecision: (request: DownloadOverlapDecisionRequest) => void;
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

function PageCell({ entryId, page, side, pair, index, thumbnailClient, onPreviewOpen, onPreviewClose }: {
  entryId: string;
  page?: number;
  side: "existing" | "incoming";
  pair?: DownloadOverlapPagePair;
  index: number;
  thumbnailClient?: ThumbnailClient;
  onPreviewOpen: (preview: PageHoverPreview) => void;
  onPreviewClose: () => void;
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
  return (
    <GalleryThumbnail
      className={`download-overlap-page-cell ${matched ? "is-matched" : "is-unique"}`}
      thumbnailKey={thumbnailKey}
      consumer="review"
      priority={index < 6 ? "visible" : "prefetch"}
      client={thumbnailClient}
      alt={`${sideLabel} ${page}페이지`}
      aria-label={`${sideLabel} ${page}페이지 · ${matchLabel}`}
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
      <span className="download-overlap-page-number">{sideLabel} {page}p</span>
      <span className="download-overlap-page-status">{matched ? "일치" : "추가"}</span>
    </GalleryThumbnail>
  );
}

function PageAlignment({ candidate, incoming, previewWidth, thumbnailClient }: {
  candidate: DownloadOverlapCandidate;
  incoming: DownloadOverlapGalleryRef;
  previewWidth: number;
  thumbnailClient?: ThumbnailClient;
}) {
  const [hoverPreview, setHoverPreview] = useState<PageHoverPreview | null>(null);
  const columns = useMemo(
    () => buildDownloadOverlapAlignment(candidate, incoming.pageCount),
    [candidate, incoming.pageCount],
  );
  const existingUnique = uniquePagesForSide(columns, "existing");
  const incomingUnique = uniquePagesForSide(columns, "incoming");
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
        <div className="download-overlap-alignment-scroll" data-thumbnail-scroll-root tabIndex={0} aria-label="기존 A와 신규 B 페이지 정렬표">
          <div className="download-overlap-alignment-grid" style={gridStyle}>
            <strong className="download-overlap-row-label">기존 A</strong>
            {columns.map((column, index) => (
              <PageCell key={`existing:${column.key}`} entryId={candidate.existing.entryId} page={column.existingPage} side="existing" pair={column.pair} index={index} thumbnailClient={thumbnailClient} onPreviewOpen={setHoverPreview} onPreviewClose={() => setHoverPreview(null)} />
            ))}
            <strong className="download-overlap-row-label">신규 B</strong>
            {columns.map((column, index) => (
              <PageCell key={`incoming:${column.key}`} entryId={incoming.entryId} page={column.incomingPage} side="incoming" pair={column.pair} index={index} thumbnailClient={thumbnailClient} onPreviewOpen={setHoverPreview} onPreviewClose={() => setHoverPreview(null)} />
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

export function DownloadOverlapReviewDialog({ open, review, loading = false, error = null, decisionPending = false, browserFixture = false, autoMode = "off", previewWidth, thumbnailClient, onClose, onRetry, onDecision }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const [candidateId, setCandidateId] = useState<string | null>(null);

  const pendingCandidates = useMemo(
    () => review?.candidates.filter((candidate) => candidate.decision === undefined) ?? [],
    [review],
  );
  const candidate = review?.candidates.find((item) => item.candidateId === candidateId)
    ?? pendingCandidates[0]
    ?? review?.candidates[0];
  const autoPlan = useMemo(() => review ? buildStrictOverlapPlan(review) : null, [review]);
  const outcome = useMemo(
    () => review && candidate ? comparisonOutcome(review, candidate) : null,
    [candidate, review],
  );
  const reviewPending = review?.state === "pending";
  const remainingAfterCandidate = pendingCandidates.filter((item) =>
    item.candidateId !== candidate?.candidateId).length;

  useEffect(() => {
    setCandidateId(review?.candidates.find((item) => item.decision === undefined)?.candidateId ?? review?.candidates[0]?.candidateId ?? null);
  }, [review?.reviewId, review?.revision]);

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

  const decide = (action: DownloadOverlapDecisionRequest["action"], currentCandidateId?: string) => {
    if (!review || !reviewPending || decisionPending) return;
    if (action === "remove_existing_continue" && !window.confirm(`기존 앨범 A를 제거 처리할까요? 완료 앨범은 영구 삭제하지 않고 격리 영역으로 이동하며, 다른 중복 검토에 멈춘 staging이면 그 다운로드만 취소합니다.${candidate?.existingUniquePages ? ` A에만 있는 ${candidate.existingUniquePages}장도 해당 처리에 포함됩니다.` : ""} 신규 앨범 B는 남은 후보 검토 후 완료됩니다.`)) return;
    if (action === "remove_incoming" && !window.confirm(`신규 앨범 B 다운로드를 취소할까요?${candidate?.incomingUniquePages ? ` B에만 있는 ${candidate.incomingUniquePages}장도 완료되지 않습니다.` : ""} 기존 앨범 A와 다른 보유 파일은 변경하지 않습니다.`)) return;
    onDecision({
      reviewId: review.reviewId,
      expectedRevision: review.revision,
      action,
      ...(currentCandidateId ? { candidateId: currentCandidateId } : {}),
    });
  };

  if (!open && !review) return null;

  return (
    <dialog className="review-dialog download-overlap-dialog" ref={dialog} aria-labelledby="download-overlap-title" aria-describedby={review ? "download-overlap-safety" : undefined} aria-busy={loading || decisionPending} onCancel={(event) => { event.preventDefault(); onClose(); }} onClose={onClose}>
      <div className="review-form">
        <header className="dialog-header">
          <div>
            <span className="eyebrow">DOWNLOAD OVERLAP REVIEW</span>
            <h2 id="download-overlap-title">다운로드 판본 중복 검토</h2>
          </div>
          <button ref={closeButton} type="button" className="icon-button small" title="닫기" aria-label="닫기" onClick={onClose}><FluentIcon glyph="\uE711" /></button>
        </header>

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
              <span className="review-signal">{reviewPending ? "완료 전 일시 정지" : "처리된 판본 검토"}</span>
              <strong>{relationLabel[candidate.relation]} · 신뢰도 {percent(candidate.confidence)}</strong>
              <span id="download-overlap-safety">
                {reviewPending
                  ? "신규 B 파일은 검증됐지만 아직 완료 manifest를 만들지 않았습니다. 제거는 영구 삭제가 아니며, 완료된 기존 A는 격리 영역으로 이동하고 검토 중 staging A와 신규 B는 취소 상태로 보존합니다."
                  : "이 화면은 판정 당시의 A/B 비교 근거와 선택을 읽기 전용으로 보여줍니다. 탐색·목록 제외는 활동 기록이나 설정에서 해제할 수 있지만, 격리된 실제 파일은 이 판정 기록에서 복원되지 않습니다."}
                {browserFixture ? " · 브라우저 검토 fixture" : ""}
              </span>
            </div>

            {autoMode !== "off" && autoPlan ? (
              <div className={`download-overlap-auto-recommendation${autoMode === "strict_quarantine" ? " is-automatic" : ""}`} role="status">
                <FluentIcon glyph={autoPlan.winner === "incoming" ? "\uE73A" : "\uE74D"} />
                <div>
                  <strong>{autoMode === "strict_quarantine" ? "안전 기준 자동 정리 대상" : "안전 기준 추천"}</strong>
                  <span>{autoPlan.summary}</span>
                  <small>
                    일반 판본은 포함률 95% 이상·페이지 차이 5장 이하에서 판단하며, 무검열 표식이 확인되면 그 판본을 우선합니다. 작은 판본에 고유 페이지가 없고 98% 이상 포함되며 큰 판본이 1.5배·8장 이상 큰 명확한 합본이면 작은 판본의 무검열 표식보다 합본을 우선합니다. 그 밖의 근거 부족은 직접 검토합니다.
                    {autoMode === "strict_quarantine" ? " 이 창을 닫으면 기존 재검증 후 영구 삭제 대신 격리 영역으로 이동합니다." : " 최종 선택은 직접 적용해 주세요."}
                  </small>
                </div>
              </div>
            ) : null}

            {review.candidates.length > 1 ? (
              <div className="download-overlap-candidate-tabs" role="tablist" aria-label="겹침 후보">
                {review.candidates.map((item) => {
                  const tabOutcome = candidateOutcomeLabel(review, item);
                  return (
                    <button type="button" role="tab" aria-selected={item.candidateId === candidate.candidateId} className={item.candidateId === candidate.candidateId ? "is-active" : ""} key={item.candidateId} onClick={() => setCandidateId(item.candidateId)}>
                      후보 {item.rank} · #{item.existing.galleryId}
                      {tabOutcome ? <small className={`is-${tabOutcome.tone}`}>{tabOutcome.label}</small> : null}
                    </button>
                  );
                })}
              </div>
            ) : null}

            <div className="download-overlap-artifacts">
              <ArtifactSummary gallery={candidate.existing} label="기존 앨범 A" page={candidate.pagePairs[0]?.existingSourcePage ?? 1} presentation={outcome?.existing} thumbnailClient={thumbnailClient} />
              <ArtifactSummary gallery={review.incoming} label="신규 앨범 B" page={candidate.pagePairs[0]?.incomingSourcePage ?? 1} presentation={outcome?.incoming} thumbnailClient={thumbnailClient} />
            </div>

            <dl className="download-overlap-metrics">
              <div><dt>일치 페이지</dt><dd>{candidate.matchedPages}장</dd></div>
              <div><dt>SHA-256 / 시각</dt><dd>{candidate.exactPages} / {candidate.visualPages}</dd></div>
              <div><dt>기존 A 범위</dt><dd>{percent(candidate.existingCoverage)}</dd></div>
              <div><dt>신규 B 범위</dt><dd>{percent(candidate.incomingCoverage)}</dd></div>
              <div><dt>연속 일치</dt><dd>{candidate.longestAlignedRun}장</dd></div>
              <div><dt>고유 페이지</dt><dd>기존 A {candidate.existingUniquePages} · 신규 B {candidate.incomingUniquePages}</dd></div>
            </dl>

            <PageAlignment candidate={candidate} incoming={review.incoming} previewWidth={previewWidth} thumbnailClient={thumbnailClient} />
          </div>
        ) : null}

        {reviewPending ? (
          <div className="review-actions download-overlap-actions">
            <div className="download-overlap-action-note" role="note">
              <FluentIcon glyph="\uE946" />
              <span>
                <strong>현재 후보에만 적용됩니다.</strong>
                {' '}`둘 다 보존`은 기존 제외를 복구하지 않고 이 A/B를 유지로 확정합니다.
                {!candidate?.decision
                  ? remainingAfterCandidate > 0
                    ? ` 선택 후 남은 후보 ${remainingAfterCandidate}개를 계속 검토합니다.`
                    : " 마지막 후보이면 검토를 완료하고 신규 B 다운로드를 재개합니다."
                  : " 이 후보는 이미 처리됐으므로 미처리 후보 탭을 선택해 주세요."}
              </span>
            </div>
            <ReviewAction help="아무 판정도 저장하지 않고 검토 창만 닫습니다. 다음에 같은 검토를 다시 열 수 있습니다."><button type="button" className="text-button" onClick={onClose}>검토 미루기</button></ReviewAction>
            <ReviewAction help={`현재 후보의 기존 앨범 A를 제거 처리합니다. 완료본은 영구 삭제하지 않고 격리 영역으로 이동하며, 다른 중복 검토에 멈춘 staging이면 그 staging 다운로드와 자체 검토만 취소합니다. 남은 후보 검토 또는 신규 B 완료 절차는 계속됩니다.${candidate?.existingUniquePages ? ` A에만 있는 ${candidate.existingUniquePages}장도 해당 처리에 포함됩니다.` : ""}`}><button type="button" className="text-button danger-button" disabled={!candidate || decisionPending || Boolean(candidate.decision)} onClick={() => candidate && decide("remove_existing_continue", candidate.candidateId)}>기존 A 제거</button></ReviewAction>
            <ReviewAction help={`신규 앨범 B 다운로드 전체를 취소합니다. 기존 A와 다른 보유 앨범은 변경하지 않습니다.${candidate?.incomingUniquePages ? ` B에만 있는 ${candidate.incomingUniquePages}장도 완료되지 않습니다.` : ""}`}><button type="button" className="text-button danger-button" disabled={decisionPending} onClick={() => decide("remove_incoming")}>신규 B 제거</button></ReviewAction>
            <ReviewAction help="현재 A/B 후보가 중복이 아니라고 기록하고 둘 다 보존합니다. 같은 판본 지문 쌍은 다음 탐지에서 제외되며, 기존 제외나 격리를 복구하지 않습니다."><button type="button" className="text-button" disabled={!candidate || decisionPending || Boolean(candidate.decision)} onClick={() => candidate && decide("false_positive_continue", candidate.candidateId)}>오탐 판정</button></ReviewAction>
            <ReviewAction help="현재 A/B 후보를 둘 다 보존으로 확정합니다. 기존에 제외·격리된 앨범을 복구하는 기능은 아닙니다. 남은 후보가 있으면 계속 검토하고, 마지막 후보이면 신규 B 완료 절차를 재개합니다."><button type="button" className="primary-button" disabled={!candidate || decisionPending || Boolean(candidate.decision)} onClick={() => candidate && decide("keep_both_continue", candidate.candidateId)}>둘 다 보존</button></ReviewAction>
          </div>
        ) : (
          <div className="review-actions download-overlap-readonly-actions">
            <div role="note">
              <FluentIcon glyph="\uE8A5" />
              <span>
                <strong>읽기 전용 판정 기록</strong>
                이 창에서는 판정을 바꾸거나 제외를 복구하지 않습니다. 탐색·목록 제외는 활동 기록이나 설정에서 해제할 수 있지만, 격리된 실제 파일은 이 판정 기록에서 복원되지 않습니다.
              </span>
            </div>
            <button type="button" className="primary-button" onClick={onClose}>닫기</button>
          </div>
        )}
      </div>
    </dialog>
  );
}
