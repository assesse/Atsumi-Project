import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { createMultiviewApi, emptyMultiview, multiviewEntries, type MultiviewApi, type MultiviewPane, type MultiviewSnapshot } from "../../api/multiview";
import { createOfficialBrowserApi, emptyOfficialBrowserSnapshot, hiddenOfficialBrowserViewport, type OfficialBrowserApi, type OfficialBrowserViewport } from "../../api/officialBrowser";
import { hasNativeOverlay, measureOfficialBrowserViewport, nativeModalOcclusion } from "./OfficialBrowserPanel";
import { ConnectionSetup, accountStatus, type ConnectionMode } from "./ConnectionSetup";
import { FluentIcon } from "../../components/FluentIcon";
import { observeNativeOverlayGeometry } from "./nativeOverlayGeometry";
import "./MadoWorkspace.css";

type MadoLayout = { version: 1; direction: "horizontal" | "vertical"; horizontal: number; vertical: number };
const layoutKey = "atsumi.mado.layout.v1";
const defaultLayout: MadoLayout = { version: 1, direction: "horizontal", horizontal: 60, vertical: 50 };
const boundedRatio = (value: number) => Math.min(76, Math.max(24, Math.round(value)));
export function readMadoLayout(): MadoLayout {
  try {
    const value = JSON.parse(localStorage.getItem(layoutKey) ?? "null") as Partial<MadoLayout> | null;
    if (value?.version === 1 && ["horizontal", "vertical"].includes(value.direction ?? "") && typeof value.horizontal === "number" && Number.isFinite(value.horizontal) && typeof value.vertical === "number" && Number.isFinite(value.vertical))
      return { version: 1, direction: value.direction!, horizontal: boundedRatio(value.horizontal), vertical: boundedRatio(value.vertical) };
  } catch { /* Layout preferences are optional, never account or channel state. */ }
  return { ...defaultLayout };
}
const isRecording = (pane: MultiviewPane) => !!pane.recordingId || ["armed", "starting", "recording", "stopping"].includes(pane.recordingStatus ?? "");
const channelName = (pane: MultiviewPane | undefined) => {
  const value = pane?.channelName?.trim();
  return value && value.length <= 160 && !/[\u0000-\u001f\u007f]/.test(value) ? value : "방송";
};
const chatWarning = (pane: MultiviewPane | undefined) => {
  const status = pane?.chatStatus;
  const message = status === "storage_failed" ? "채팅 저장 실패 · 일부 채팅이 누락되었을 수 있습니다."
    : ["failed", "unavailable", "disconnected", "observer_unavailable", "queue_overflow", "unsupported_frame", "frame_too_large", "invalid_frame", "message_too_large", "message_truncated", "decode_failed", "observer_overflow", "connection_gap", "partial"].includes(status ?? "") ? "채팅 일부 누락 가능 · 채팅 기록을 확인해 주세요." : "";
  if (!message) return "";
  const count = pane?.chatCount;
  return message + (typeof count === "number" && Number.isSafeInteger(count) && count >= 0 ? ` 현재 ${count.toLocaleString("ko-KR")}개 기록.` : "");
};

