import {
  cloneElement,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type CSSProperties,
  type ReactElement,
} from "react";
import type {
  DownloadOverlapCandidate,
  DownloadOverlapDecisionRequest,
  DownloadOverlapGalleryRef,
  DownloadOverlapPagePair,
  DownloadOverlapReview,
} from "../api/contracts";
import {
  buildDownloadOverlapAlignment,
  formatPageRanges,
  uniquePagesForSide,
} from "../downloadOverlap/alignment";
import { artifactPageThumbnailKey, type ThumbnailClient } from "../thumbnail";
import { FluentIcon } from "./FluentIcon";
import { GalleryThumbnail } from "./GalleryThumbnail";

type Props = {
  open: boolean;
  review?: DownloadOverlapReview;
  loading?: boolean;
  error?: string | null;
  decisionPending?: boolean;
  browserFixture?: boolean;
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

function ArtifactSummary({ gallery, label, page, thumbnailClient }: {
  gallery: DownloadOverlapGalleryRef;
  label: string;
  page: number;
  thumbnailClient?: ThumbnailClient;
}) {
  return (
    <article className="download-overlap-artifact">
      <GalleryThumbnail
        className="download-overlap-cover"
        thumbnailKey={artifactPageThumbnailKey(gallery.entryId, page, Number(gallery.galleryId) % 6)}
        consumer="review"
        priority="critical"
        client={thumbnailClient}
        alt={`${gallery.title} ${page}페이지`}
      />
      <div>
        <strong className="download-overlap-edition-label">{label}</strong>
        <strong className="download-overlap-artifact-title">{gallery.title}</strong>
        <span>{gallery.artists.join(", ") || "작가 정보 없음"}</span>
        <span>#{gallery.galleryId} · {gallery.pageCount}p</span>
      </div>
    </article>
  );
}

function PageCell({ entryId, page, side, pair, index, thumbnailClient }: {
  entryId: string;
  page?: number;
  side: "existing" | "incoming";
  pair?: DownloadOverlapPagePair;
  index: number;
  thumbnailClient?: ThumbnailClient;
}) {
  if (!page) {
    return <div className="download-overlap-page-cell is-gap" aria-label="이 판본에는 대응 페이지 없음"><span>—</span></div>;
  }
  const matched = Boolean(pair);
  const matchLabel = pair
    ? pair.exactSha256 ? "SHA-256 일치" : `시각 ${percent(pair.visualSimilarity)}`
    : "이 판본에만 있음";
  const sideLabel = side === "existing" ? "기존 A" : "신규 B";
  return (
    <GalleryThumbnail
      className={`download-overlap-page-cell ${matched ? "is-matched" : "is-unique"}`}
      thumbnailKey={artifactPageThumbnailKey(entryId, page, index)}
      consumer="review"
      priority={index < 6 ? "visible" : "prefetch"}
      client={thumbnailClient}
      alt={`${sideLabel} ${page}페이지`}
      title={`${sideLabel} ${page}p · ${matchLabel}`}
    >
      <span className="download-overlap-page-number">{sideLabel} {page}p</span>
      <span className="download-overlap-page-status">{matched ? "일치" : "추가"}</span>
    </GalleryThumbnail>
  );
}

function PageAlignment({ candidate, incoming, thumbnailClient }: {
  candidate: DownloadOverlapCandidate;
  incoming: DownloadOverlapGalleryRef;
  thumbnailClient?: ThumbnailClient;
}) {
  const columns = useMemo(
    () => buildDownloadOverlapAlignment(candidate, incoming.pageCount),
    [candidate, incoming.pageCount],
  );
  const existingUnique = uniquePagesForSide(columns, "existing");
  const incomingUnique = uniquePagesForSide(columns, "incoming");
  const gridStyle = { "--overlap-page-columns": columns.length } as CSSProperties;

  return (
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
            <PageCell key={`existing:${column.key}`} entryId={candidate.existing.entryId} page={column.existingPage} side="existing" pair={column.pair} index={index} thumbnailClient={thumbnailClient} />
          ))}
          <strong className="download-overlap-row-label">신규 B</strong>
          {columns.map((column, index) => (
            <PageCell key={`incoming:${column.key}`} entryId={incoming.entryId} page={column.incomingPage} side="incoming" pair={column.pair} index={index} thumbnailClient={thumbnailClient} />
          ))}
        </div>
      </div>
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

