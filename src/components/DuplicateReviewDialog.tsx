import { useEffect, useMemo, useRef } from "react";
import type {
  DuplicateDecisionAction,
  DuplicateDecisionRequest,
  DuplicateGalleryRef,
  DuplicateReview,
} from "../api/contracts";
import type { Gallery, GalleryId } from "../core/types";
import {
  artifactPageThumbnailKey,
  galleryCoverThumbnailKey,
  type ThumbnailClient,
} from "../thumbnail";
import { FluentIcon } from "./FluentIcon";
import { GalleryThumbnail } from "./GalleryThumbnail";

type DuplicateReviewDialogProps = {
  open: boolean;
  review?: DuplicateReview;
  galleries?: ReadonlyMap<GalleryId, Gallery>;
  loading?: boolean;
  error?: string | null;
  decisionPending?: boolean;
  browserFixture?: boolean;
  thumbnailClient?: ThumbnailClient;
  onClose: () => void;
  onRetry: () => void;
  onRescan: () => void;
  onDecision: (request: DuplicateDecisionRequest) => void;
};

const relationLabel: Record<DuplicateReview["candidate"]["relation"], string> = {
  exact: "파일이 정확히 일치",
  contains: "한 작품이 다른 작품을 포함",
  partial: "일부 페이지가 일치",
  translation_visual: "번역판으로 추정되는 시각 일치",
};

const evidenceLabel: Record<DuplicateReview["evidence"][number]["kind"], string> = {
  exact_sha256: "SHA-256 정확 일치",
  visual_hash: "시각 해시",
  sequence_alignment: "페이지 순서 정렬",
  e_hentai_relation: "E-Hentai 관계 정보",
};

const decisionLabel: Record<DuplicateDecisionAction, string> = {
  hide_parent: "작품 A 숨김",
  hide_candidate: "작품 B 숨김",
  series_link: "연작 연결",
  series_unlink: "연작 연결 해제",
  exclude_pair: "작품 쌍 제외",
};

const percent = (value: number): string =>
  `${Math.round(Math.min(1, Math.max(0, Number.isFinite(value) ? value : 0)) * 100)}%`;

const containmentDecision = (review?: DuplicateReview) => {
  if (!review || review.candidate.relation !== "contains") return null;
  const { parent, candidate } = review.candidate;
  if (parent.pageCount === candidate.pageCount) return null;
  return parent.pageCount > candidate.pageCount
    ? { keep: parent, hide: candidate, hideAction: "hide_candidate" as const, keepSide: "parent" as const }
    : { keep: candidate, hide: parent, hideAction: "hide_parent" as const, keepSide: "candidate" as const };
};

const visualGallery = (
  gallery: DuplicateGalleryRef,
  galleries?: ReadonlyMap<GalleryId, Gallery>,
): Pick<Gallery, "id" | "thumbnailKey" | "coverIndex"> => {
  const current = galleries?.get(gallery.galleryId);
  return current ?? {
    id: gallery.galleryId,
    coverIndex: Math.abs(Number(gallery.galleryId)) % 6,
  };
};

function ReviewCard({
  gallery,
  label,
  visual,
  thumbnailClient,
}: {
  gallery: DuplicateGalleryRef;
  label: string;
  visual: Pick<Gallery, "id" | "thumbnailKey" | "coverIndex">;
  thumbnailClient?: ThumbnailClient;
}) {
  return (
    <section className="review-card">
      <h3>{label}</h3>
      <GalleryThumbnail
        className="review-hero"
        thumbnailKey={galleryCoverThumbnailKey(visual)}
        consumer="review"
        priority="critical"
        client={thumbnailClient}
        alt={`${gallery.title} 표지`}
      />
      <dl className="review-fields">
        <dt>제목</dt><dd>{gallery.title}</dd>
        <dt>작가</dt><dd>{gallery.artist || "정보 없음"}</dd>
        <dt>그룹</dt><dd>{gallery.group || "정보 없음"}</dd>
        <dt>페이지</dt><dd>{gallery.pageCount}p</dd>
        <dt>갤러리 ID</dt><dd>#{gallery.galleryId}</dd>
        <dt>아티팩트</dt><dd>{gallery.entryId}</dd>
      </dl>
    </section>
  );
}

