import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { communityError, type CommunityApi, type CommunitySource, type Cursor, type MyReviewPage, type OwnReview, type WorkKey, type Writer } from "./api";
import { ReviewCard } from "./ReviewCard";
import { ReviewEditor } from "./ReviewEditor";

export function MyReviewsDialog({ source, api, privacyMode, initialWork, onClose, onChanged }: {
  source: CommunitySource; api: CommunityApi; privacyMode: boolean; initialWork?: WorkKey | null;
  onClose: () => void; onChanged: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [page, setPage] = useState<MyReviewPage | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ work: WorkKey; writer?: Writer } | null>(initialWork ? { work: initialWork } : null);
  const [saving, setSaving] = useState(false);
  const sequence = useRef(0);
  // StrictMode remounts effects in development. Reuse an in-flight read rather
  // than opening the native identity vault twice and racing its refresh lock.
  const reads = useRef(new Map<string, { api: CommunityApi; request: Promise<MyReviewPage> }>());
  const load = useCallback(async (cursor: Cursor | null = null) => {
    const version = ++sequence.current;
    setLoading(true); setError(null);
    try {
      // The existing owner-only RPC pages both services. Skip foreign-only pages
      // without downloading an unbounded history or exposing them in this service.
      let after = cursor;
      let value: MyReviewPage;
      for (let attempt = 0; ; attempt++) {
        const key = JSON.stringify(after);
        let pending = reads.current.get(key);
        if (!pending || pending.api !== api) {
          const request = api.myReviews(after);
          pending = { api, request };
          reads.current.set(key, pending);
          const release = () => { if (reads.current.get(key)?.request === request) reads.current.delete(key); };
          void request.then(release, release);
        }
        value = await pending.request;
        if (version !== sequence.current) return;
        value = { ...value, items: value.items.filter(item => item.source === source) };
        if (value.items.length || !value.nextCursor || attempt >= 4) break;
        if (JSON.stringify(after) === JSON.stringify(value.nextCursor)) throw new Error("후기 목록의 다음 위치를 확인하지 못했습니다.");
        after = value.nextCursor;
      }
      if (version !== sequence.current) return;
      setPage((old) => ({ ...value, items: cursor && old ? [...old.items, ...value.items.filter((item) => !old.items.some((previous) => previous.id === item.id))] : value.items }));
    } catch (error) { if (version === sequence.current) setError(communityError(error)); }
    finally { if (version === sequence.current) setLoading(false); }
  }, [api, source]);
  useEffect(() => { void load(); return () => { sequence.current++; }; }, [load]);
  useLayoutEffect(() => {
    const node = dialog.current;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    node?.showModal(); closeButton.current?.focus();
    return () => { node?.close(); if (opener?.isConnected) opener.focus({ preventScroll: true }); };
  }, []);
  const edit = (review: OwnReview) => {
    if (!page?.profile) return;
    setNotice(null);
    setEditing({ work: { source: review.source, workId: review.workId }, writer: { profile: page.profile, mine: review } });
  };
  return <dialog ref={dialog} className="community-mine-dialog" aria-labelledby="community-mine-title" data-gallery-shortcuts-suspended
    onCancel={(event) => { event.preventDefault(); if (!saving) onClose(); }} onKeyDown={(event) => event.stopPropagation()}>
    <header className="community-mine-heading"><h2 id="community-mine-title">내 후기</h2><button ref={closeButton} type="button" className="icon-button" aria-label="내 후기 닫기" disabled={saving} onClick={onClose}>×</button></header>
    <div className="community-mine-scroll">
      {page?.profile ? <details className="community-identity"><summary>내 익명 키 <span>{page.profile.nickname}</span></summary><p>작성자 식별 ID</p><code>{page.profile.id}</code><p>비밀 인증키는 앱에 암호화해 보관합니다. 이 ID는 로그인·복구용이 아닙니다. 앱 초기화로 삭제되지 않습니다.</p></details> : null}
      {error ? <div className="community-error" role="alert">{error} <button className="text-button" disabled={loading} onClick={() => void load()}>다시 시도</button></div> : null}
      {notice ? <p className="community-notice" role="status">{notice}</p> : null}
      {editing ? <ReviewEditor key={`${editing.work.source}:${editing.work.workId}`} work={editing.work} initialWriter={editing.writer} api={api} onClose={() => setEditing(null)} onBusy={setSaving} onSaved={(message) => { setEditing(null); setNotice(message); void load(); onChanged(); }} /> : <>
        {loading ? <p className="community-load-status" role="status"><span className="spinner catalog-refresh-spinner" aria-hidden="true" /> 내 후기를 불러오는 중…</p> : null}
        {!loading && !error && !page?.items.length ? <div className="community-empty"><strong>{page?.nextCursor ? "이 범위에는 해당 서비스의 후기가 없습니다." : "아직 작성한 후기가 없습니다."}</strong><p>{page?.nextCursor ? "내 후기 더 보기로 이전 기록을 확인하세요." : page?.identityIssued ? "앨범 상세보기나 페이지 프리뷰에서 코멘트를 남겨보세요." : "아직 익명 키가 발급되지 않았습니다. 앨범에서 처음 코멘트를 작성할 때 발급됩니다."}</p></div> : null}
        <section className="community-review-grid" aria-label="내 후기 목록" aria-busy={loading}>
          {page?.items.map((review) => <ReviewCard key={review.id} review={review} privacyMode={privacyMode} hidden={review.hidden} onEdit={() => edit(review)} />)}
        </section>
        {page?.nextCursor ? <button type="button" className="text-button community-more" disabled={loading} onClick={() => void load(page.nextCursor)}>{loading ? "불러오는 중…" : "내 후기 더 보기"}</button> : null}
      </>}
    </div>
  </dialog>;
}
