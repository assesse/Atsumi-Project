import { useEffect, useMemo, useRef, useState } from "react";
import { createScreenshotsApi, type ScreenshotItem, type ScreenshotsApi } from "../../api/screenshots";
import { createLiveChannelsApi } from "../../api/liveChannels";
import { emptyRecordingSelection, retainRecordingSelection, selectRecording } from "./recordingSelection";
import "./RecordingLibrary.css";
import "./ScreenshotLibrary.css";

const date = (value: number) => new Date(value).toLocaleString("ko-KR", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
const message = (cause: unknown) => cause instanceof Error ? cause.message : "스크린샷을 불러오지 못했습니다.";
// Only two thumbnails can decode concurrently, including during a quick tab change.
let imagesInFlight = 0;
const waitingImages: (() => void)[] = [];
async function withImageSlot<T>(work: () => Promise<T>): Promise<T> {
  if (imagesInFlight >= 2) await new Promise<void>(resolve => waitingImages.push(resolve));
  else imagesInFlight++;
  try { return await work(); }
  finally { const next = waitingImages.shift(); if (next) next(); else imagesInFlight--; }
}
function Thumbnail({ item, api, privacy }: { item: ScreenshotItem; api: ScreenshotsApi; privacy: boolean }) {
  const target = useRef<HTMLSpanElement>(null);
  const [source, setSource] = useState(""); const [error, setError] = useState(false);
  useEffect(() => {
    setSource(""); setError(false);
    if (privacy) return;
    let cancelled = false, started = false;
    const load = () => {
      if (started || cancelled) return; started = true;
      void withImageSlot(() => cancelled ? Promise.resolve("") : api.image(item.id)).then(value => { if (!cancelled) setSource(value); }).catch(() => { if (!cancelled) setError(true); });
    };
    const observer = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { observer?.disconnect(); load(); } }, { rootMargin: "180px" });
    if (observer && target.current) observer.observe(target.current); else load();
    return () => { cancelled = true; observer?.disconnect(); };
  }, [api, item.id, privacy]);
  return <span ref={target} className="screenshot-thumbnail" aria-busy={!privacy && !source && !error}>
    {privacy ? <span>프라이버시 모드</span> : source ? <img src={source} alt="" decoding="async" /> : <span>{error ? "미리보기 없음" : "불러오는 중…"}</span>}
  </span>;
}
function ScreenshotViewer({ item, api, close, move }: { item: ScreenshotItem; api: ScreenshotsApi; close(): void; move(direction: number): void }) {
  const dialog = useRef<HTMLDialogElement>(null); const [source, setSource] = useState(""); const [error, setError] = useState("");
  useEffect(() => { dialog.current?.showModal(); return () => { dialog.current?.close(); }; }, []);
  useEffect(() => {
    let cancelled = false; setSource(""); setError("");
    void withImageSlot(() => cancelled ? Promise.resolve("") : api.image(item.id, true)).then(value => { if (!cancelled) setSource(value); }).catch(cause => { if (!cancelled) setError(message(cause)); });
    return () => { cancelled = true; };
  }, [api, item.id]);
  return <dialog ref={dialog} className="screenshot-viewer" aria-label="스크린샷 보기" onCancel={event => { event.preventDefault(); close(); }} onKeyDown={event => {
    if (event.key === "ArrowLeft") { event.preventDefault(); move(-1); }
    if (event.key === "ArrowRight") { event.preventDefault(); move(1); }
  }}>
    <header><span>{date(item.createdAt)} · {item.width}×{item.height}</span><button type="button" className="screenshot-close" aria-label="닫기" onClick={close}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg></button></header>
    <div className="screenshot-viewer-image" aria-busy={!source && !error}>{source ? <img src={source} alt="저장된 스크린샷" /> : <p role={error ? "alert" : "status"}>{error || "불러오는 중…"}</p>}</div>
    <footer><button type="button" onClick={() => move(-1)}>이전</button><button type="button" onClick={() => move(1)}>다음</button></footer>
  </dialog>;
}
export function ScreenshotLibrary({ runtime, privacy = false, api: providedApi }: { runtime: "tauri" | "browser-mock"; privacy?: boolean; api?: ScreenshotsApi }) {
  const api = useMemo(() => providedApi ?? createScreenshotsApi(runtime), [providedApi, runtime]);
  const [items, setItems] = useState<ScreenshotItem[]>([]), [loading, setLoading] = useState(true), [error, setError] = useState("");
  const [selection, setSelection] = useState(emptyRecordingSelection), [limit, setLimit] = useState(24);
  const [viewing, setViewing] = useState<string | null>(null), [confirm, setConfirm] = useState<string[] | null>(null), [deleting, setDeleting] = useState(false), [openingFolder, setOpeningFolder] = useState(false);
  const [names, setNames] = useState<Record<string, string>>({});
  const root = useRef<HTMLElement>(null), confirmDialog = useRef<HTMLDialogElement>(null);
  const mounted = useRef(false), revision = useRef(0), deletePending = useRef(false);
  const refresh = async () => {
    const ticket = ++revision.current; setLoading(true); setError("");
    try {
      const value = await api.list();
      if (mounted.current && ticket === revision.current) { setItems(value); setSelection(previous => retainRecordingSelection(previous, value.map(item => item.id))); }
    } catch (cause) { if (mounted.current && ticket === revision.current) setError(message(cause)); }
    finally { if (mounted.current && ticket === revision.current) setLoading(false); }
  };
  useEffect(() => { mounted.current = true; void refresh(); return () => { mounted.current = false; revision.current++; }; }, [api]);
  const visible = items.slice(0, limit), ids = visible.map(item => item.id), selecting = selection.ids.length >= 2;
  useEffect(() => { if (privacy) { setViewing(null); setSelection(emptyRecordingSelection()); } }, [privacy]);
  const channels = [...new Set(visible.map(item => item.channelId).filter(Boolean))].join(",");
  useEffect(() => {
    if (privacy || runtime !== "tauri") return;
    let cancelled = false; const channelsApi = createLiveChannelsApi(runtime);
    // Sequential, deduplicated profile lookup; never one network request per photo.
    void (async () => {
      for (const channel of channels.split(",").filter(Boolean)) {
        if (cancelled) break;
        if (names[channel]) continue;
        const result = await channelsApi.profile(channel);
        if (!cancelled && result.ok) setNames(previous => ({ ...previous, [channel]: result.data.channelName }));
      }
    })();
    return () => { cancelled = true; };
  }, [channels, privacy, runtime]);
  useEffect(() => {
    if (confirm && !confirmDialog.current?.open) confirmDialog.current?.showModal();
    else if (!confirm && confirmDialog.current?.open) { confirmDialog.current.close(); root.current?.focus(); }
  }, [confirm]);
  const folder = async () => {
    if (openingFolder) return; setOpeningFolder(true); setError("");
    try { await api.folder(selection.ids.length === 1 ? selection.ids[0] : undefined); }
    catch (cause) { if (mounted.current) setError(message(cause)); }
    finally { if (mounted.current) setOpeningFolder(false); }
  };
  const remove = async () => {
    if (!confirm || deletePending.current) return; deletePending.current = true; setDeleting(true); setError("");
    try {
      const result = await api.delete(confirm);
      if (!mounted.current) return;
      setItems(previous => previous.filter(item => !result.deleted.includes(item.id)));
      setSelection(previous => ({ ids: previous.ids.filter(id => !result.deleted.includes(id)), anchorId: null }));
      setConfirm(null);
      if (result.failures.length) setError(`${result.failures.length}개 삭제 실패 · ${result.failures[0]?.message ?? "파일을 확인해 주세요."}`);
    } catch (cause) { if (mounted.current) setError(message(cause)); }
    finally { deletePending.current = false; if (mounted.current) setDeleting(false); }
  };
  const viewed = items.find(item => item.id === viewing);
  return <section ref={root} className="recording-library screenshot-library" aria-label="스크린샷 목록" tabIndex={-1} onKeyDown={event => {
    if (viewing || confirm || deleting || (event.target as Element).closest("input,textarea,select")) return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") { event.preventDefault(); setSelection(event.shiftKey ? emptyRecordingSelection() : { ids, anchorId: ids[0] ?? null }); }
    else if (event.key === "Escape") setSelection(emptyRecordingSelection());
    else if (event.key === "Delete" && selection.ids.length) { event.preventDefault(); setConfirm([...selection.ids]); }
  }}>
    <header className="recording-library-heading"><div><h2>스크린샷</h2><p>{items.length}개</p></div><div className="recording-library-heading-actions">
      <button type="button" disabled={loading || deleting} aria-busy={loading} onClick={() => void refresh()}>{loading ? "불러오는 중…" : "새로고침"}</button>
      <button type="button" disabled={openingFolder || runtime !== "tauri"} aria-busy={openingFolder} onClick={() => void folder()}>저장 폴더 열기</button>
    </div></header>
    {error ? <p role="alert" className="recording-library-delete-errors">{error}</p> : null}
    {selecting ? <div className="recording-library-selection"><span>{selection.ids.length}개 선택</span><button type="button" disabled={deleting} onClick={() => setSelection(emptyRecordingSelection())}>선택 해제</button><button type="button" disabled={deleting} className="recording-library-delete" onClick={() => setConfirm([...selection.ids])}>선택 삭제</button></div> : selection.ids.length === 1 ? <div className="recording-library-single-actions"><span>1개 선택</span><div><button type="button" disabled={privacy || deleting} onClick={() => setViewing(selection.ids[0] ?? null)}>열기</button> <button type="button" disabled={deleting} className="recording-library-delete" onClick={() => setConfirm([...selection.ids])}>삭제</button></div></div> : null}
    {!loading && !items.length ? <div className="recording-library-empty"><strong>저장된 스크린샷이 없습니다.</strong><p>라이브 플레이어의 카메라 버튼으로 저장할 수 있습니다.</p></div> : null}
    <div className="recording-library-grid screenshot-grid">
      {visible.map(item => <div key={item.id} className={`recording-library-card-wrap${selecting ? " is-selecting" : ""}`}>
        <button type="button" className="recording-library-card screenshot-card" aria-pressed={selection.ids.includes(item.id)} disabled={deleting}
          aria-label={privacy ? "스크린샷" : `${names[item.channelId] || "스크린샷"} · ${date(item.createdAt)}`} onClick={event => {
            if (event.detail > 1) return;
            if (event.ctrlKey || event.metaKey || event.shiftKey) event.preventDefault();
            setSelection(previous => selectRecording(previous, item.id, ids, { ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey }));
            if (!event.shiftKey && selection.ids.includes(item.id) && (event.ctrlKey || event.metaKey || selection.ids.length === 1)) event.currentTarget.blur();
          }} onDoubleClick={event => { if (!event.ctrlKey && !event.metaKey && !event.shiftKey && !privacy) setViewing(item.id); }}
          onKeyDown={event => {
            if (event.key === "Enter" && !privacy) { event.preventDefault(); setViewing(item.id); }
            else if (event.key === " ") { event.preventDefault(); setSelection(previous => selectRecording(previous, item.id, ids, { ctrl: true, shift: event.shiftKey })); }
          }}>
          <Thumbnail item={item} api={api} privacy={privacy} />
          <span className="screenshot-card-caption"><strong>{privacy ? "스크린샷" : names[item.channelId] || "스크린샷"}</strong><time>{date(item.createdAt)}</time><small>{item.width}×{item.height} · {(item.bytes / 1_000_000).toFixed(1)} MB</small></span>
        </button>
        {selecting ? <input className="recording-library-check" type="checkbox" aria-label={`${privacy ? "스크린샷" : date(item.createdAt)} 선택`} checked={selection.ids.includes(item.id)} disabled={deleting} onChange={() => setSelection(previous => selectRecording(previous, item.id, ids, { ctrl: true, shift: false }))} /> : null}
      </div>)}
    </div>
    {limit < items.length ? <button type="button" className="recording-library-more" onClick={() => setLimit(value => value + 24)}>더 보기</button> : null}
    <dialog ref={confirmDialog} className="recording-library-delete-dialog" aria-labelledby="screenshot-delete-heading" onCancel={event => { event.preventDefault(); if (!deleting) setConfirm(null); }}>
      <h3 id="screenshot-delete-heading">스크린샷 {confirm?.length ?? 0}개를 삭제할까요?</h3><p>선택한 이미지 파일을 삭제합니다. 되돌릴 수 없습니다.</p>
      {error ? <p role="alert">{error}</p> : null}
      <footer><button type="button" autoFocus disabled={deleting} onClick={() => setConfirm(null)}>취소</button><button type="button" className="recording-library-delete" disabled={deleting} onClick={() => void remove()}>{deleting ? "삭제 중…" : "삭제"}</button></footer>
    </dialog>
    {viewed && !privacy ? <ScreenshotViewer item={viewed} api={api} close={() => { setViewing(null); root.current?.focus(); }} move={direction => {
      const index = items.findIndex(item => item.id === viewing); setViewing(items[Math.max(0, Math.min(items.length - 1, index + direction))]?.id ?? null);
    }} /> : null}
  </section>;
}
