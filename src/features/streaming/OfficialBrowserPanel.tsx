import { lazy, Suspense, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { ApiResult } from "../../api/contracts";
import {
  claimOfficialBrowserViewport, createOfficialBrowserApi, emptyOfficialBrowserSnapshot, hiddenOfficialBrowserViewport,
  type BrowserRecording, type OfficialBrowserApi, type OfficialBrowserSnapshot, type OfficialBrowserViewport,
} from "../../api/officialBrowser";
import "./OfficialBrowserPanel.css";
import { RecordingLibrary } from "./RecordingLibrary";
import { emptyRecordingAttempt, recordingStatus } from "./recordingStatus";
import { hasNativeOverlay, nativeModalOcclusion, observeNativeOverlayGeometry } from "./nativeOverlayGeometry";
export { hasNativeOverlay, nativeModalOcclusion } from "./nativeOverlayGeometry";
// The full, locally preserved player/chat skin is only needed when opening a recording.
const RecordingReplay = lazy(() => import("./RecordingReplay").then(module => ({ default: module.RecordingReplay })));
import type { ReplayApi } from "../../api/replay";
import { createMultiviewApi, multiviewEntries, type MultiviewApi } from "../../api/multiview";
import { ConnectionSetup, accountStatus, type ConnectionMode } from "./ConnectionSetup";

type PendingAction = "open" | "mado" | "start" | "stop" | "extension" | "folder" | "segment" | "login" | "logout" | "auth-refresh" | "installer" | "merge" | "merged" | "control" | "request-control" | "delete" | "record-only";

/** Focus/touch accessible help. Native child paint must yield to its popup. */
function Help({ label, text }: { label: string; text: string }) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const anchor = useRef<DOMRect | null>(null);
  const tooltip = useRef<HTMLSpanElement>(null);
  const [position, setPosition] = useState({ left: 16, top: 16 });
  const [overlapsStage, setOverlapsStage] = useState(false);
  const show = (element: HTMLElement) => {
    const rect = element.getBoundingClientRect();
    anchor.current = rect;
    setPosition({ left: Math.max(16, Math.min(rect.left, window.innerWidth - 376)), top: Math.max(16, rect.bottom + 8) });
    setOpen(true);
  };
  useLayoutEffect(() => {
    if (!open || !tooltip.current || !anchor.current) return;
    const bounds = tooltip.current.getBoundingClientRect();
    const top = Math.max(16, anchor.current.top - bounds.height - 8);
    if (position.top !== top) { setPosition((current) => ({ ...current, top })); return; }
    const stage = tooltip.current.closest(".official-browser-panel")?.querySelector(".official-browser-stage")?.getBoundingClientRect();
    setOverlapsStage(!!stage && bounds.right > stage.left && bounds.left < stage.right && bounds.bottom > stage.top && bounds.top < stage.bottom);
  }, [open, position, text]);
  useEffect(() => {
    if (!open) return;
    const close = (event: Event) => { if (!(event.target instanceof Node) || !tooltip.current?.contains(event.target)) setOpen(false); };
    document.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => { document.removeEventListener("scroll", close, true); window.removeEventListener("resize", close); };
  }, [open]);
  return <span className="official-browser-help" onMouseEnter={(event) => show(event.currentTarget)}
    onMouseLeave={(event) => { if (!event.currentTarget.contains(document.activeElement)) setOpen(false); }}
    onFocus={(event) => show(event.currentTarget)} onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false); }}
    onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); setOpen(false); } }}>
    <button type="button" className="official-browser-help-trigger" aria-label={`${label} 도움말`} aria-describedby={open ? id : undefined} onClick={(event) => show(event.currentTarget)}>?</button>
    {open ? <span ref={tooltip} id={id} role="tooltip" className="official-browser-tooltip" style={position} data-native-overlay={overlapsStage}>{text}</span> : null}
  </span>;
}

export type OfficialBrowserPanelProps = {
  runtime: OfficialBrowserApi["runtime"];
  active: boolean;
  view: "live" | "recordings";
  privacyMode?: boolean;
  api?: OfficialBrowserApi;
  replayApi?: ReplayApi;
  onMadoMode?: () => void;
  multiviewApi?: MultiviewApi;
  /** An attached receiver cannot be navigated/reconnected underneath capture. */
  connectionLocked?: boolean;
  onRecordOnly?: () => void | Promise<void>;
};


