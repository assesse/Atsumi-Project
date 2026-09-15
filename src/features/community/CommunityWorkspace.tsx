import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import type { ContentSource, WorkspaceViewId } from "../../app/workspaceRegistry";
import { workspaceRegistry } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { DropdownSelect } from "../../components/DropdownSelect";
import { communityApi, communityError, validWorkId, type CommunityApi, type CommunitySource, type Cursor, type Review, type ReviewPage, type WorkKey, type Writer, type ReviewInput } from "./api";
import "./CommunityWorkspace.css";

const sourceOptions = [{ value: "all", label: "전체" }, { value: "hitomi", label: "Hitomi" }, { value: "danbooru", label: "Danbooru" }] as const;
const sourceLabel = (source: CommunitySource) => source === "hitomi" ? "Hitomi" : "Danbooru";
const workLabel = (work: WorkKey) => `${sourceLabel(work.source)} #${work.workId}`;
const dateLabel = (value: string) => new Date(value).toLocaleDateString("ko-KR");

type Props = {
  source: ContentSource; collapsed: boolean; autoFindCount?: number; attentionCount: number;
  onToggleRail: () => void; onSourceChange: (source: ContentSource) => void;
  onNavigate: (view: WorkspaceViewId) => void; onSettings: () => void;
  initialReview?: WorkKey | null;
  api?: CommunityApi;
};
export function CommunityWorkspace({ source, collapsed, autoFindCount = 0, attentionCount, onToggleRail, onSourceChange, onNavigate, onSettings, initialReview = null, api = communityApi }: Props) {
  // The workspace mounts on navigation. Reading from the rail leaves the editor
  // closed; only an explicit review shortcut initializes a writing attempt.
  const [sourceFilter, setSourceFilter] = useState<CommunitySource | "all">(initialReview?.source ?? "all");
  const [workDraft, setWorkDraft] = useState(initialReview?.workId ?? "");
  const [query, setQuery] = useState<{ source: CommunitySource | null; workId: string | null }>(initialReview ?? { source: null, workId: null });
  const [page, setPage] = useState<ReviewPage>({ items: [], nextCursor: null });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [editorKey, setEditorKey] = useState<WorkKey | null>(initialReview);
  const loadSequence = useRef(0);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++loadSequence.current; }; }, []);
  const load = useCallback(async (cursor: Cursor | null = null) => {
    const sequence = ++loadSequence.current;
    setLoading(true); setError(null);
    try {
      const result = query.source && query.workId ? await api.work({ source: query.source, workId: query.workId }, cursor) : await api.feed(query.source, cursor);
      if (!mounted.current || sequence !== loadSequence.current) return;
      setPage((previous) => ({ ...result, items: cursor ? [...previous.items, ...result.items.filter((item) => !previous.items.some((old) => old.id === item.id))] : result.items }));
    } catch (error) {
      if (mounted.current && sequence === loadSequence.current) setError(communityError(error));
    } finally { if (mounted.current && sequence === loadSequence.current) setLoading(false); }
  }, [api, query]);
  useEffect(() => { setPage({ items: [], nextCursor: null }); void load(); }, [load]);
  const search = (event: FormEvent) => {
    event.preventDefault(); const id = workDraft.trim();
    if (id && (!validWorkId(id) || sourceFilter === "all")) { setError("작품번호 검색 시 Hitomi 또는 Danbooru를 선택하고 숫자로 입력해 주세요."); return; }
    setQuery({ source: sourceFilter === "all" ? null : sourceFilter, workId: id || null });
  };
  const chooseWork = (work: WorkKey) => {
    setSourceFilter(work.source); setWorkDraft(work.workId); setQuery(work);
  };
  const startWriting = () => {
    const id = workDraft.trim();
    if (sourceFilter === "all" || !validWorkId(id)) { setError("후기를 남길 사이트와 작품번호를 먼저 입력해 주세요."); return; }
    setNotice(null); setError(null); setEditorKey({ source: sourceFilter, workId: id });
  };
  return <div className={`app-shell community-shell${collapsed ? " sidebar-collapsed" : ""}`}>
    <SideRail source={source} view={workspaceRegistry[source].navigation[0].view} collapsed={collapsed} autoFindCount={autoFindCount} attentionCount={attentionCount} sourceLabel="Community · Supabase" onNavigate={onNavigate} onSourceChange={onSourceChange} onToggle={onToggleRail} />
    <main className="community-workspace">
      <header className="community-header"><div><span className="eyebrow">ATSUMI COMMUNITY</span><h1>커뮤니티</h1><p>작품번호로 찾고, 짧은 감상을 나눠보세요.</p></div><button className="text-button" onClick={onSettings}>설정</button></header>
      <div className="community-content">
        <form className="community-search" onSubmit={search}>
          <DropdownSelect ariaLabel="후기 사이트" value={sourceFilter} options={sourceOptions} onChange={setSourceFilter} variant="toolbar" />
          <input aria-label="후기 작품번호" placeholder="작품번호 · 비워두면 최신 후기" inputMode="numeric" maxLength={20} value={workDraft} onChange={(e) => setWorkDraft(e.target.value)} />
          <button className="text-button" type="submit">후기 찾기</button>
          <button className="text-button primary" type="button" onClick={startWriting}>후기 작성 / 수정</button>
        </form>
        <p className="community-privacy">열람은 인증 없이 · 첫 작성 때만 익명 키 발급 · 앱 초기화 후에도 작성자 키 유지</p>
        <div className={`community-columns${editorKey ? " is-editing" : ""}`}>
          <section className="community-feed" aria-label="공개 후기 목록">
            <header className="community-feed-heading"><h2>{query.source && query.workId ? workLabel({ source: query.source, workId: query.workId }) : "최신 후기"}</h2><button type="button" className="text-button" disabled={loading} onClick={() => void load()}>새로고침</button></header>
            {page.summary ? <div className="community-summary"><strong>{page.summary.averageRating === null ? "아직 별점 없음" : `★ ${page.summary.averageRating.toFixed(1)}`}</strong><span>후기 {page.summary.reviewCount}개</span><span>추천 {page.summary.recommendationCount}명</span></div> : null}
            {error ? <p className="community-error" role="alert">{error}</p> : null}
            {notice ? <p className="community-notice" role="status">{notice}</p> : null}
            {loading && !page.items.length ? <div className="community-loading" role="status" aria-label="후기 불러오는 중">{[1, 2, 3].map((key) => <div key={key} />)}</div> : null}
            {!loading && !error && !page.items.length ? <div className="community-empty"><strong>아직 등록된 후기가 없습니다.</strong><p>별점만 남겨도 좋아요. 사이트와 작품번호를 선택해 첫 후기를 남겨보세요.</p></div> : null}
            {page.items.map((review) => <ReviewCard key={review.id} review={review} onChoose={() => chooseWork(review)} onEdit={() => { setEditorKey({ source: review.source, workId: review.workId }); setNotice(null); }} onReport={async (reason) => { await api.report(review.id, reason); setNotice("신고가 접수되었습니다. 운영자가 확인합니다."); }} />)}
            {page.nextCursor ? <button className="text-button community-more" disabled={loading} onClick={() => void load(page.nextCursor)}>{loading ? "불러오는 중…" : "후기 더 보기"}</button> : null}
          </section>
          {editorKey ? <ReviewEditor key={`${editorKey.source}:${editorKey.workId}`} work={editorKey} api={api} onClose={() => setEditorKey(null)} onSaved={(message) => { setNotice(message); setEditorKey(null); chooseWork(editorKey); }} /> : null}
        </div>
        <details className="community-help"><summary>익명 키와 공개 범위 안내</summary><p>공개되는 정보는 사이트·작품번호·닉네임·후기·별점·추천 여부입니다. 다운로드 내역, 기기 식별 정보, 이미지, 파일 경로는 전송하지 않습니다. Supabase는 서비스 운영에 필요한 접속 정보(IP 등)를 처리할 수 있습니다.</p><p>작성자 키는 Windows 사용자 계정으로 암호화해 앱 설정·캐시와 별도로 보관합니다. 앱 초기화는 키와 서버 후기를 지우지 않습니다. Windows 재설치 또는 보관 파일을 직접 삭제하면 이전 후기의 수정·삭제 권한을 잃을 수 있습니다. 현재 다른 PC로 옮기거나 분실한 키를 복구하는 기능은 없습니다.</p><p>닉네임은 중복될 수 있으며 본인 인증을 의미하지 않습니다. 작품마다 이 앱의 작성자 키로 후기 한 개를 남길 수 있습니다. 개인정보·불법 콘텐츠·도배는 게시하지 마세요.</p></details>
      </div>
    </main>
  </div>;
}

