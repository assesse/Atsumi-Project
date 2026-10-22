import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { pinterestError } from "./api";
import { type PinterestBrowserAction, type PinterestBrowserApi, type PinterestBrowserSnapshot, pinterestBrowserSurfaceRect } from "./browserApi";
import "./PinterestOfficialSearch.css";

export function PinterestOfficialSearch({ api, query, revision, active, privacyMode, onImport, onReturn }: {
  api: PinterestBrowserApi; query: string; revision: number; active: boolean; privacyMode: boolean;
  onImport: (id: string) => Promise<void>; onReturn: () => void;
}) {
  const [snapshot, setSnapshot] = useState<PinterestBrowserSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null), [busy, setBusy] = useState(false);
  const [attempt, setAttempt] = useState(0), [importing, setImporting] = useState(false);
  const surface = useRef<HTMLDivElement>(null), epoch = useRef<number | null>(null), sequence = useRef(0);
  const mounted = useRef(true), importingRef = useRef(false);
  const importRequest = useRef(0), openingGeneration = useRef(0);
  const actionsGeneration = useRef(0);
  const currentVisible = useRef(active && !privacyMode);
  currentVisible.current = active && !privacyMode;
  const hide = (current: number) => api.viewport({ epoch: current, sequence: ++sequence.current, x: 0, y: 0, width: 0, height: 0, visible: false }).catch(() => {});
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; importRequest.current++; actionsGeneration.current++; const current = epoch.current; if (current !== null) { void hide(current); void api.action(current, "close").catch(() => {}); } }; }, [api]);
  useEffect(() => {
    const generation = ++openingGeneration.current;
    actionsGeneration.current++;
    let current = true;
    setBusy(true); setError(null); setSnapshot(null);
    const previous = epoch.current; epoch.current = null;
    if (previous !== null) void hide(previous);
    void api.open(query).then(value => {
      if (!current || !mounted.current || openingGeneration.current !== generation) { void api.action(value.epoch, "close").catch(() => {}); return; }
      epoch.current = value.epoch; sequence.current = 0; setSnapshot(value);
      if (!currentVisible.current) void hide(value.epoch);
    }).catch(cause => { if (current) setError(pinterestError(cause)); }).finally(() => { if (current) setBusy(false); });
    return () => { current = false; importRequest.current++; };
  }, [api, query, revision, attempt]);
  useEffect(() => {
    const current = snapshot?.epoch;
    if (current === undefined || !active || privacyMode) return;
    let disposed = false, pending = false;
    const poll = async () => {
      if (pending || disposed || document.hidden) return;
      pending = true;
      try { const next = await api.snapshot(current); if (!disposed && epoch.current === current) setSnapshot(next); }
      catch (cause) { if (!disposed) setError(pinterestError(cause)); }
      finally { pending = false; }
    };
    const timer = window.setInterval(() => void poll(), 900);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [active, api, privacyMode, snapshot?.epoch]);
  useLayoutEffect(() => {
    const current = snapshot?.epoch;
    if (current === undefined) return;
    if (!active || privacyMode || importing) { void hide(current); return; }
    let stopped = false, frame = 0, last = "";
    const measure = () => {
      frame = 0; if (stopped || !surface.current) return;
      const blockers = [...document.querySelectorAll<HTMLElement>('dialog[open],[role="dialog"][aria-modal="true"],[role="alertdialog"],[role="menu"],[role="listbox"],.toast,[data-native-overlay="true"]')].some(el => {
        if (el.closest('[hidden],[aria-hidden="true"]')) return false;
        const style = getComputedStyle(el); return style.display !== "none" && style.visibility !== "hidden";
      });
      const rect = pinterestBrowserSurfaceRect(surface.current, window.innerWidth, window.innerHeight);
      const value = { ...rect, visible: rect.visible && !blockers && !document.hidden && !document.fullscreenElement && currentVisible.current };
      const key = JSON.stringify(value); if (key === last) return; last = key;
      void api.viewport({ ...value, epoch: current, sequence: ++sequence.current }).catch(cause => { if (!stopped) { last = ""; setError(pinterestError(cause)); } });
    };
    // Hides are immediate; coalesce ordinary resize/scroll writes to a frame.
    const update = () => { if (!frame) frame = requestAnimationFrame(measure); };
    const changed = () => { measure(); };
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    for (let element: Element | null = surface.current; element; element = element.parentElement) resize?.observe(element);
    const mutations = new MutationObserver(changed);
    mutations.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["open", "hidden", "class", "style", "aria-hidden", "aria-modal", "role", "data-native-overlay"] });
    window.addEventListener("resize", update); window.addEventListener("scroll", update, true); document.addEventListener("visibilitychange", changed); document.addEventListener("fullscreenchange", changed);
    measure();
    return () => { stopped = true; if (frame) cancelAnimationFrame(frame); resize?.disconnect(); mutations.disconnect(); window.removeEventListener("resize", update); window.removeEventListener("scroll", update, true); document.removeEventListener("visibilitychange", changed); document.removeEventListener("fullscreenchange", changed); void hide(current); };
  }, [active, api, privacyMode, importing, snapshot?.epoch]);
  const action = async (kind: PinterestBrowserAction) => {
    if (epoch.current === null || busy) return;
    const current = epoch.current, generation = ++actionsGeneration.current;
    setBusy(true); setError(null);
    try { await api.action(current, kind); }
    catch (cause) { if (mounted.current && generation === actionsGeneration.current) setError(pinterestError(cause)); }
    finally { if (mounted.current && generation === actionsGeneration.current) setBusy(false); }
  };
  const importPin = async () => {
    if (!snapshot?.currentPinId || busy || snapshot.loading || importingRef.current || epoch.current !== snapshot.epoch) return;
    const ticket = ++importRequest.current, current = snapshot.epoch;
    importingRef.current = true; setImporting(true); setError(null);
    try {
      const id = await api.select(current, snapshot.currentPinId);
      if (!mounted.current || ticket !== importRequest.current || epoch.current !== current || !currentVisible.current) return;
      await hide(current);
      await onImport(id);
    } catch (cause) { if (mounted.current && ticket === importRequest.current) setError(pinterestError(cause)); }
    finally { importingRef.current = false; if (mounted.current) setImporting(false); }
  };
  const returning = () => { importRequest.current++; if (epoch.current !== null) void hide(epoch.current); onReturn(); };
  return <section className="pinterest-official-search" hidden={!active} aria-label="Pinterest 공식 검색">
    <div className="pinterest-official-tools">
      <button type="button" aria-label="공식 검색 뒤로" title="뒤로" disabled={!snapshot || busy} onClick={() => void action("back")}>‹</button>
      <button type="button" aria-label="공식 검색 앞으로" title="앞으로" disabled={!snapshot || busy} onClick={() => void action("forward")}>›</button>
      <button type="button" aria-label="공식 검색 새로고침" title="새로고침" disabled={busy} onClick={() => snapshot ? void action("reload") : setAttempt(n => n + 1)}>↻</button>
      <span className="pinterest-official-status" role="status">{busy || snapshot?.loading ? <><span className="spinner" />불러오는 중</> : "Pinterest 공식 검색"}</span>
      <button type="button" className="secondary-button" disabled={!snapshot?.loading} onClick={() => void action("stop")}>중지</button>
      <button type="button" className="primary-button" disabled={!snapshot?.currentPinId || busy || snapshot.loading || importing || privacyMode} onClick={() => void importPin()}>{importing ? <span className="spinner" /> : null}핀 가져오기</button>
      <button type="button" className="secondary-button" onClick={returning}>목록 보기</button>
    </div>
    <p className="pinterest-official-help">핀을 연 뒤 ‘핀 가져오기’를 누르세요. 로그인은 Pinterest 전용 세션에서 직접 진행하며, 공개 이미지 핀만 저장할 수 있습니다.</p>
    {error || snapshot?.error ? <div className="pinterest-message is-error" role="alert">{error ?? snapshot?.error}</div> : null}
    <div ref={surface} className="pinterest-official-surface" aria-label="공식 검색 화면">
      {privacyMode ? <span>프라이버시 모드</span> : busy && !snapshot ? <span className="spinner" role="status" aria-label="공식 창 여는 중" /> : null}
    </div>
  </section>;
}
