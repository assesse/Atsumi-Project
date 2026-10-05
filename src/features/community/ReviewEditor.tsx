import { useEffect, useRef, useState } from "react";
import { communityError, REVIEW_COMMENT_LIMIT, reviewCommentLength, type CommunityApi, type ReviewInput, type WorkKey, type Writer } from "./api";

export const workLabel = (work: WorkKey) => `${work.source === "hitomi" ? "Hitomi" : "Danbooru"} #${work.workId}`;

export function ReviewEditor({ work, api, initialWriter, onClose, onSaved, onBusy }: {
  work: WorkKey; api: CommunityApi; initialWriter?: Writer; onClose: () => void;
  onSaved: (message: string) => void; onBusy?: (busy: boolean) => void;
}) {
  const [writer, setWriter] = useState<Writer | null>(initialWriter ?? null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const toInput = (value?: Writer): ReviewInput => ({ ...work, nickname: value?.profile.nickname ?? "", rating: value?.mine?.rating ?? 5, recommended: value?.mine?.recommended ?? false, comment: value?.mine?.comment ?? "" });
  const [input, setInput] = useState<ReviewInput>(() => toInput(initialWriter));
  const pending = useRef(false);
  const commentLength = reviewCommentLength(input.comment);
  const request = useRef<Promise<Writer> | null>(null);
  useEffect(() => {
    // The own-history path already knows its author. Opening/editing it cannot
    // issue a new identity, even if the identity file later becomes unavailable.
    if (initialWriter) return;
    let active = true;
    request.current ??= api.beginWriting(work); // Explicit legacy writing shortcut only.
    void request.current.then((value) => {
      if (active) { setWriter(value); setInput(toInput(value)); }
    }).catch((error) => { if (active) setError(communityError(error)); });
    return () => { active = false; };
  }, [api, initialWriter, work]);
  const submit = async (remove = false) => {
    if (pending.current || !writer) return;
    if (!remove && commentLength > REVIEW_COMMENT_LIMIT) { setError("코멘트는 100자 이내로 입력해 주세요."); return; }
    if (remove && !window.confirm(`${workLabel(work)}에 남긴 내 후기를 삭제할까요? 삭제한 후기는 복구할 수 없습니다.`)) return;
    pending.current = true; setBusy(true); onBusy?.(true); setError(null);
    try {
      if (remove) await api.delete(work); else await api.save(input);
      onSaved(remove ? "내 후기를 삭제했습니다. 작성자 키는 그대로 유지됩니다." : "후기를 저장했습니다.");
    } catch (error) { setError(communityError(error)); }
    finally { pending.current = false; setBusy(false); onBusy?.(false); }
  };
  return <section className="community-editor" aria-label="후기 작성">
    <header><h2>{workLabel(work)}</h2><button type="button" className="text-button" onClick={onClose} disabled={busy}>목록으로</button></header>
    {!writer && !error ? <p role="status"><span className="spinner catalog-refresh-spinner" aria-hidden="true" /> 작성자 정보를 불러오는 중…</p> : null}
    {error ? <p className="community-error" role="alert">{error}</p> : null}
    {writer ? <form onSubmit={(e) => { e.preventDefault(); void submit(); }}>
      {writer.mine?.hidden ? <p className="community-error">운영자가 숨긴 후기입니다. 수정해도 공개 상태로 바뀌지 않습니다.</p> : null}
      <label>닉네임<input aria-label="후기 닉네임" required minLength={2} maxLength={24} value={input.nickname} onChange={(e) => setInput((old) => ({ ...old, nickname: e.target.value }))} disabled={busy} /></label>
      <fieldset className="community-rating"><legend>별점</legend>{[1, 2, 3, 4, 5].map((rating) => <button key={rating} type="button" aria-label={`${rating}점`} aria-pressed={input.rating === rating} className={rating <= input.rating ? "is-filled" : ""} disabled={busy} onClick={() => setInput((old) => ({ ...old, rating }))}>★</button>)}</fieldset>
      <label className="community-recommend-input"><input type="checkbox" checked={input.recommended} disabled={busy} onChange={(e) => setInput((old) => ({ ...old, recommended: e.target.checked }))} />추천</label>
      <label>한마디<textarea aria-label="짧은 후기" placeholder="100자 이내 · 별점만 남겨도 됩니다." rows={3} disabled={busy} aria-invalid={commentLength > REVIEW_COMMENT_LIMIT || undefined} value={input.comment} onChange={(e) => setInput((old) => ({ ...old, comment: e.target.value }))} /><span className="community-charcount">{commentLength} / {REVIEW_COMMENT_LIMIT}</span></label>
      {commentLength > REVIEW_COMMENT_LIMIT ? <p className="community-error" role="alert">기존 내용은 유지했습니다. 저장하려면 100자 이내로 줄여 주세요.</p> : null}
      <p className="community-editor-note">등록한 내용은 공개됩니다. 닉네임 변경은 이전 후기에도 적용됩니다.</p>
      <div className="community-editor-actions">{writer.mine ? <button type="button" className="text-button danger-button" disabled={busy} onClick={() => void submit(true)}>내 후기 삭제</button> : null}<button className="text-button primary" aria-busy={busy} disabled={busy || commentLength > REVIEW_COMMENT_LIMIT || Array.from(input.nickname.trim()).length < 2}>{busy ? <><span className="spinner catalog-refresh-spinner" aria-hidden="true" /> 처리 중…</> : writer.mine ? "후기 수정" : "후기 등록"}</button></div>
    </form> : null}
  </section>;
}