function ReviewCard({ review, onChoose, onEdit, onReport }: { review: Review; onChoose: () => void; onEdit: () => void; onReport: (reason: string) => Promise<void> }) {
  const [reporting, setReporting] = useState(false); const [reason, setReason] = useState(""); const [busy, setBusy] = useState(false); const [error, setError] = useState<string | null>(null);
  const report = async (event: FormEvent) => { event.preventDefault(); if (busy) return; setBusy(true); setError(null); try { await onReport(reason.trim()); setReporting(false); setReason(""); } catch (error) { setError(communityError(error)); } finally { setBusy(false); } };
  return <article className="community-review">
    <div className="community-review-top"><button className="community-work-link" onClick={onChoose}>{workLabel(review)}</button><span className="community-stars" aria-label={`별점 ${review.rating}점`}>{"★".repeat(review.rating)}<span>{"☆".repeat(5 - review.rating)}</span></span>{review.recommended ? <span className="community-recommended">추천</span> : null}</div>
    {review.comment ? <p className="community-comment">{review.comment}</p> : <p className="community-comment is-empty">별점을 남겼습니다.</p>}
    <footer><span>{review.nickname}</span><time dateTime={review.createdAt}>{dateLabel(review.createdAt)}</time><button className="community-subtle" onClick={onEdit}>내 후기 작성 / 수정</button><button className="community-subtle" onClick={() => setReporting((value) => !value)}>신고</button></footer>
    {reporting ? <form className="community-report" onSubmit={(e) => void report(e)}><input aria-label="신고 사유" placeholder="신고 사유 (이미 발급된 작성자 키 필요)" maxLength={300} required value={reason} onChange={(e) => setReason(e.target.value)} /><button className="text-button" disabled={busy || !reason.trim()}>신고 접수</button></form> : null}
    {error ? <p role="alert" className="community-error">{error}</p> : null}
  </article>;
}