function Pane({ pane, epoch, api, privacy, onError }: { pane: MultiviewPane; epoch: number; api: MultiviewApi; privacy: boolean; onError(message: string): void }) {
  const container = useRef<HTMLDivElement>(null);
  const report = useRef(onError); report.current = onError;
  useEffect(() => {
    const element = container.current;
    if (!element || api.runtime !== "tauri") return;
    let disposed = false, frame = 0, latest: { viewport: OfficialBrowserViewport; revision: number } | null = null, lastKey = "", revision = 0;
    let visibleFlights = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const hidden = { ...hiddenOfficialBrowserViewport, epoch };
    const send = async (viewport: OfficialBrowserViewport, current: number, attempt = 0) => {
      if (disposed) return;
      clearTimeout(retry);
      retry = undefined;
      const serialVisible = viewport.visible && !viewport.occluded;
      if (serialVisible) visibleFlights++;
      try {
        const result = await api.setViewport(pane.paneId, viewport);
        if (!disposed && current === revision && !result.ok) {
          lastKey = "";
          if (viewport.occluded) void api.setViewport(pane.paneId, hidden).catch(() => {});
        }
        if (!disposed && current === revision && !result.ok && !["VIEWPORT_STALE", "MULTIVIEW_STALE"].includes(result.error.code)) {
          if (viewport.visible && ["VIEWPORT_BUSY", "MULTIVIEW_BUSY", "VIEWPORT_CLIP_FAILED"].includes(result.error.code) && attempt < 2) {
            retry = setTimeout(() => { if (!disposed && current === revision) void send(viewport, current, attempt + 1); }, attempt ? 320 : 120);
          } else report.current(result.error.message);
        }
      } catch {
        if (!disposed && current === revision) {
          lastKey = "";
          if (viewport.occluded) void api.setViewport(pane.paneId, hidden).catch(() => {});
          report.current("시청 영역을 표시하지 못했습니다. 다시 적용해 주세요.");
        }
      }
      finally {
        if (serialVisible) {
          visibleFlights--;
          if (!disposed && latest) {
            const next = latest; latest = null;
            if (next.revision === revision) void send(next.viewport, next.revision);
          }
        }
      }
    };
    const update = () => {
      if (disposed) return;
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      const measured = privacy || document.hidden || document.fullscreenElement ? hidden : { ...measureOfficialBrowserViewport(element), epoch };
      const viewport = { ...measured, ...(measured.visible ? nativeModalOcclusion(element, measured) : { occluded: false }) };
      const key = JSON.stringify(viewport);
      if (key === lastKey) return;
      lastKey = key;
      const current = ++revision;
      clearTimeout(retry); retry = undefined;
      if (!viewport.visible || viewport.occluded) { latest = null; void send(viewport, current); }
      else if (visibleFlights) latest = { viewport, revision: current };
      else void send(viewport, current);
    };
    const schedule = () => { if (!disposed && !frame) frame = requestAnimationFrame(update); };
    const mutation = new MutationObserver(() => { if (hasNativeOverlay()) update(); else schedule(); });
    mutation.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["open", "hidden", "aria-hidden", "aria-modal", "class", "style", "data-native-overlay"] });
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    resize?.observe(element);
    const stopOverlayGeometry = observeNativeOverlayGeometry(update);
    window.addEventListener("resize", schedule); document.addEventListener("scroll", schedule, true); document.addEventListener("visibilitychange", update); document.addEventListener("fullscreenchange", update);
    update();
    return () => {
      disposed = true; revision++; latest = null; clearTimeout(retry); cancelAnimationFrame(frame); resize?.disconnect(); stopOverlayGeometry(); mutation.disconnect();
      window.removeEventListener("resize", schedule); document.removeEventListener("scroll", schedule, true); document.removeEventListener("visibilitychange", update); document.removeEventListener("fullscreenchange", update);
      void api.setViewport(pane.paneId, hidden).catch(() => {});
    };
  }, [api, epoch, pane.paneId, privacy]);
  return <div className={`mado-native-slot is-${pane.kind}`} ref={container} aria-label={pane.kind === "video" ? "방송 화면" : "채팅 화면"}>
    <span>{privacy ? "프라이버시 모드" : pane.kind === "video" ? "방송 연결 중" : "채팅 연결 중"}</span>
  </div>;
}

