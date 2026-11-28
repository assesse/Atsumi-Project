import { useEffect, useRef, useState } from "react";
import { FluentIcon } from "../../components/FluentIcon";
import type { BrowserApi, BrowserPreferences, BrowserSnapshot } from "./BrowserApp";
export type LibraryKind = "history" | "bookmarks" | "preferences";
export type LibraryEntry = { id: string; url: string; title: string; visitedAt: number };
export type LibraryPage = { items: LibraryEntry[]; hasMore: boolean; nextOffset: number };
const labels = { history: "방문 기록", bookmarks: "북마크", preferences: "브라우저 설정" };
const dateLabel = (value: number) => new Date(value).toLocaleString("ko-KR", { month: "long", day: "numeric", hour: "2-digit", minute: "2-digit" });
function Toggle({ label, hint, checked, disabled, onChange }: { label: string; hint: string; checked: boolean; disabled?: boolean; onChange: () => void }) {
  return <div className="browser-preference-row"><div><strong>{label}</strong><p>{hint}</p></div><button type="button" className="browser-switch" role="switch" aria-label={label} aria-checked={checked} disabled={disabled} onClick={onChange}><span className="browser-switch-track"><span/></span><span>{checked ? "켜짐" : "꺼짐"}</span></button></div>;
}
export function BrowserLibrary({ kind, api, state, onKind, onOpen, onClose, onChanged }: { kind: LibraryKind; api: BrowserApi; state: BrowserSnapshot; onKind: (kind: LibraryKind) => void; onOpen: (url: string, newTab?: boolean) => void; onClose: () => void; onChanged: () => void }) {
  const [query, setQuery] = useState("");
  const [page, setPage] = useState<LibraryPage>({ items: [], hasMore: false, nextOffset: 0 });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [confirmClear, setConfirmClear] = useState(false);
  const [saving, setSaving] = useState(false);
  const serial = useRef(0);
  useEffect(() => { setQuery(""); setConfirmClear(false); }, [kind]);
  useEffect(() => {
    const request = ++serial.current;
    setError(null); setPage({ items: [], hasMore: false, nextOffset: 0 });
    if (kind === "preferences") { setLoading(false); return; }
    setLoading(true);
    const timer = window.setTimeout(() => {
      void api.libraryQuery(kind, query, 0).then(value => { if (request === serial.current) setPage(value); }, e => { if (request === serial.current) setError(String(e)); }).finally(() => { if (request === serial.current) setLoading(false); });
    }, query ? 180 : 0);
    return () => { window.clearTimeout(timer); ++serial.current; };
  }, [api, kind, query, revision]);
  const more = async () => {
    if (loading || kind === "preferences") return;
    const request = serial.current; setLoading(true);
    try { const next = await api.libraryQuery(kind, query, page.nextOffset); if (request === serial.current) setPage(previous => ({ ...next, items: [...previous.items, ...next.items] })); }
    catch (e) { if (request === serial.current) setError(String(e)); }
    finally { if (request === serial.current) setLoading(false); }
  };
  const remove = async (id?: string) => {
    if (kind === "preferences" || saving || loading) return;
    setSaving(true); setError(null);
    try { await api.libraryUpdate(kind, id ? "remove" : "clear", id); setConfirmClear(false); if (id) setPage(previous => ({ ...previous, items: previous.items.filter(item => item.id !== id), nextOffset: Math.max(0, previous.nextOffset - 1) })); else setRevision(n => n + 1); onChanged(); }
    catch (e) { setError(String(e)); } finally { setSaving(false); }
  };
  const preference = async (key: keyof BrowserPreferences) => {
    setSaving(true); setError(null);
    try { await api.preferences({ ...state.preferences, [key]: !state.preferences[key] }); }
    catch (e) { setError(String(e)); } finally { setSaving(false); }
  };
  return <section className="browser-library" aria-label={labels[kind]}>
    <div className="browser-library-heading"><h1>{labels[kind]}</h1><button type="button" className="browser-icon-button" aria-label="브라우저로 돌아가기" title="브라우저로 돌아가기 · Esc" onClick={onClose}><FluentIcon glyph="\uE711"/></button></div>
    <nav className="browser-library-tabs" aria-label="브라우저 자료와 설정">{(Object.keys(labels) as LibraryKind[]).map(value => <button type="button" key={value} aria-current={kind === value ? "page" : undefined} onClick={() => onKind(value)}><FluentIcon glyph={value === "history" ? "\uE81C" : value === "bookmarks" ? "\uE734" : "\uE713"}/>{labels[value]}</button>)}</nav>
    {error && <p role="alert" className="browser-library-error">{error}</p>}
    {kind === "preferences" ? <div className="browser-preferences">
      <section className="browser-preference-card"><h2>다음에도 이어서</h2><Toggle label="이전 탭 다시 열기" hint="브라우저를 다시 열면 마지막에 보던 탭으로 돌아옵니다." checked={state.preferences.restoreTabs} disabled={saving} onChange={() => void preference("restoreTabs")}/><div className="browser-session-note"><FluentIcon glyph="\uE73E"/><div><strong>로그인 정보 유지</strong><p>이 브라우저의 쿠키와 사이트 데이터를 보관합니다. 사이트에서 로그인이 만료되면 다시 로그인해 주세요.</p></div></div></section>
      <section className="browser-preference-card"><h2>비밀번호와 자동 완성</h2><Toggle label="비밀번호 저장 제안" hint="로그인할 때 브라우저가 저장 여부를 묻습니다." checked={state.preferences.savePasswords} disabled={saving || !state.passwordSavingAvailable} onChange={() => void preference("savePasswords")}/><Toggle label="입력 내용 자동 완성" hint="이름과 이메일 등 이전에 입력한 정보를 제안합니다." checked={state.preferences.autofill} disabled={saving || !state.passwordSavingAvailable} onChange={() => void preference("autofill")}/><p className="browser-preference-hint">{state.passwordSavingAvailable ? "저장한 비밀번호는 브라우저가 관리합니다. 저장 제안을 꺼도 기존 비밀번호의 자동 완성은 유지됩니다." : "비밀번호 저장 기능을 준비하고 있습니다. 계속 사용할 수 없으면 WebView2 업데이트를 확인해 주세요."}</p></section>
    </div> : <>
      <div className="browser-library-tools"><label className="browser-library-search"><FluentIcon glyph="\uE721"/><input aria-label={labels[kind] + " 검색"} placeholder={labels[kind] + " 검색"} value={query} onChange={e => setQuery(e.target.value)}/></label>{kind === "history" && <button type="button" className="browser-button" onClick={() => setConfirmClear(true)} disabled={saving}>기록 삭제</button>}</div>
      {confirmClear && <div className="browser-clear-confirm" role="alertdialog" aria-label="방문 기록 삭제 확인"><div><strong>방문 기록을 모두 지울까요?</strong><p>북마크와 로그인 정보는 남습니다.</p></div><button type="button" className="browser-button" onClick={() => setConfirmClear(false)} disabled={saving}>취소</button><button type="button" className="browser-button danger" onClick={() => void remove()} disabled={saving}>모두 삭제</button></div>}
      <div className="browser-library-list" aria-busy={loading}>{page.items.map(item => <article className="browser-library-row" key={item.id}><button type="button" className="browser-library-open" onClick={() => onOpen(item.url)} title={item.url}><span className="browser-history-icon"><FluentIcon glyph={kind === "history" ? "\uE774" : "\uE735"}/></span><span><strong>{item.title || item.url}</strong><span>{item.url}</span></span></button><time dateTime={new Date(item.visitedAt).toISOString()}>{dateLabel(item.visitedAt)}</time><button type="button" className="browser-icon-button" aria-label={(item.title || item.url) + " 새 탭에서 열기"} title="새 탭에서 열기" onClick={() => onOpen(item.url, true)}><FluentIcon glyph="\uE8A7"/></button><button type="button" className="browser-icon-button" aria-label={(item.title || item.url) + " 삭제"} title="삭제" disabled={saving || loading} onClick={() => void remove(item.id)}><FluentIcon glyph="\uE74D"/></button></article>)}</div>
      {loading && <p role="status" className="browser-library-empty"><span className="browser-spinner"/>불러오는 중</p>}
      {!loading && !error && !page.items.length && <div className="browser-library-empty"><FluentIcon glyph={kind === "history" ? "\uE81C" : "\uE734"}/><strong>{query ? "검색 결과가 없습니다." : kind === "history" ? "방문한 페이지가 여기에 쌓입니다." : "주소창의 별을 눌러 북마크를 추가하세요."}</strong></div>}
      {page.hasMore && <button type="button" className="browser-button browser-library-more" disabled={loading || saving} onClick={() => void more()}>더 보기</button>}
    </>}
  </section>;
}
