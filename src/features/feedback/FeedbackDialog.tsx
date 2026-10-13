import { useEffect, useRef, useState } from "react";
import type { ContentSource } from "../../app/workspaceRegistry";
import { feedbackApi, feedbackError, type FeedbackApi, type FeedbackDraft } from "./api";
import "./FeedbackDialog.css";

export function FeedbackDialog({ source, onClose, api = feedbackApi }: { source: ContentSource; onClose(): void; api?: FeedbackApi }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const done = useRef<HTMLButtonElement>(null);
  const [draft, setDraft] = useState<FeedbackDraft | null>(null);
  const [comment, setComment] = useState("");
  const [preparing, setPreparing] = useState(true);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");
  const [receipt, setReceipt] = useState("");
  const [attempt, setAttempt] = useState(0);
  const inFlight = useRef(false);
  const [submitted, setSubmitted] = useState(false);
  useEffect(() => { dialog.current?.showModal(); }, []);
  useEffect(() => { if (receipt) done.current?.focus(); }, [receipt]);
  useEffect(() => {
    let active = true;
    setPreparing(true); setError("");
    void api.prepare(source).then(value => { if (active) setDraft(value); }, e => { if (active) setError(feedbackError(e)); }).finally(() => { if (active) setPreparing(false); });
    return () => { active = false; };
  }, [api, source, attempt]);
  const send = async () => {
    if (!draft || inFlight.current || receipt) return;
    inFlight.current = true; setSubmitted(true); setSending(true); setError("");
    try { setReceipt((await api.send(draft.id, comment.trim())).receiptId); }
    catch (e) { setError(feedbackError(e)); }
    finally { inFlight.current = false; setSending(false); }
  };
  return <dialog ref={dialog} className="feedback-dialog" aria-labelledby="feedback-title" data-gallery-shortcuts-suspended="true" onCancel={event => { event.preventDefault(); if (!inFlight.current) onClose(); }}>
    <header><h2 id="feedback-title">오류·의견 보내기</h2>{!receipt ? <button type="button" className="feedback-close" aria-label="닫기" disabled={sending} onClick={onClose}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg></button> : null}</header>
    {receipt ? <div role="status"><p>개발자에게 비공개로 전달했습니다.</p><p className="feedback-receipt">접수번호 <code>{receipt}</code></p></div> : <>
      <p>어떤 문제가 있었나요? 최근 2시간의 진단 기록을 함께 보냅니다.</p>
      <label htmlFor="feedback-comment">코멘트 <span>선택</span></label>
      <textarea id="feedback-comment" rows={4} maxLength={1000} value={comment} disabled={submitted} placeholder="어떤 화면에서 무엇을 했는지 알려 주세요. 개인정보는 적지 마세요." onChange={event => setComment(event.target.value)} />
      <p className="feedback-privacy">이미지·채팅·검색어·작품번호·파일 경로·로그인 정보는 진단 기록에 포함하지 않습니다. 보내기를 누를 때만 코멘트와 기록이 전송됩니다.</p>
      {preparing ? <p role="status" className="feedback-loading">진단 기록 준비 중…</p> : draft ? <details><summary>전송 내용 확인 · {draft.eventCount.toLocaleString()}건 · {Math.ceil(draft.bytes / 1000)} KB</summary><pre>{JSON.stringify(draft.payload, null, 2)}</pre></details> : null}
      <p className="feedback-privacy">진단 전송용 익명 식별자를 사용합니다. 공개 커뮤니티에는 올라가지 않습니다.</p>
      {error ? <p role="alert" className="feedback-error">{error}</p> : null}
      {error && submitted ? <p className="feedback-privacy">같은 내용으로 다시 보내면 중복 접수되지 않습니다. 내용을 바꾸려면 닫고 다시 열어 주세요.</p> : null}
    </>}
    <footer>{receipt ? <button ref={done} type="button" onClick={onClose}>완료</button> : <>
      {!preparing && !draft ? <button type="button" onClick={() => setAttempt(value => value + 1)}>다시 준비</button> : null}
      <button type="button" disabled={!draft || preparing || sending} aria-busy={sending} onClick={() => void send()}>{sending ? "보내는 중…" : "보내기"}</button>
    </>}</footer>
  </dialog>;
}