export function MadoWorkspace({ runtime, privacy, onLeave, api: suppliedApi, officialApi: suppliedOfficialApi }: { runtime: MultiviewApi["runtime"]; privacy: boolean; onLeave(): void; api?: MultiviewApi; officialApi?: OfficialBrowserApi }) {
  const api = useMemo(() => suppliedApi ?? createMultiviewApi(runtime), [runtime, suppliedApi]);
  const officialApi = useMemo(() => suppliedOfficialApi ?? createOfficialBrowserApi(runtime), [runtime, suppliedOfficialApi]);
  const [account, setAccount] = useState(() => emptyOfficialBrowserSnapshot(runtime));
  const [authRefreshing, setAuthRefreshing] = useState(false);
  const [connectionMode, setConnectionMode] = useState<ConnectionMode>("mado");
  const [inputs, setInputs] = useState(["", "", "", ""]);
  const [mode, setMode] = useState<"paired" | "chats">("paired");
  const [appliedMode, setAppliedMode] = useState<"paired" | "chats">("paired");
  const [lead, setLead] = useState(0);
  const [snapshot, setSnapshot] = useState(emptyMultiview);
  const [settings, setSettings] = useState(true);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const [layout, setLayout] = useState(readMadoLayout);
  const [resizing, setResizing] = useState(false);
  const captureChat = account.captureChatEnabled !== false;
  const [expiredControl, setExpiredControl] = useState<string | null>(null);
  const [dismissedControl, setDismissedControl] = useState<string | null>(null);
  const surfaces = useRef<HTMLDivElement>(null), cancelControl = useRef<HTMLButtonElement>(null);
  const settingsClose = useRef<HTMLButtonElement>(null);
  const settingsPriorFocus = useRef<HTMLElement | null>(null);
  const settingsWasOpen = useRef(false);
  const drag = useRef<{ pointerId: number; direction: MadoLayout["direction"]; bounds: DOMRect } | null>(null);
  const dragFrame = useRef(0), nextRatio = useRef<{ value: number; direction: MadoLayout["direction"] } | null>(null);
  const seenUiAction = useRef<string | null>(null);
  const snapshotRef = useRef(snapshot); snapshotRef.current = snapshot;
  const lifetime = useRef(0), inFlight = useRef(false), requestRevision = useRef(0), edited = useRef(false);
  const activeRecording = snapshot.panes.some(isRecording);
  const requestedControl = snapshot.pendingControl;
  const controlPane = snapshot.panes.find((pane) => pane.paneId === requestedControl?.paneId && pane.channelId === requestedControl.channelId && pane.kind === "video");
  const control = api.runtime === "tauri" && requestedControl && controlPane && requestedControl.action === "screenshot"
    && requestedControl.id !== expiredControl && requestedControl.id !== dismissedControl && Number.isFinite(requestedControl.expiresAt) && requestedControl.expiresAt > Date.now() ? requestedControl : null;
  const canApprove = !!control && !pending && (control.action === "record_stop" ? isRecording(controlPane!) && controlPane!.recordingStatus !== "stopping"
    : !pollError && controlPane!.ready === true && (control.action !== "record_start" || !isRecording(controlPane!)));
  useEffect(() => {
    const open = settings && !privacy && !control;
    if (open) {
      if (!settingsWasOpen.current) settingsPriorFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      settingsClose.current?.focus();
    } else if (settingsWasOpen.current && !control && settingsPriorFocus.current?.isConnected) settingsPriorFocus.current.focus();
    settingsWasOpen.current = open;
  }, [settings, privacy, control?.id]);
  useEffect(() => {
    if (!settings || runtime !== "tauri") return;
    let disposed = false, refreshSequence = 0;
    const refresh = async () => {
      if (inFlight.current) return;
      const revision = requestRevision.current, sequence = ++refreshSequence;
      const current = () => !disposed && sequence === refreshSequence && revision === requestRevision.current && !inFlight.current;
      try {
        const result = await officialApi.snapshot();
        if (!current()) return;
        if (result.ok && result.data) setAccount(result.data);
        else setAccount((previous) => ({ ...previous, authStatus: "unknown", authChecking: false, authError: result.ok ? "로그인 상태 응답이 없습니다." : result.error.message }));
      } catch {
        if (current()) setAccount((previous) => ({ ...previous, authStatus: "unknown", authChecking: false, authError: "로그인 상태를 확인하지 못했습니다. 다시 확인해 주세요." }));
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 2000);
    return () => { disposed = true; clearInterval(timer); };
  }, [officialApi, runtime, settings]);
  useEffect(() => {
    const timer = setTimeout(() => { try { localStorage.setItem(layoutKey, JSON.stringify(layout)); } catch { /* Optional local layout preference. */ } }, 200);
    return () => clearTimeout(timer);
  }, [layout]);
  useEffect(() => () => { cancelAnimationFrame(dragFrame.current); drag.current = null; nextRatio.current = null; }, []);
  useEffect(() => {
    const generation = ++lifetime.current;
    inFlight.current = false; setPending(false); setError(null); setPollError(null);
    let timer: ReturnType<typeof setTimeout> | undefined, cancelled = false, first = true;
    const poll = async () => {
      const revision = requestRevision.current;
      try {
        if (!inFlight.current) {
          const result = await api.snapshot();
          if (cancelled || lifetime.current !== generation || requestRevision.current !== revision || inFlight.current) return;
          if (!result.ok) { setPollError(result.error.message); return; }
          setPollError(null); setSnapshot(result.data); snapshotRef.current = result.data;
          if (first && result.data.active) {
            const channels = [...new Set(result.data.panes.map((pane) => pane.channelId))];
            const videos = result.data.panes.filter((pane) => pane.kind === "video");
            const restoredMode = videos.length === 1 && channels.length > 1 ? "chats" : "paired";
            setAppliedMode(restoredMode);
            if (!edited.current) {
              setSettings(false); setMode(restoredMode);
              setInputs(Array.from({ length: 4 }, (_, i) => channels[i] ?? ""));
              setLead(Math.max(0, channels.indexOf(videos[0]?.channelId ?? "")));
            }
          }
          first = false;
        }
      } catch { if (!cancelled && generation === lifetime.current && revision === requestRevision.current) setPollError("마도 상태를 확인하지 못했습니다. 다시 확인하고 있습니다."); }
      finally { if (!cancelled) timer = setTimeout(() => void poll(), 2000); }
    };
    if (api.runtime === "tauri") void poll();
    // Each Pane hides on unmount. Closing here would interrupt its recording.
    return () => { cancelled = true; clearTimeout(timer); lifetime.current++; };
  }, [api]);
  useEffect(() => {
    if (!requestedControl) return;
    const remaining = requestedControl.expiresAt - Date.now();
    if (!Number.isFinite(remaining) || remaining <= 0) { setExpiredControl(requestedControl.id); return; }
    const timer = setTimeout(() => setExpiredControl(requestedControl.id), Math.min(remaining, 2147483647));
    return () => clearTimeout(timer);
  }, [requestedControl?.id, requestedControl?.expiresAt]);
  useEffect(() => {
    if (control) cancelControl.current?.focus();
  }, [control?.id]);
  useEffect(() => {
    const intent = snapshot.pendingUiAction;
    if (!intent || seenUiAction.current === intent.id) return;
    seenUiAction.current = intent.id;
    void api.ackUiAction(intent.paneId, intent.id, snapshot.epoch).catch(() => {});
  }, [api, snapshot.pendingUiAction?.id, snapshot.epoch]);
  const run = async (action: (isCurrent: () => boolean) => Promise<void>) => {
    if (inFlight.current) return;
    requestRevision.current++;
    const generation = lifetime.current;
    const isCurrent = () => generation === lifetime.current;
    inFlight.current = true; setPending(true); setError(null);
    try { await action(isCurrent); } catch { if (isCurrent()) setError("요청을 처리하지 못했습니다. 다시 시도해 주세요."); }
    finally { if (isCurrent()) { inFlight.current = false; setPending(false); } }
  };
  const apply = () => void run(async () => {
    if (activeRecording) { setError("녹화를 먼저 중지해 주세요."); return; }
    if (connectionMode === "general") {
      const entries = multiviewEntries([inputs[0] ?? ""], "paired", 0);
      if (typeof entries === "string") { setError(entries); return; }
      const generation = lifetime.current;
      if (snapshot.active) {
        const closed = await api.close(snapshot.epoch);
        if (generation !== lifetime.current) return;
        if (!closed.ok) { setError(closed.error.message); return; }
        setSnapshot(closed.data); snapshotRef.current = closed.data;
      }
      const opened = await officialApi.open(entries[0]!.channelId);
      if (generation !== lifetime.current) return;
      if (!opened.ok) { setError(opened.error.message); return; }
      onLeave();
      return;
    }
    const entries = multiviewEntries(inputs, mode, lead);
    if (typeof entries === "string") { setError(entries); return; }
    const generation = lifetime.current;
    const result = await api.configure(entries);
    if (generation !== lifetime.current) { if (result.ok) void api.close(result.data.epoch).catch(() => {}); return; }
    if (!result.ok) { setError(result.error.message); return; }
    setSnapshot(result.data); snapshotRef.current = result.data; setAppliedMode(mode); setSettings(false);
  });
  const leave = () => void run(async (isCurrent) => {
    if (activeRecording) { setError("녹화를 먼저 중지해 주세요."); return; }
    if (snapshot.active) { const result = await api.close(snapshot.epoch); if (!isCurrent()) return; if (!result.ok) { setError(result.error.message); return; } snapshotRef.current = result.data; setSnapshot(result.data); }
    if (!isCurrent()) return;
    onLeave();
  });
  const confirm = (approve: boolean) => {
    if (!control || control.expiresAt <= Date.now() || (approve && !canApprove)) return;
    const intent = control;
    void run(async (isCurrent) => {
      const result = await api.confirmControl({ paneId: intent.paneId, requestId: intent.id, approve, rightsAcknowledged: approve, captureChat, epoch: snapshot.epoch });
      if (!isCurrent()) return;
      if (result.ok) { setDismissedControl(intent.id); setSnapshot(result.data); snapshotRef.current = result.data; }
      else setError(result.error.message);
    });
  };
  const controlKeys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" && !pending) { event.preventDefault(); event.stopPropagation(); confirm(false); }
    if (event.key !== "Tab") return;
    const elements = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled)')];
    const first = elements[0], last = elements.at(-1);
    if (!first || !last) { event.preventDefault(); return; }
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  };
  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => {
      if (api.runtime !== "tauri" || privacy || pending || pollError || control || hasNativeOverlay() || event.key.toLowerCase() !== "s" || event.defaultPrevented || event.repeat || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="dialog"], [role="alertdialog"]')) return;
      const videos = snapshot.panes.filter((pane) => pane.kind === "video");
      // Multiple native players handle S in their own focused document. The
      // app-level shortcut is unambiguous only when a single video is shown.
      if (videos.length !== 1) return;
      const pane = videos[0];
      if (!pane?.ready) return;
      event.preventDefault();
      void run(async (isCurrent) => {
        const result = await api.requestControl(pane.paneId, "screenshot", snapshot.epoch);
        if (!isCurrent()) return;
        if (result.ok) setSnapshot(result.data); else setError(result.error.message);
      });
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [api, privacy, pending, pollError, control?.id, snapshot]);
  const updateRatio = (value: number, direction = layout.direction) => setLayout((current) => ({ ...current, [direction]: boundedRatio(value) }));
  const moveDivider = (event: ReactPointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || event.pointerId !== current.pointerId) return;
    const size = current.direction === "horizontal" ? current.bounds.width : current.bounds.height;
    const offset = current.direction === "horizontal" ? event.clientX - current.bounds.left : event.clientY - current.bounds.top;
    if (size > 0) {
      nextRatio.current = { value: offset / size * 100, direction: current.direction };
      if (!dragFrame.current) dragFrame.current = requestAnimationFrame(() => { dragFrame.current = 0; const next = nextRatio.current; nextRatio.current = null; if (next) updateRatio(next.value, next.direction); });
    }
  };
  const endDivider = () => { cancelAnimationFrame(dragFrame.current); dragFrame.current = 0; const next = nextRatio.current; nextRatio.current = null; if (next) updateRatio(next.value, next.direction); drag.current = null; setResizing(false); };
  const dividerKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const down = layout.direction === "horizontal" ? "ArrowLeft" : "ArrowUp", up = layout.direction === "horizontal" ? "ArrowRight" : "ArrowDown";
    if (![down, up, "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    updateRatio(event.key === "Home" ? 24 : event.key === "End" ? 76 : layout[layout.direction] + (event.key === down ? -1 : 1) * (event.shiftKey ? 10 : 2));
  };
  const channels = [...new Set(snapshot.panes.map((pane) => pane.channelId))];
  const number = (channelId: string) => channels.indexOf(channelId) + 1;
  const label = (channelId: string) => `${number(channelId)}. ${privacy ? "방송" : channelName(snapshot.panes.find((pane) => pane.channelId === channelId))}`;
  const renderPane = (pane: MultiviewPane) => <Pane key={pane.paneId} pane={pane} epoch={snapshot.epoch} api={api} privacy={privacy || resizing} onError={setError} />;
  const warnings = snapshot.panes.filter(pane => pane.kind === "video").flatMap(pane => {
    const message = [pane.error, chatWarning(pane)].filter(Boolean).join(" · ");
    return message ? [{ paneId: pane.paneId, label: label(pane.channelId), message }] : [];
  });
  return <section className="mado-workspace" aria-label="마도 모드">
    <header className="mado-toolbar"><strong title="마법도서관 · 최대 네 방송의 영상과 채팅을 배치합니다">마도</strong>
      <button type="button" onClick={() => { setConnectionMode("mado"); setSettings(true); }}>연결 설정</button>
      {appliedMode === "chats" && snapshot.active ? <button type="button" onClick={() => setLayout((current) => ({ ...current, direction: current.direction === "horizontal" ? "vertical" : "horizontal" }))} title="영상과 채팅의 배치 방향을 바꿉니다. 분할선은 끌거나 방향키로 조절할 수 있습니다.">{layout.direction === "horizontal" ? "위아래 배치" : "좌우 배치"}</button> : null}
      {!privacy && warnings.length ? <div className="mado-notices">{warnings.map(warning => <span key={warning.paneId} className="mado-chat-warning" role="alert" tabIndex={0} title={`${warning.label} · ${warning.message}`} aria-label={`${warning.label} · ${warning.message}`}>⚠ {warning.label} · 확인 필요</span>)}</div> : null}
    </header>
    {settings && !privacy && !control ? <div className="official-browser-dialog-backdrop" role="dialog" aria-modal="true" data-native-preserve-video="true" aria-label="연결 설정" onKeyDown={(event) => {
      if (event.key === "Escape") { event.preventDefault(); setSettings(false); }
      if (event.key === "Tab") { const elements = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled)')]; const first = elements[0], last = elements.at(-1); if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); } else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); } }
    }}><ConnectionSetup mode={connectionMode} onMode={setConnectionMode} modeDisabled={pending || activeRecording}
      inputs={inputs} onInput={(index, value) => { edited.current = true; setInputs((rows) => rows.map((row, i) => i === index ? value : row)); }} disabled={pending || activeRecording || runtime !== "tauri"} pending={pending} onConnect={apply} onClose={() => setSettings(false)} closeRef={settingsClose}
      auth={accountStatus(account)} accountDisabled={pending || activeRecording || account.accountBusy === true || runtime !== "tauri"}
      accountReason={activeRecording ? "녹화를 중지한 뒤 방송·모드·계정을 변경할 수 있습니다." : undefined}
      onLogin={() => void run(async () => { const result = await officialApi.login(); if (result.ok) setAccount(result.data); else setError(result.error.message); })}
      onLogout={() => void run(async () => { const result = await officialApi.logout(); if (result.ok) setAccount(result.data); else setError(result.error.message); })}
      authChecking={authRefreshing || account.authChecking} authError={account.authError} refreshDisabled={pending || account.accountBusy === true || runtime !== "tauri"}
      onRefresh={() => void run(async () => { setAuthRefreshing(true); try { const result = await officialApi.snapshot(true); if (result.ok) setAccount(result.data); else setError(result.error.message); } finally { setAuthRefreshing(false); } })}
      onGrid={() => void run(async () => { const result = await officialApi.connectExtension(); if (result.ok) setAccount(result.data); else setError(result.error.message); })}
      gridDisabled={pending || activeRecording || snapshot.active || runtime !== "tauri" || !account.windowOpen} gridStatus={account.extensionStatus === "not_connected" ? "미연결" : account.extensionStatus ?? "상태 확인 전"}
      onInstaller={(browser) => void run(async () => { const result = await officialApi.openInstaller(browser); if (!result.ok) setError(result.error.message); })} installerDisabled={pending || activeRecording || runtime !== "tauri"}
      options={<><div className="mado-layout-options"><button type="button" aria-pressed={mode === "paired"} onClick={() => { edited.current = true; setMode("paired"); }}>4화면4챗</button><button type="button" aria-pressed={mode === "chats"} onClick={() => { edited.current = true; setMode("chats"); }}>1화면4챗</button></div>{mode === "chats" ? <label className="mado-broadcast-select">
        <span>영상 방송</span>
        <span className="mado-broadcast-select-control">
          <select value={lead} onChange={(event) => { edited.current = true; setLead(Number(event.target.value)); }}>{inputs.map((_, index) => <option key={index} value={index}>방송 {index + 1}</option>)}</select>
          <FluentIcon glyph="\uE70D" />
        </span>
      </label> : null}</>}
    /></div> : null}
    {(error || pollError) && !control ? <div className="mado-error" role="alert" data-native-overlay="true"><span>{error || pollError}</span>{error ? <button type="button" aria-label="오류 닫기" onClick={() => setError(null)}>×</button> : null}</div> : null}
    {snapshot.active ? <div ref={surfaces} className={`mado-surfaces is-${appliedMode} direction-${layout.direction}${resizing ? " is-resizing" : ""}`} data-channels={channels.length} style={{ "--mado-main-share": `${layout[layout.direction]}fr`, "--mado-chat-share": `${100 - layout[layout.direction]}fr` } as CSSProperties}>
      {appliedMode === "paired" ? channels.map((channelId) => <div className="mado-pair" key={channelId}>{snapshot.panes.filter((pane) => pane.channelId === channelId).map((pane) => pane.kind === "video" ? renderPane(pane) : <div className="mado-chat-cell" key={pane.paneId}>{renderPane(pane)}</div>)}</div>) : <>
        <div className="mado-lead">{snapshot.panes.filter((pane) => pane.kind === "video").map((pane) => <div className="mado-lead-inner" key={pane.paneId}>{renderPane(pane)}</div>)}</div>
        <div className="mado-divider" role="separator" tabIndex={0} aria-label="영상과 채팅 크기" aria-orientation={layout.direction === "horizontal" ? "vertical" : "horizontal"} aria-valuemin={24} aria-valuemax={76} aria-valuenow={layout[layout.direction]} aria-valuetext={`영상 ${layout[layout.direction]}%, 채팅 ${100 - layout[layout.direction]}%`} onKeyDown={dividerKey}
          onPointerDown={(event) => { if (event.button !== 0 || !surfaces.current) return; event.preventDefault(); drag.current = { pointerId: event.pointerId, direction: layout.direction, bounds: surfaces.current.getBoundingClientRect() }; event.currentTarget.setPointerCapture?.(event.pointerId); setResizing(true); }}
          onPointerMove={moveDivider} onPointerUp={endDivider} onPointerCancel={endDivider} onLostPointerCapture={endDivider} />
        <div className="mado-chat-wall">{snapshot.panes.filter((pane) => pane.kind === "chat").map((pane) => <div className="mado-chat-cell" key={pane.paneId}>{renderPane(pane)}</div>)}</div>
      </>}
    </div> : <p className="mado-empty">방송 주소를 입력하고 원하는 배치를 적용해 주세요.</p>}
    {control ? <div className="official-browser-dialog-backdrop" role="alertdialog" aria-modal="true" data-native-preserve-video="true" aria-labelledby="mado-control-title" onKeyDown={controlKeys}>
      <div className="official-browser-dialog" data-native-dialog-surface="true"><h3 id="mado-control-title">화면을 저장할까요?</h3><p>{label(control.channelId)} · 스크린샷을 저장합니다.</p>
        {error || pollError ? <p role="alert" className="official-browser-error">{error || pollError}</p> : null}
        <div><button ref={cancelControl} type="button" disabled={pending} onClick={() => confirm(false)}>취소</button><button type="button" disabled={!canApprove} onClick={() => confirm(true)}>{pending ? "처리 중…" : "저장 확인"}</button></div>
      </div>
    </div> : null}
  </section>;
}