function ReviewEditor({ work, api, onClose, onSaved }: { work: WorkKey; api: CommunityApi; onClose: () => void; onSaved: (message: string) => void }) {
  const [writer, setWriter] = useState<Writer | null>(null); const [error, setError] = useState<string | null>(null); const [busy, setBusy] = useState(true);
  const [input, setInput] = useState<ReviewInput>({ ...work, nickname: "", rating: 5, recommended: false, comment: "" });
  const pending = useRef(false); const request = useRef<Promise<Writer> | null>(null);
  // StrictMode effect re-entry shares a single issuance attempt. Native locking
  // and the persistent Issuing marker protect reload/multiple processes too.
  useEffect(() => {
    let active = true;
    request.current ??= api.beginWriting(work);
    void request.current.then((value) => { if (!active) return; setWriter(value); setInput({ ...work, nickname: value.profile.nickname, rating: value.mine?.rating ?? 5, recommended: value.mine?.recommended ?? false, comment: value.mine?.comment ?? "" }); }).catch((error) => { if (active) setError(communityError(error)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [api, work]);
  const submit = async (remove = false) => {
    if (pending.current || !writer) return;
    if (remove && !window.confirm(`${workLabel(work)}에 남긴 내 후기를 삭제할까요? 삭제한 후기는 복구할 수 없습니다.`)) return;
    pending.current = true; setBusy(true); setError(null);
    try { if (remove) await api.delete(work); else await api.save(input); onSaved(remove ? "내 후기를 삭제했습니다. 작성자 키는 그대로 유지됩니다." : "후기를 저장했습니다."); }
    catch (error) { setError(communityError(error)); }
    finally { pending.current = false; setBusy(false); }
  };
  return <section className="community-editor" aria-label="후기 작성">
    <header><div><span className="eyebrow">MY REVIEW</span><h2>{workLabel(work)}</h2></div><button type="button" className="text-button" onClick={onClose} disabled={busy}>닫기</button></header>
    {busy && !writer ? <p role="status">이 앱의 작성자 키를 준비하는 중…</p> : null}
    {error ? <p className="community-error" role="alert">{error}</p> : null}
    {writer ? <form onSubmit={(e) => { e.preventDefault(); void submit(); }}>
      {writer.mine?.hidden ? <p className="community-error">운영자가 숨긴 후기입니다. 수정해도 공개 상태로 바뀌지 않습니다.</p> : null}
      <label>닉네임<input aria-label="후기 닉네임" required minLength={2} maxLength={24} value={input.nickname} onChange={(e) => setInput((old) => ({ ...old, nickname: e.target.value }))} disabled={busy} /></label>
      <fieldset className="community-rating"><legend>별점</legend>{[1, 2, 3, 4, 5].map((rating) => <button key={rating} type="button" aria-label={`${rating}점`} aria-pressed={input.rating === rating} className={rating <= input.rating ? "is-filled" : ""} disabled={busy} onClick={() => setInput((old) => ({ ...old, rating }))}>★</button>)}</fieldset>
      <label className="community-recommend-input"><input type="checkbox" checked={input.recommended} disabled={busy} onChange={(e) => setInput((old) => ({ ...old, recommended: e.target.checked }))} />추천하는 작품이에요</label>
      <label>짧은 후기<textarea aria-label="짧은 후기" placeholder="어떤 점이 좋았나요? (별점만 등록해도 됩니다)" maxLength={500} rows={6} disabled={busy} value={input.comment} onChange={(e) => setInput((old) => ({ ...old, comment: e.target.value }))} /><span className="community-charcount">{Array.from(input.comment).length} / 500</span></label>
      <p className="community-editor-note">등록하면 누구나 열람할 수 있습니다. 닉네임 변경은 이전 후기에도 적용됩니다. 수정·삭제 권한은 이 앱에 보관된 익명 키로 확인합니다.</p>
      <div className="community-editor-actions">{writer.mine ? <button type="button" className="text-button danger-button" disabled={busy} onClick={() => void submit(true)}>내 후기 삭제</button> : null}<button className="text-button primary" disabled={busy || Array.from(input.nickname.trim()).length < 2}>{busy ? "저장 중…" : writer.mine ? "후기 수정" : "후기 등록"}</button></div>
    </form> : null}
  </section>;
}