/** Preserve the site's viewport size while clipping native paint to scroll containers. */
export function measureOfficialBrowserViewport(stage: HTMLElement): OfficialBrowserViewport {
  const rect = stage.getBoundingClientRect();
  if (![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) || rect.width < 1 || rect.height < 1) return hiddenOfficialBrowserViewport;
  let left = Math.max(0, rect.left), top = Math.max(0, rect.top);
  let right = Math.min(window.innerWidth, rect.right), bottom = Math.min(window.innerHeight, rect.bottom);
  for (let parent = stage.parentElement; parent; parent = parent.parentElement) {
    const style = getComputedStyle(parent);
    if (style.display === "none" || style.visibility === "hidden") return hiddenOfficialBrowserViewport;
    const bounds = parent.getBoundingClientRect();
    if (/(auto|scroll|hidden|clip)/.test(style.overflowX || style.overflow)) {
      left = Math.max(left, bounds.left + parent.clientLeft);
      right = Math.min(right, bounds.left + parent.clientLeft + parent.clientWidth);
    }
    if (/(auto|scroll|hidden|clip)/.test(style.overflowY || style.overflow)) {
      top = Math.max(top, bounds.top + parent.clientTop);
      bottom = Math.min(bottom, bounds.top + parent.clientTop + parent.clientHeight);
    }
  }
  if (right <= left || bottom <= top) return hiddenOfficialBrowserViewport;
  const snap = (value: number) => Math.round(value * 100) / 100;
  return {
    x: snap(rect.left), y: snap(rect.top), width: snap(rect.width), height: snap(rect.height), visible: true,
    clip: { x: snap(left - rect.left), y: snap(top - rect.top), width: snap(right - left), height: snap(bottom - top) },
  };
}