export function DuplicateReviewDialog({
  open,
  review,
  galleries,
  loading = false,
  error = null,
  decisionPending = false,
  browserFixture = false,
  thumbnailClient,
  onClose,
  onRetry,
  onRescan,
  onDecision,
}: DuplicateReviewDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);

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
      const target = opener.current;
      opener.current = null;
      if (target?.isConnected) target.focus();
    };
  }, [open]);

  const parentVisual = useMemo(
    () => review ? visualGallery(review.candidate.parent, galleries) : undefined,
    [galleries, review],
  );
  const candidateVisual = useMemo(
    () => review ? visualGallery(review.candidate.candidate, galleries) : undefined,
    [galleries, review],
  );
  const containment = containmentDecision(review);

  const decide = (action: DuplicateDecisionAction, extra: Partial<DuplicateDecisionRequest> = {}) => {
    if (!review || decisionPending) return;
    onDecision({
      candidateId: review.candidate.candidateId,
      expectedRevision: review.candidate.revision,
      action,
      ...extra,
    });
  };

  if (!open && !review) return null;

  return (
    <dialog
      className="review-dialog"
      ref={dialog}
      aria-labelledby="review-dialog-title"
      aria-describedby={review ? "review-dialog-safety" : undefined}
      aria-busy={loading || decisionPending}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClose={onClose}
    >
      <div className="review-form">
        <header className="dialog-header">
          <div>
            <span className="eyebrow">DUPLICATE REVIEW</span>
            <h2 id="review-dialog-title">작품 중복 검토</h2>
          </div>
          <button ref={closeButton} type="button" className="icon-button small" title="닫기" aria-label="닫기" onClick={onClose}>
            <FluentIcon glyph="\uE711" />
          </button>
        </header>

        {loading && !review ? (
          <div className="review-loading" role="status"><span className="spinner" /> 저장된 중복 근거를 불러오는 중</div>
        ) : error && !review ? (
          <div className="review-loading" role="alert">
            <FluentIcon glyph="\uE7BA" />
            <strong>중복 검토를 불러오지 못했습니다.</strong>
            <span>{error}</span>
            <button type="button" className="text-button" onClick={onRetry}>다시 불러오기</button>
          </div>
        ) : review && parentVisual && candidateVisual ? (
          <div className="review-scroll">
            {error ? (
              <div className="inline-error review-inline-error" role="alert">
                <span>{error}</span>
                <button type="button" className="text-button" onClick={onRetry}>최신 내용 다시 불러오기</button>
              </div>
            ) : null}
            <div className="review-summary">
              <span className="review-signal">{relationLabel[review.candidate.relation]}</span>
              <strong>
                신뢰도 {percent(review.candidate.confidence)} · {review.candidate.matchedPages}개 페이지 일치
                {browserFixture ? " · 브라우저 검토 fixture" : ""}
              </strong>
              <span id="review-dialog-safety">
                부모 범위 {percent(review.candidate.parentCoverage)} · 후보 범위 {percent(review.candidate.candidateCoverage)}.
                자동으로 파일을 삭제하지 않으며 판정은 이력에 보존됩니다.
              </span>
            </div>
            {containment ? (
              <div className="containment-policy" role="note">
                <strong>{containment.keep.pageCount}p 포괄 작품을 남깁니다.</strong>
                <span>{containment.hide.pageCount}p 귀속 작품은 포괄 작품에 완전히 포함된 것으로 판정되어 숨김 대상만 선택할 수 있습니다.</span>
              </div>
            ) : null}

            <section className="review-evidence" aria-labelledby="review-evidence-title">
              <h3 id="review-evidence-title">판정 근거</h3>
              <div className="evidence-list">
                {review.evidence.map((evidence) => (
                  <article key={evidence.evidenceId} className="evidence-item">
                    <strong>{evidenceLabel[evidence.kind]}</strong>
                    <span>신뢰도 {percent(evidence.confidence)} · {evidence.matchedPages}개 페이지</span>
                    <p>{evidence.description}</p>
                  </article>
                ))}
              </div>
            </section>

            <div className="review-columns">
              <ReviewCard
                gallery={review.candidate.parent}
                label={containment ? (containment.keepSide === "parent" ? "포괄 작품" : "귀속 작품") : "작품 A"}
                visual={parentVisual}
                thumbnailClient={thumbnailClient}
              />
              <ReviewCard
                gallery={review.candidate.candidate}
                label={containment ? (containment.keepSide === "candidate" ? "포괄 작품" : "귀속 작품") : "작품 B"}
                visual={candidateVisual}
                thumbnailClient={thumbnailClient}
              />
            </div>

            <details className="match-pairs" open>
              <summary>일치 페이지 전체보기 · {review.pagePairs.length}쌍</summary>
              {review.pagePairs.length ? (
                <div className="pair-strip">
                  {review.pagePairs.map((pair, index) => (
                    <article key={`${pair.parentSourcePage}:${pair.candidateSourcePage}`} className="pair">
                      <GalleryThumbnail
                        className="pair-image"
                        thumbnailKey={artifactPageThumbnailKey(review.candidate.parent.entryId, pair.parentSourcePage, parentVisual.coverIndex + pair.parentSourcePage - 1)}
                        consumer="review"
                        priority={index < 4 ? "visible" : "prefetch"}
                        client={thumbnailClient}
                        alt={`${review.candidate.parent.title} 원본 ${pair.parentSourcePage}페이지`}
                      >
                        <span>P {pair.parentSourcePage}</span>
                      </GalleryThumbnail>
                      <GalleryThumbnail
                        className="pair-image"
                        thumbnailKey={artifactPageThumbnailKey(review.candidate.candidate.entryId, pair.candidateSourcePage, candidateVisual.coverIndex + pair.candidateSourcePage - 1)}
                        consumer="review"
                        priority={index < 4 ? "visible" : "prefetch"}
                        client={thumbnailClient}
                        alt={`${review.candidate.candidate.title} 원본 ${pair.candidateSourcePage}페이지`}
                      >
                        <span>C {pair.candidateSourcePage}</span>
                      </GalleryThumbnail>
                      <div className="pair-metrics">
                        <strong>{pair.exactSha256 ? "SHA-256 일치" : `시각 ${percent(pair.visualSimilarity)}`}</strong>
                        <span>dHash {pair.dHashDistance} · pHash {pair.pHashDistance}</span>
                        <span>detail {pair.detailHashDistance} · edge {percent(pair.edgeSimilarity)}</span>
                        {pair.lowInformation ? <em>저정보량 페이지</em> : null}
                      </div>
                    </article>
                  ))}
                </div>
              ) : <p className="review-empty">표시할 일치 페이지가 없습니다.</p>}
            </details>

            <section className="decision-history" aria-labelledby="decision-history-title">
              <h3 id="decision-history-title">판정 이력 · {review.decisions.length}건</h3>
              {review.decisions.length ? (
                <ol>
                  {[...review.decisions].reverse().map((decision) => (
                    <li key={decision.decisionId}>
                      <strong>{containment && decision.action === containment.hideAction ? "귀속 작품 숨김" : decisionLabel[decision.action]}</strong>
                      <span>revision {decision.candidateRevision} · {new Date(decision.createdAt).toLocaleString("ko-KR")}</span>
                    </li>
                  ))}
                </ol>
              ) : <p className="review-empty">아직 사용자가 적용한 판정이 없습니다.</p>}
            </section>
          </div>
        ) : null}

        <div className={`review-actions${containment ? " is-containment" : ""}`}>
          <button type="button" className="text-button scan-button" disabled={decisionPending} onClick={onRescan}>
            <FluentIcon glyph="\uE9D9" /> 전체 다시 검사
          </button>
          <span />
          {containment ? (
            <button type="button" className="text-button danger-button containment-keep-action" disabled={decisionPending} onClick={() => decide(containment.hideAction)}>
              {containment.keep.pageCount}p 포괄 작품 유지 · {containment.hide.pageCount}p 귀속 작품 숨기기
            </button>
          ) : (
            <>
              <button type="button" className="text-button danger-button" disabled={!review || decisionPending} onClick={() => decide("hide_parent")}>작품 A 숨기기</button>
              <button type="button" className="text-button danger-button" disabled={!review || decisionPending} onClick={() => decide("hide_candidate")}>작품 B 숨기기</button>
            </>
          )}
          <button type="button" className="text-button" disabled={!review || decisionPending} onClick={() => decide("exclude_pair")}>이 작품 쌍 제외</button>
        </div>
      </div>
    </dialog>
  );
}
