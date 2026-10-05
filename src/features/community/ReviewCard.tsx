import { useState, type FormEvent } from "react";
import { communityError, type Review } from "./api";
import { CommunityWorkPreview } from "./CommunityWorkPreview";
import { workLabel } from "./ReviewEditor";

export function ReviewCard({ review, privacyMode, hidden, onEdit, onReport }: {
  review: Review; privacyMode: boolean; hidden?: boolean; onEdit?: () => void;
  onReport?: (reason: string) => Promise<void>;
}) {
  const [reporting, setReporting] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const report = async (event: FormEvent) => {
    event.preventDefault(); if (busy || !onReport) return;
    setBusy(true); setError(null);
    try { await onReport(reason.trim()); setReporting(false); setReason(""); }
    catch (error) { setError(communityError(error)); }
    finally { setBusy(false); }
  };
  return <article className="community-review" tabIndex={0} aria-label={`${workLabel(review)} 후기`} data-reporting={reporting || undefined}>
    <div className="community-review-cover">
      <CommunityWorkPreview work={review} privacyMode={privacyMode} />
    </div>
    <div className="community-review-details">
      <strong className="community-work-label">{workLabel(review)}</strong>
      {review.recommended ? <span className="community-recommended">추천</span> : null}
      {review.workSummary ? <span className="community-work-summary">작품 평균 {review.workSummary.averageRating?.toFixed(1) ?? "–"} · 후기 {review.workSummary.reviewCount.toLocaleString("ko-KR")}개</span> : null}
      <footer><div className="community-review-author"><span title={review.nickname}>{review.nickname}</span><time dateTime={review.createdAt}>{new Date(review.createdAt).toLocaleDateString("ko-KR")}</time></div>
        {onEdit ? <button type="button" className="text-button" onClick={onEdit}>수정</button> : null}
        {onReport ? <button type="button" className="community-subtle" aria-expanded={reporting} disabled={busy} onClick={() => setReporting((value) => !value)}>{reporting ? "닫기" : "신고"}</button> : null}
      </footer>
      {reporting ? <form className="community-report" onSubmit={(e) => void report(e)}><input aria-label="신고 사유" placeholder="신고 사유 (발급된 작성자 키 필요)" maxLength={300} required disabled={busy} value={reason} onChange={(e) => setReason(e.target.value)} /><button className="text-button" aria-busy={busy} disabled={busy || !reason.trim()}>{busy ? "접수 중…" : "신고 접수"}</button></form> : null}
      {error ? <p role="alert" className="community-error">{error}</p> : null}
    </div>
    <div className="community-review-body">
      <div className="community-review-top"><span className="community-stars" aria-label={`별점 ${review.rating}점`}>{"★".repeat(review.rating)}<span>{"☆".repeat(5 - review.rating)}</span></span>{hidden ? <span className="community-hidden">비공개 처리됨</span> : null}</div>
      <p title={review.comment || undefined} aria-label="후기 내용" className={`community-comment${review.comment ? "" : " is-empty"}`}>{review.comment || "별점을 남겼습니다."}</p>
    </div>
  </article>;
}