const chatLabels: Record<string, string> = {
  disabled: "저장 안 함", connecting: "연결 중", connected: "연결됨",
  reconnecting: "재연결 중", stopped: "종료됨", failed: "연결 실패",
  unavailable: "공개 채팅 연결 불가", storage_failed: "기록 저장 오류",
  page_connected: "공식 페이지 수신 기록 중", observing: "공식 채팅 수신 대기",
  waiting_socket: "공식 채팅 수신 대기", receiving: "공식 채팅 수신 중",
  disconnected: "연결 끊김 · 누락 가능", observer_unavailable: "이 환경에서 공식 채팅 관찰 불가",
  queue_overflow: "대기열 초과 · 일부 누락", unsupported_frame: "지원하지 않는 메시지 형식 · 일부 누락",
  frame_too_large: "메시지 묶음 한도 초과 · 일부 누락", invalid_frame: "메시지 해석 실패 · 일부 누락",
  message_too_large: "메시지 크기 한도 초과 · 일부 누락", message_truncated: "긴 메시지 일부 잘림",
  decode_failed: "메시지 해석 실패 · 일부 누락", observer_overflow: "채팅 연결 한도 초과 · 일부 누락",
  connection_gap: "채팅 연결 공백 · 일부 누락", partial: "종료됨 · 일부 누락",
};
const duration = (value: number) => {
  const seconds = Math.max(0, Math.floor(value));
  return `${Math.floor(seconds / 3600).toString().padStart(2, "0")}:${Math.floor(seconds / 60 % 60).toString().padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
};
const bytes = (value: number) => value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GiB` : `${(value / 1024 ** 2).toFixed(1)} MiB`;

function connectionLabel(snapshot: OfficialBrowserSnapshot): string {
  if (snapshot.status === "desktop_required") return "데스크톱 앱 필요";
  if (snapshot.status === "stopping") return "파일 마무리 중";
  if (snapshot.status === "starting") return "녹화 준비 중";
  if (snapshot.status === "waiting_source") return "수신 대기";
  if (snapshot.recordingId || snapshot.status === "recording") return "녹화 중";
  if (snapshot.error || snapshot.status === "error") return "연결 확인 필요";
  if (snapshot.status === "loading") return "공식 플레이어 확인 중";
  if (snapshot.ready && snapshot.windowOpen) return "녹화 준비됨";
  if (snapshot.windowOpen) return "로그인·재생 상태 확인 필요";
  return "시청 창 닫힘";
}

export function OfficialBrowserPanel({ runtime, active, view, privacyMode = false, api: suppliedApi, replayApi, onMadoMode, multiviewApi, connectionLocked = false, onRecordOnly }: OfficialBrowserPanelProps) {
  const api = useMemo(() => suppliedApi ?? createOfficialBrowserApi(runtime), [suppliedApi, runtime]);
  const madoApi = useMemo(() => multiviewApi ?? createMultiviewApi(runtime), [multiviewApi, runtime]);
  const desktop = runtime === "tauri" && api.runtime === "tauri";
  const [snapshot, setSnapshot] = useState(() => emptyOfficialBrowserSnapshot(runtime));
  const [input, setInput] = useState("");
  const [connectionMode, setConnectionMode] = useState<ConnectionMode>("general");
  const [madoInputs, setMadoInputs] = useState(["", "", "", ""]);
  const [madoLayout, setMadoLayout] = useState<"paired" | "chats">("paired");
  const [madoLead, setMadoLead] = useState(0);
  const [captureChat, setCaptureChat] = useState(true);
  useEffect(() => { if (snapshot.captureChatEnabled !== undefined) setCaptureChat(snapshot.captureChatEnabled); }, [snapshot.captureChatEnabled]);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [replayId, setReplayId] = useState<string | null>(null);
  const [viewportError, setViewportError] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsClose = useRef<HTMLButtonElement>(null);
  const [nativeVisible, setNativeVisible] = useState(false);
  const [dismissedControl, setDismissedControl] = useState<string | null>(null);
  const [expiredControl, setExpiredControl] = useState<string | null>(null);
  const [dismissedFocusError, setDismissedFocusError] = useState("");
  const seenUiAction = useRef<string | null>(null);
  const stage = useRef<HTMLDivElement>(null);
  const controlCancel = useRef<HTMLButtonElement>(null);
  const controlWasOpen = useRef<string | null>(null);
  const controlPriorFocus = useRef<HTMLElement | null>(null);
  const lifetime = useRef(0);
  const requestVersion = useRef(0);
  const actionInFlight = useRef(false);
  const recording = snapshot.recordings.find((item) => item.id === snapshot.recordingId)
    ?? snapshot.recordings.find((item) => item.status === "recording");
  const hasRecording = !!snapshot.recordingId || snapshot.status === "recording" || snapshot.status === "stopping" || snapshot.status === "starting";
  const recordings = useMemo(() => [...snapshot.recordings].sort((left, right) => right.startedAt - left.startedAt), [snapshot.recordings]);
  const selected = recordings.find((item) => item.id === selectedId) ?? recording ?? recordings[0];
  const extensionBusy = ["connecting", "연결 중"].includes(snapshot.extensionStatus ?? "");
  const extensionLabel = snapshot.extensionStatus === "not_connected" || !snapshot.extensionStatus ? "미연결" : snapshot.extensionStatus;
  const extensionSummary = ({ "미연결": "미연결", "연결됨": "연결됨", "connecting": "연결 중", "연결 중": "연결 중", "확장 로드 완료 · 공식 페이지 감지 확인 중": "확인 중", "공식 페이지 확장 감지됨 · 커넥터/고화질은 실제 재생으로 확인": "확장 감지됨" } as Record<string, string>)[extensionLabel] ?? "확인 필요";
  // One predicate owns behavior and visual readiness. Loading an extension is
  // not itself required: ordinary-quality playback can also be recorded.
  const canStart = active && desktop && pending === null && snapshot.ready && snapshot.windowOpen
    && !hasRecording && !pollError && !extensionBusy;
  const requestedControl = snapshot.pendingControl;
  const control = desktop && active && view === "live" && requestedControl
    && requestedControl.id !== dismissedControl && requestedControl.id !== expiredControl
    && requestedControl.action === "screenshot"
    && requestedControl.channelId === snapshot.channelId && Number.isFinite(requestedControl.expiresAt)
    && requestedControl.expiresAt > Date.now() ? requestedControl : null;
  const canApproveControl = !!control && pending === null && snapshot.ready && snapshot.windowOpen && !pollError && !extensionBusy;
  const startHint = !desktop ? "데스크톱 앱에서 녹화할 수 있습니다."
    : hasRecording ? "녹화 중에는 방송·모드를 변경할 수 없습니다."
    : pending === "start" ? "녹화를 준비하고 있습니다."
    : extensionBusy ? "고화질 연결이 끝나면 녹화할 수 있습니다."
    : pollError ? "연결 상태를 다시 확인하고 있습니다."
    : !snapshot.windowOpen ? "시청을 시작한 뒤 녹화할 수 있습니다."
    : !snapshot.ready ? snapshot.error || (snapshot.captureDiagnostics && !["waiting_video", "waiting_tracks", "waiting_init"].includes(snapshot.captureDiagnostics.reason)
      ? "현재 방송의 원본 저장을 지원하지 않습니다. 시청은 계속할 수 있습니다." : "영상이 재생되면 녹화할 수 있습니다.") : null;

  useLayoutEffect(() => {
    if (!desktop) return;
    const owner = claimOfficialBrowserViewport(api, setViewportError);
    // No native surface is attached here. Send the privacy/detach boundary
    // once, without observing the entire document for an invisible player.
    if (!active || view !== "live" || !snapshot.windowOpen || privacyMode) {
      setNativeVisible(false);
      owner.update({ ...hiddenOfficialBrowserViewport, ...(privacyMode ? { suspendAudio: true } : {}), epoch: snapshot.viewportEpoch ?? 0 });
      return () => owner.release();
    }
    let frame: number | undefined;
    let disposed = false;
    const measure = () => {
      if (disposed) return;
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      frame = undefined;
      const visible = active && view === "live" && snapshot.windowOpen && !privacyMode
        && !document.hidden && !document.fullscreenElement;
      const measured = visible && stage.current ? measureOfficialBrowserViewport(stage.current) : hiddenOfficialBrowserViewport;
      // Native main-document reload increments this epoch. An old document's
      // delayed show must not reveal the child over a freshly loading UI.
      const occlusion = measured.visible && stage.current ? nativeModalOcclusion(stage.current, measured) : { occluded: false };
      const viewport = { ...measured, ...occlusion, epoch: snapshot.viewportEpoch ?? 0 };
      setNativeVisible(viewport.visible && (!viewport.occluded || viewport.preserveBackground === true));
      owner.update(viewport);
    };
    const schedule = () => {
      if (!disposed && frame === undefined) frame = window.requestAnimationFrame(measure);
    };
    // Modal/menu insertion masks pixels/input without releasing the viewport
    // lease (a release would briefly hide/suspend the media controller).
    const mutated = () => {
      if (hasNativeOverlay()) measure();
      else schedule();
    };
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    if (stage.current) {
      for (let element: Element | null = stage.current; element; element = element.parentElement) resize?.observe(element);
    }
    const mutation = new MutationObserver(mutated);
    mutation.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["open", "hidden", "aria-hidden", "aria-modal", "class", "style", "data-native-overlay"] });
    const stopOverlayGeometry = observeNativeOverlayGeometry(measure);
    window.addEventListener("resize", schedule);
    document.addEventListener("scroll", schedule, true);
    document.addEventListener("visibilitychange", schedule);
    document.addEventListener("fullscreenchange", schedule);
    measure();
    return () => {
      disposed = true;
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      resize?.disconnect();
      mutation.disconnect();
      stopOverlayGeometry();
      window.removeEventListener("resize", schedule);
      document.removeEventListener("scroll", schedule, true);
      document.removeEventListener("visibilitychange", schedule);
      document.removeEventListener("fullscreenchange", schedule);
      owner.release();
    };
  }, [api, desktop, active, view, snapshot.windowOpen, snapshot.viewportEpoch, privacyMode]);

  useEffect(() => {
    if (!active || view !== "live") { setSettingsOpen(false); }
  }, [active, view]);
  useEffect(() => {
    if (settingsOpen && !control) settingsClose.current?.focus();
  }, [settingsOpen, control?.id]);
  useEffect(() => {
    if (!requestedControl) return;
    const delay = requestedControl.expiresAt - Date.now();
    if (!Number.isFinite(delay) || delay <= 0) { setExpiredControl(requestedControl.id); return; }
    const timer = setTimeout(() => setExpiredControl(requestedControl.id), Math.min(delay, 2_147_483_647));
    return () => clearTimeout(timer);
  }, [requestedControl?.id, requestedControl?.expiresAt]);
  useEffect(() => {
    if (control) {
      if (!controlWasOpen.current) controlPriorFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      controlCancel.current?.focus(); // A queued Enter must never approve a remote intent.
    } else if (controlWasOpen.current && controlPriorFocus.current?.isConnected) controlPriorFocus.current.focus();
    controlWasOpen.current = control?.id ?? null;
  }, [control?.id]);

  useEffect(() => {
    const generation = ++lifetime.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let cancelled = false;
    actionInFlight.current = false;
    setPending(null);
    setPollError(null);
    setActionError(null);
    setMessage(null);
    setSnapshot(emptyOfficialBrowserSnapshot(runtime));

    const poll = async () => {
      const version = requestVersion.current;
      try {
        if (!actionInFlight.current) {
          const result = await api.snapshot();
          if (!cancelled && generation === lifetime.current && version === requestVersion.current && !actionInFlight.current) {
            if (result.ok) { setSnapshot(result.data); setPollError(null); }
            else setPollError(result.error.message);
          }
        }
      } catch {
        if (!cancelled && generation === lifetime.current && version === requestVersion.current && !actionInFlight.current) {
          setPollError("공식 시청 창의 상태를 확인하지 못했습니다. 자동으로 다시 확인합니다.");
        }
      } finally {
        if (!cancelled) timer = setTimeout(() => void poll(), 2000);
      }
    };
    if (active && desktop) void poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
      if (generation === lifetime.current) lifetime.current += 1;
      // Navigation only detaches this observer; recording belongs to the native host.
    };
  }, [api, active, desktop, runtime]);

  const runAction = async <T,>(kind: PendingAction, operation: () => Promise<ApiResult<T>>, onSuccess?: (value: T) => void) => {
    if (!active || !desktop || actionInFlight.current) return;
    actionInFlight.current = true;
    requestVersion.current += 1;
    const generation = lifetime.current;
    setPending(kind);
    setActionError(null);
    setMessage(null);
    try {
      const result = await operation();
      if (generation !== lifetime.current) return;
      if (result.ok) onSuccess?.(result.data);
      else setActionError(result.error.message);
    } catch {
      if (generation === lifetime.current) setActionError("요청을 처리하지 못했습니다. 공식 시청 창의 상태를 확인한 뒤 다시 시도해 주세요.");
    } finally {
      if (generation === lifetime.current) { actionInFlight.current = false; setPending(null); }
    }
  };
  const acceptSnapshot = (next: OfficialBrowserSnapshot) => { setSnapshot(next); setPollError(null); };
  useEffect(() => {
    const intent = snapshot.pendingUiAction;
    if (!active || !desktop || view !== "live" || privacyMode || control || pending !== null || !intent
      || !["open_settings", "record_only"].includes(intent.action) || seenUiAction.current === intent.id) return;
    seenUiAction.current = intent.id;
    if (Number.isFinite(intent.expiresAt) && intent.expiresAt > Date.now()) {
      if (intent.action === "open_settings") { setConnectionMode("general"); setSettingsOpen(true); }
      if (intent.action === "record_only" && onRecordOnly) {
        void runAction("record-only", async () => { await onRecordOnly(); return { ok: true, data: undefined }; });
      }
    }
    // ACKs consume UI-only intents; never replay their possibly stale snapshot.
    void api.ackUiAction(intent.id).catch(() => {});
  }, [api, active, desktop, view, privacyMode, control?.id, pending, snapshot.pendingUiAction?.id]);
  useEffect(() => {
    if (!active || !desktop || view !== "live") return;
    const shortcut = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "s" || event.defaultPrevented || event.repeat || event.isComposing || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey
        || pending !== null || control || privacyMode || hasNativeOverlay() || !snapshot.ready || !snapshot.windowOpen) return;
      const element = event.target instanceof Element ? event.target : null;
      if (element?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="dialog"], [role="alertdialog"]')) return;
      event.preventDefault();
      void runAction("request-control", () => api.requestControl("screenshot"), acceptSnapshot);
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [api, active, desktop, view, pending, control?.id, privacyMode, snapshot.ready, snapshot.windowOpen]);
  const open = () => {
    if (connectionLocked || hasRecording || extensionBusy) return;
    if (connectionMode === "mado") {
      if (!onMadoMode) return;
      const entries = multiviewEntries(madoInputs, madoLayout, madoLead);
      if (typeof entries === "string") { setActionError(entries); return; }
      void runAction("mado", () => madoApi.configure(entries), () => { setSettingsOpen(false); onMadoMode(); });
    } else if (input.trim()) {
      void runAction("open", () => api.open(input.trim()), (next) => { acceptSnapshot(next); if (next.windowOpen) setSettingsOpen(false); });
    }
  };
  const start = () => {
    if (!canStart) return;
    void runAction("start", () => api.start({ rightsAcknowledged: true, captureChat }), (next) => {
      acceptSnapshot(next);
      setSelectedId(next.recordingId);
    });
  };
  const stop = () => {
    if (!hasRecording || snapshot.status === "stopping") return;
    void runAction("stop", () => api.stop(), (next) => {
      acceptSnapshot(next);
    });
  };
  const openFolder = (id: string) => void runAction("folder", () => api.openFolder(id), () => setMessage("녹화 폴더를 열었습니다."));
  const openSegment = (id: string, index: number) => void runAction("segment", () => api.openSegment(id, index));
  const openMerged = (id: string) => void runAction("merged", () => api.openMerged(id));
  const retryMerge = (id: string) => void runAction("merge", () => api.retryMerge(id), acceptSnapshot);
  const deleteRecordings = async (ids: string[]) => {
    if (!active || !desktop || actionInFlight.current) throw new Error("다른 작업이 끝난 뒤 다시 시도해 주세요.");
    actionInFlight.current = true;
    requestVersion.current += 1; // An earlier poll must not resurrect removed cards.
    const generation = lifetime.current;
    setPending("delete"); setActionError(null); setMessage(null);
    try {
      const result = await api.deleteRecordings(ids);
      if (!result.ok) throw new Error(result.error.message);
      if (generation === lifetime.current) {
        const removed = new Set(result.data.deletedIds);
        setSnapshot(previous => ({ ...previous, recordings: previous.recordings.filter(item => !removed.has(item.id)) }));
        setSelectedId(previous => previous && removed.has(previous) ? null : previous);
      }
      return result.data;
    } finally {
      if (generation === lifetime.current) { actionInFlight.current = false; setPending(null); }
    }
  };
  const logout = () => {
    if (hasRecording) return;
    void runAction("logout", () => api.logout(), (next) => {
      acceptSnapshot(next);
      setMessage("공식 시청 프로필에서 로그아웃했습니다. 녹화 파일과 다른 브라우저 계정은 그대로 유지됩니다.");
    });
  };
  const confirmControl = (approve: boolean) => {
    if (!control || pending !== null || control.expiresAt <= Date.now() || (approve && !canApproveControl)) return;
    const requestId = control.id;
    // Only this explicit trusted confirmation acknowledges the notice. A
    // remote player intent or a mode/layout change never approves capture.
    void runAction("control", () => api.confirmControl({ requestId, approve, rightsAcknowledged: approve, captureChat }), (next) => {
      setDismissedControl(requestId);
      acceptSnapshot(next);
    });
  };
  const controlKeys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" && pending !== "control") { event.preventDefault(); event.stopPropagation(); confirmControl(false); }
    if (event.key !== "Tab") return;
    const controls = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled)')];
    const first = controls[0], last = controls[controls.length - 1];
    if (!first || !last) { event.preventDefault(); return; }
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  };
  const settingsKeys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!settingsOpen || event.defaultPrevented) return;
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setSettingsOpen(false); }
    if (event.key !== "Tab") return;
    const controls = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled)')]
      .filter((element) => !element.closest("[hidden]") && (!element.closest("details:not([open])") || element.tagName === "SUMMARY"));
    const first = controls[0], last = controls[controls.length - 1];
    if (!first || !last) { event.preventDefault(); return; }
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  };
  const nativeLoginStatus = snapshot.loginStatus?.trim() ?? "";
  const loginLabel = ({ unknown: "계정 상태는 공식 페이지에서 확인", signed_in: "로그인됨", logged_in: "로그인됨", signed_out: "로그아웃됨", logged_out: "로그아웃됨", login_pending: "로그인·인증 창 열림", opening: "로그인·인증 창 여는 중" } as Record<string, string>)[nativeLoginStatus]
    ?? (nativeLoginStatus.length > 0 && nativeLoginStatus.length <= 200 && !/[\u0000-\u001f\u007f]/.test(nativeLoginStatus) ? nativeLoginStatus : "계정 상태는 공식 페이지에서 확인");
  const videoSize = [snapshot.videoWidth, snapshot.videoHeight].every((value) => typeof value === "number" && Number.isInteger(value) && value > 0 && value <= 16384)
    ? `${snapshot.videoWidth}×${snapshot.videoHeight}` : null;
  const chatProblem = hasRecording && ["failed", "unavailable", "storage_failed", "disconnected", "observer_unavailable", "queue_overflow", "unsupported_frame", "frame_too_large", "invalid_frame", "message_too_large", "message_truncated", "decode_failed", "observer_overflow", "connection_gap", "partial"].includes(snapshot.chatStatus ?? "");
  const recordingDetails = recording
    ? `${privacyMode ? "CHZZK 녹화" : (recording.title || recording.channelId).slice(0, 160)} · ${duration(recording.durationSeconds)} · ${bytes(recording.bytesWritten)} · 완료 파일 ${recording.segmentCount}개.${snapshot.chatStatus ? ` 채팅 ${chatLabels[snapshot.chatStatus] ?? snapshot.chatStatus}${snapshot.chatCount === undefined ? "" : ` · ${snapshot.chatCount}개`}.` : ""}`
    : `녹화 보관함 · ${recordings.filter(item => !emptyRecordingAttempt(item)).length}개${selected ? ` · ${recordingStatus(selected)}` : ""}.`;
  const screenshotDetails = snapshot.lastScreenshot ? privacyMode ? " 최근 화면이 저장되었습니다." : ` 최근 화면 저장: ${snapshot.lastScreenshot.fileName.slice(0, 160)}.` : "";
  const screenshotJustSaved = snapshot.lastScreenshot && Date.now() - snapshot.lastScreenshot.createdAt >= 0 && Date.now() - snapshot.lastScreenshot.createdAt < 5000;
  const statusErrors = [pollError, viewportError, snapshot.error, actionError].filter((value): value is string => !!value);
  const recordingPhase = snapshot.status === "starting" ? "녹화 준비" : snapshot.status === "waiting_source" ? "수신 대기" : snapshot.status === "stopping" || pending === "stop" ? "저장 중" : "녹화 중";
  const compactStatus = statusErrors.length ? hasRecording ? `${recordingPhase} · 확인 필요` : "연결 확인 필요"
    : pending === "stop" ? "파일 마무리 중"
    : chatProblem ? `${recordingPhase} · 채팅 확인`
    : screenshotJustSaved ? hasRecording ? `${recordingPhase} · 화면 저장됨` : "화면 저장됨" : connectionLabel(snapshot);
  const liveNoticeClass = view === "live" ? "official-browser-sr-only" : "official-browser-error";
  const focusError = statusErrors.join(" ") || (chatProblem ? "채팅 기록에 누락이 있을 수 있습니다. 녹화 상태를 확인해 주세요." : "");

  if (!active) return null;
  return <section className={`official-browser-panel${view === "live" ? " is-live" : ""}`} aria-label="CHZZK 공식 시청">
    {view === "live" ? <span className="official-browser-sr-only" role="status">{compactStatus}</span> : null}
    {!desktop && view !== "live" ? <p className="official-browser-note">미리보기입니다. 시청·녹화는 데스크톱 앱에서 이용해 주세요.</p> : null}
    {pollError ? <p className={liveNoticeClass} role="alert">{pollError} 마지막 확인 상태를 표시하고 있습니다.</p> : null}
    {viewportError ? <p className={liveNoticeClass} role="alert">{viewportError}</p> : null}
    {snapshot.error ? <p className={liveNoticeClass} role="alert">{snapshot.error}</p> : null}
    {actionError ? <p className={liveNoticeClass} role="alert">{actionError}</p> : null}
    {message ? <p className={view === "live" ? "official-browser-sr-only" : "official-browser-note"} role="status">{message}</p> : null}

    {view === "live" ? <div ref={stage} className="official-browser-stage" role="region" aria-label="공식 CHZZK 플레이어와 채팅" data-native-visible={nativeVisible}>
      {(!snapshot.windowOpen || settingsOpen) && !privacyMode ? <div className={`official-browser-setup${settingsOpen ? " is-modal" : ""}`} role={settingsOpen ? "dialog" : undefined} aria-modal={settingsOpen ? "true" : undefined} aria-label={settingsOpen ? "시청 설정" : "채널 연결"} data-native-overlay={settingsOpen} data-native-preserve-video={settingsOpen} hidden={!!control} onKeyDown={settingsKeys}>
        <ConnectionSetup mode={connectionMode} onMode={(mode) => { setConnectionMode(mode); if (mode === "mado" && !madoInputs.some(Boolean)) setMadoInputs([input || snapshot.channelId || "", "", "", ""]); }} modeDisabled={pending !== null || hasRecording || extensionBusy || !onMadoMode}
          inputs={connectionMode === "mado" ? madoInputs : [input]} onInput={(index, value) => connectionMode === "mado" ? setMadoInputs((rows) => rows.map((row, i) => i === index ? value : row)) : setInput(value)}
          disabled={!desktop || pending !== null || hasRecording || extensionBusy || connectionLocked} pending={pending === "open" || pending === "mado"} onConnect={open} onClose={settingsOpen ? () => setSettingsOpen(false) : undefined} closeRef={settingsClose}
          auth={pollError ? "unknown" : accountStatus(snapshot)} accountDisabled={!desktop || pending !== null || hasRecording || snapshot.accountBusy === true} accountReason={hasRecording ? "녹화를 중지한 뒤 방송·모드·계정을 변경할 수 있습니다." : undefined}
          authChecking={pending === "auth-refresh" || snapshot.authChecking} authError={snapshot.authError} refreshDisabled={!desktop || pending !== null || snapshot.accountBusy === true}
          onLogin={() => void runAction("login", () => api.login(), acceptSnapshot)} onLogout={logout} onRefresh={() => void runAction("auth-refresh", () => api.snapshot(true), acceptSnapshot)}
          onGrid={() => void runAction("extension", () => api.connectExtension(), acceptSnapshot)} gridDisabled={!desktop || pending !== null || hasRecording || extensionBusy || !snapshot.windowOpen || connectionLocked} gridStatus={extensionSummary}
          onInstaller={(browser) => void runAction("installer", () => api.openInstaller(browser))} installerDisabled={!desktop || pending !== null || hasRecording}
          helpDetails={`${recordingDetails}${screenshotDetails} 버튼을 눌러야 녹화가 시작됩니다. 다른 메뉴로 이동하거나 창을 최소화해도 녹화는 계속됩니다.`}
          options={<><div className="mado-layout-options"><button type="button" aria-pressed={madoLayout === "paired"} onClick={() => setMadoLayout("paired")}>4화면4챗</button><button type="button" aria-pressed={madoLayout === "chats"} onClick={() => setMadoLayout("chats")}>1화면4챗</button></div>{madoLayout === "chats" ? <label>영상 방송 <select value={madoLead} onChange={(event) => setMadoLead(Number(event.target.value))}>{madoInputs.map((_, index) => <option key={index} value={index}>방송 {index + 1}</option>)}</select></label> : null}</>}
        >
          <div className="official-browser-heading-actions"><span className="official-browser-resolution">{videoSize ?? "영상 확인 전"}</span><span role="status">{compactStatus}</span></div>
          <div className="official-browser-options"><label title="채팅과 함께 식별 가능한 공개 프로필 링크·시청자 수 관측을 저장합니다."><input type="checkbox" checked={captureChat} disabled={!desktop || pending !== null || hasRecording} aria-description="채팅과 함께 식별 가능한 공개 프로필 링크·시청자 수 관측을 저장합니다." onChange={(event) => { const enabled = event.target.checked; if (api.setCaptureChat) void runAction("control", () => api.setCaptureChat!(enabled)); else setCaptureChat(enabled); }} /> 채팅도 저장</label></div>
          <div className="official-browser-actions">
            {!hasRecording ? <button type="button" className="official-browser-record" data-ready={canStart} disabled={!canStart} onClick={start}>{pending === "start" ? "시작 중…" : "녹화 시작"}</button>
              : <button type="button" className="official-browser-stop" disabled={!desktop || pending !== null || snapshot.status === "stopping"} onClick={stop}>{pending === "stop" || snapshot.status === "stopping" ? "저장 중…" : "녹화 중지"}</button>}
          </div>
          {startHint ? <span className="official-browser-sr-only">{startHint}</span> : null}
          {connectionLocked ? <p className="official-browser-muted">기존 수신 세션을 함께 사용하고 있습니다. 방송·연결 변경은 시청을 닫은 뒤 일반 라이브에서 할 수 있습니다.</p> : null}
        </ConnectionSetup>
      </div> : <div className="official-browser-stage-placeholder"><span aria-hidden="true">◉</span><strong>{privacyMode ? "프라이버시 모드" : "시청 화면을 잠시 가렸습니다"}</strong><p>진행 중인 녹화는 유지됩니다.</p></div>}
    </div> : null}


    {view === "live" && snapshot.windowOpen && focusError && focusError !== dismissedFocusError && !control && !settingsOpen ? <div className="official-browser-focus-alert" role="alert" data-native-overlay="true"><span>{focusError}</span><button type="button" onClick={() => setDismissedFocusError(focusError)}>확인</button></div> : null}

    {view === "recordings" ? <RecordingLibrary recordings={recordings} selectedId={selectedId} onSelect={setSelectedId}
      disabled={!desktop || pending !== null} privacy={privacyMode} retrying={pending === "merge"} opening={pending === "merged"}
      onFolder={openFolder} onReplay={setReplayId} onOpenMerged={openMerged} onRetryMerge={retryMerge} onOpenSegment={openSegment} onDelete={deleteRecordings}
      stopControl={hasRecording ? <button type="button" className="official-browser-stop" disabled={!desktop || pending !== null || snapshot.status === "stopping"} onClick={stop}>{pending === "stop" || snapshot.status === "stopping" ? "저장 중…" : "녹화 중지"}</button> : undefined}
    /> : null}
    {control ? <div className="official-browser-dialog-backdrop" role="alertdialog" aria-modal="true" data-native-preserve-video="true" aria-labelledby="official-browser-control-title" aria-describedby="official-browser-control-description" onKeyDown={controlKeys}>
      <div className="official-browser-dialog" data-native-dialog-surface="true"><h3 id="official-browser-control-title">화면을 저장할까요?</h3>
        <p id="official-browser-control-description">설정된 저장 위치에 스크린샷을 저장합니다.</p>
        {!canApproveControl && pending !== "control" ? <p className="official-browser-muted">현재 재생·녹화 상태를 확인해 주세요.</p> : null}
        {actionError ? <p className="official-browser-error" role="alert">{actionError}</p> : null}
        <div><button ref={controlCancel} type="button" disabled={pending !== null} onClick={() => confirmControl(false)}>취소</button><button type="button" className="official-browser-primary" disabled={!canApproveControl} onClick={() => confirmControl(true)}>{pending === "control" ? "처리 중…" : "저장 확인"}</button></div>
      </div>
    </div> : null}
    {replayId && active ? <Suspense fallback={<p role="status">다시보기를 준비하고 있습니다…</p>}><RecordingReplay key={replayId} recordingId={replayId} runtime={runtime} privacyMode={privacyMode} liveRecording={hasRecording} onClose={() => setReplayId(null)} api={replayApi} /></Suspense> : null}
  </section>;
}