export function DownloadOverlapReviewDialog({ open, review, loading = false, error = null, decisionPending = false, browserFixture = false, thumbnailClient, onClose, onRetry, onDecision }: Props) {
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
    if (!review || decisionPending) return;
    if (action === "remove_existing_continue" && !window.confirm(`기존 앨범 A를 제거 처리할까요? 완료 앨범은 복구 가능한 격리로 이동하고, 다른 중복 검토에 멈춘 staging이면 그 다운로드만 취소합니다.${candidate?.existingUniquePages ? ` A에만 있는 ${candidate.existingUniquePages}장도 해당 처리에 포함됩니다.` : ""} 신규 앨범 B는 남은 후보 검토 후 완료됩니다.`)) return;
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
              <span className="review-signal">완료 전 일시 정지</span>
              <strong>{relationLabel[candidate.relation]} · 신뢰도 {percent(candidate.confidence)}</strong>
              <span id="download-overlap-safety">
                신규 B 파일은 검증됐지만 아직 완료 manifest를 만들지 않았습니다. 제거는 영구 삭제가 아니며, 완료된 기존 A는 복구 가능한 격리로 이동하고 검토 중 staging A와 신규 B는 취소 상태로 보존합니다.
                {browserFixture ? " · 브라우저 검토 fixture" : ""}
              </span>
            </div>

            {review.candidates.length > 1 ? (
              <div className="download-overlap-candidate-tabs" role="tablist" aria-label="겹침 후보">
                {review.candidates.map((item) => (
                  <button type="button" role="tab" aria-selected={item.candidateId === candidate.candidateId} className={item.candidateId === candidate.candidateId ? "is-active" : ""} key={item.candidateId} onClick={() => setCandidateId(item.candidateId)}>
                    후보 {item.rank} · #{item.existing.galleryId}
                    {item.decision ? <small>처리됨</small> : null}
                  </button>
                ))}
              </div>
            ) : null}

            <div className="download-overlap-artifacts">
              <ArtifactSummary gallery={candidate.existing} label="기존 앨범 A" page={candidate.pagePairs[0]?.existingSourcePage ?? 1} thumbnailClient={thumbnailClient} />
              <ArtifactSummary gallery={review.incoming} label="신규 앨범 B" page={candidate.pagePairs[0]?.incomingSourcePage ?? 1} thumbnailClient={thumbnailClient} />
            </div>

            <dl className="download-overlap-metrics">
              <div><dt>일치 페이지</dt><dd>{candidate.matchedPages}장</dd></div>
              <div><dt>SHA-256 / 시각</dt><dd>{candidate.exactPages} / {candidate.visualPages}</dd></div>
              <div><dt>기존 A 범위</dt><dd>{percent(candidate.existingCoverage)}</dd></div>
              <div><dt>신규 B 범위</dt><dd>{percent(candidate.incomingCoverage)}</dd></div>
              <div><dt>연속 일치</dt><dd>{candidate.longestAlignedRun}장</dd></div>
              <div><dt>고유 페이지</dt><dd>기존 A {candidate.existingUniquePages} · 신규 B {candidate.incomingUniquePages}</dd></div>
            </dl>

            <PageAlignment candidate={candidate} incoming={review.incoming} thumbnailClient={thumbnailClient} />
          </div>
        ) : null}

        <div className="review-actions download-overlap-actions">
          <ReviewAction help="아무 판정도 저장하지 않고 검토 창만 닫습니다. 다음에 같은 검토를 다시 열 수 있습니다."><button type="button" className="text-button" onClick={onClose}>검토 미루기</button></ReviewAction>
          <ReviewAction help={`현재 후보의 기존 앨범 A를 제거 처리합니다. 완료본은 복구 가능한 격리로 이동하고, 다른 중복 검토에 멈춘 staging이면 그 staging 다운로드와 자체 검토만 취소합니다. 남은 후보 검토 또는 신규 B 완료 절차는 계속됩니다.${candidate?.existingUniquePages ? ` A에만 있는 ${candidate.existingUniquePages}장도 해당 처리에 포함됩니다.` : ""}`}><button type="button" className="text-button danger-button" disabled={!review || !candidate || decisionPending || Boolean(candidate.decision)} onClick={() => candidate && decide("remove_existing_continue", candidate.candidateId)}>기존 A 제거</button></ReviewAction>
          <ReviewAction help={`신규 앨범 B 다운로드 전체를 취소합니다. 기존 A와 다른 보유 앨범은 변경하지 않습니다.${candidate?.incomingUniquePages ? ` B에만 있는 ${candidate.incomingUniquePages}장도 완료되지 않습니다.` : ""}`}><button type="button" className="text-button danger-button" disabled={!review || decisionPending} onClick={() => decide("remove_incoming")}>신규 B 제거</button></ReviewAction>
          <ReviewAction help="현재 A/B 후보가 중복이 아니라고 기록합니다. 같은 판본 지문 쌍은 다음 탐지에서 제외됩니다."><button type="button" className="text-button" disabled={!review || !candidate || decisionPending || Boolean(candidate.decision)} onClick={() => candidate && decide("false_positive_continue", candidate.candidateId)}>오탐 판정</button></ReviewAction>
          <ReviewAction help="현재 A/B 후보를 둘 다 보관해도 문제없다고 기록하고, 남은 후보 검토 또는 신규 B 완료 절차를 계속합니다."><button type="button" className="primary-button" disabled={!review || !candidate || decisionPending || Boolean(candidate.decision)} onClick={() => candidate && decide("keep_both_continue", candidate.candidateId)}>문제 없음</button></ReviewAction>
        </div>
      </div>
    </dialog>
  );
}
