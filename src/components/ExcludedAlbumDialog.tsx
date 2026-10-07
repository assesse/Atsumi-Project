import { useEffect, useId, useRef, useState } from "react";
import type { ExplorationExclusionContext } from "../api/contracts";
import { animateWindowClose } from "./WindowMotion";
import "./ExcludedAlbumDialog.css";

export function ExcludedAlbumDialog({ context, title, onClose, onRestore, onReview, onOpenRetained }: {
  context: ExplorationExclusionContext; title: string; onClose(): void;
  onRestore(): Promise<boolean>; onReview(): void; onOpenRetained(): void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cancelAnimation = useRef<(() => void) | undefined>(undefined);
  const [pending, setPending] = useState(false);
  const heading = useId();
  useEffect(() => { dialog.current?.showModal(); return () => cancelAnimation.current?.(); }, []);
  const close = () => {
    if (pending) return;
    cancelAnimation.current?.();
    cancelAnimation.current = animateWindowClose(dialog.current, onClose);
  };
  const restore = async () => {
    if (pending) return;
    setPending(true);
    try { if (await onRestore()) onClose(); } finally { setPending(false); }
  };
  const canReview = Boolean(context.reviewId || context.legacyCandidateId);
  return <dialog ref={dialog} className="excluded-album-dialog" aria-labelledby={heading} data-gallery-shortcuts-suspended
    onCancel={(event) => { event.preventDefault(); close(); }}>
    <header><h2 id={heading}>제외 근거</h2><button type="button" className="icon-button small" aria-label="제외 근거 닫기" disabled={pending} onClick={close}>×</button></header>
    <strong className="excluded-album-title">{title}</strong><small>#{context.galleryId}</small>
    <ul>{context.reasons.length ? context.reasons.map((reason, index) => <li key={`${reason.kind}:${index}`}>
      <span>{reason.detail}</span><small>{Number.isFinite(Date.parse(reason.excludedAt)) ? new Date(reason.excludedAt).toLocaleString("ko-KR") : reason.excludedAt}</small>
    </li>) : <li>{context.quarantined ? "격리된 앨범입니다." : "현재 확인되는 제외 기록이 없습니다."}</li>}</ul>
    {context.retainedGallery ? <button type="button" className="excluded-retained" disabled={pending} onClick={onOpenRetained}>
      <small>보존된 판본 · #{context.retainedGallery.galleryId}</small><strong>{context.retainedGallery.title}</strong><span>상세 열기 →</span>
    </button> : <p>연결된 보존 판본을 확인할 수 없습니다.</p>}
    <p>복원하면 목록에서 다시 볼 수 있습니다. 다운로드를 새로 시작하지는 않습니다.</p>
    <footer>{canReview ? <button type="button" className="mini-command" disabled={pending} onClick={onReview}>판본 비교 보기</button> : null}
      <button type="button" className="mini-command" disabled={pending || (!context.quarantined && !context.reasons.length)} onClick={() => void restore()}>{pending ? "복원 중…" : "제외 해제·복원"}</button></footer>
  </dialog>;
}
