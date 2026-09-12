import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { ApiResult } from "../../api/contracts";
import {
  claimOfficialBrowserViewport, createOfficialBrowserApi, emptyOfficialBrowserSnapshot, hiddenOfficialBrowserViewport,
  type BrowserRecording, type OfficialBrowserApi, type OfficialBrowserSnapshot, type OfficialBrowserViewport,
} from "../../api/officialBrowser";
import "./OfficialBrowserPanel.css";
import { RecordingPlayback, recordingMergeLabel } from "./RecordingPlayback";
import { RecordingReplay } from "./RecordingReplay";
import type { ReplayApi } from "../../api/replay";

type PendingAction = "open" | "start" | "stop" | "extension" | "folder" | "segment" | "login" | "logout" | "installer" | "merge" | "merged" | "control" | "request-control";

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
};

export function hasNativeOverlay(): boolean {
  return [...document.querySelectorAll<HTMLElement>('dialog[open], [role="dialog"][aria-modal="true"], [role="alertdialog"], [role="menu"], [data-native-overlay="true"]')].some((element) => {
    const style = getComputedStyle(element);
    return !element.closest('[hidden], [aria-hidden="true"]') && style.display !== "none" && style.visibility !== "hidden";
  });
}

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

const recordingLabels: Record<BrowserRecording["status"], string> = {
  recording: "녹화 중", stopped: "저장 완료", interrupted: "중단됨", failed: "실패",
};
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
const savedChatNote = (recording: BrowserRecording): { text: string; warning: boolean } => {
  if (recording.captureChat === false) return { text: "이 기록은 채팅 저장을 사용하지 않았습니다.", warning: false };
  if (recording.captureChat !== true || !recording.chatStatus) return { text: "이 기록의 채팅 저장 상태는 확인되지 않았습니다.", warning: false };
  const status = recording.chatStatus;
  const warning = ["failed", "unavailable", "storage_failed", "disconnected", "observer_unavailable", "queue_overflow", "unsupported_frame", "frame_too_large", "invalid_frame", "message_too_large", "message_truncated", "decode_failed", "observer_overflow", "connection_gap", "partial"].includes(status);
  const text = status === "storage_failed" ? "채팅 저장 실패 · 일부 기록 누락 가능"
    : warning ? `채팅 일부 누락 가능 · ${chatLabels[status] ?? "상태 확인 필요"}` : `채팅 ${chatLabels[status] ?? "저장 상태 확인 필요"}`;
  const count = recording.chatCount;
  return { warning, text: text + (typeof count === "number" && Number.isSafeInteger(count) && count >= 0 ? ` · ${count.toLocaleString("ko-KR")}개 기록` : "") };
};
function SavedChatWarning({ recording }: { recording: BrowserRecording }) {
  const note = savedChatNote(recording);
  return note.warning ? <span className="official-browser-chat-warning official-browser-saved-chat-warning" role="img" aria-label={note.text} title={note.text}> ⚠</span> : null;
}
const duration = (value: number) => {
  const seconds = Math.max(0, Math.floor(value));
  return `${Math.floor(seconds / 3600).toString().padStart(2, "0")}:${Math.floor(seconds / 60 % 60).toString().padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
};
const bytes = (value: number) => value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GiB` : `${(value / 1024 ** 2).toFixed(1)} MiB`;

function connectionLabel(snapshot: OfficialBrowserSnapshot): string {
  if (snapshot.status === "desktop_required") return "데스크톱 앱 필요";
  if (snapshot.status === "stopping") return "파일 마무리 중";
  if (snapshot.status === "starting") return "녹화 준비 중";
  if (snapshot.recordingId || snapshot.status === "recording") return "녹화 중";
  if (snapshot.error || snapshot.status === "error") return "연결 확인 필요";
  if (snapshot.status === "loading") return "공식 플레이어 확인 중";
  if (snapshot.ready && snapshot.windowOpen) return "녹화 준비됨";
  if (snapshot.windowOpen) return "로그인·재생 상태 확인 필요";
  return "시청 창 닫힘";
}

export function OfficialBrowserPanel({ runtime, active, view, privacyMode = false, api: suppliedApi, replayApi, onMadoMode }: OfficialBrowserPanelProps) {
  const api = useMemo(() => suppliedApi ?? createOfficialBrowserApi(runtime), [suppliedApi, runtime]);
  const desktop = runtime === "tauri" && api.runtime === "tauri";
  const [snapshot, setSnapshot] = useState(() => emptyOfficialBrowserSnapshot(runtime));
  const [input, setInput] = useState("");
  const [rights, setRights] = useState(false);
  const [captureChat, setCaptureChat] = useState(true);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [replayId, setReplayId] = useState<string | null>(null);
  const [viewportError, setViewportError] = useState<string | null>(null);
  const [logoutConfirm, setLogoutConfirm] = useState(false);
  const [controlsOpen, setControlsOpen] = useState<boolean | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const modalOcclusion = useRef(false);
  modalOcclusion.current = logoutConfirm || settingsOpen || replayId !== null;
  const settingsClose = useRef<HTMLButtonElement>(null);
  const [nativeVisible, setNativeVisible] = useState(false);
  const [dismissedControl, setDismissedControl] = useState<string | null>(null);
  const [expiredControl, setExpiredControl] = useState<string | null>(null);
  const [dismissedFocusError, setDismissedFocusError] = useState("");
  const seenUiAction = useRef<string | null>(null);
  const stage = useRef<HTMLDivElement>(null);
  const logoutCancel = useRef<HTMLButtonElement>(null);
  const logoutTrigger = useRef<HTMLButtonElement>(null);
  const logoutWasOpen = useRef(false);
  const controlCancel = useRef<HTMLButtonElement>(null);
  const controlWasOpen = useRef<string | null>(null);
  const controlPriorFocus = useRef<HTMLElement | null>(null);
  const lifetime = useRef(0);
  const requestVersion = useRef(0);
  const actionInFlight = useRef(false);
  const recording = snapshot.recordings.find((item) => item.id === snapshot.recordingId)
    ?? snapshot.recordings.find((item) => item.status === "recording");
  const hasRecording = !!snapshot.recordingId || !!recording || snapshot.status === "recording" || snapshot.status === "stopping" || snapshot.status === "starting";
  const recordings = useMemo(() => [...snapshot.recordings].sort((left, right) => right.startedAt - left.startedAt), [snapshot.recordings]);
  const selected = recordings.find((item) => item.id === selectedId) ?? recording ?? recordings[0];
  const extensionBusy = ["connecting", "연결 중"].includes(snapshot.extensionStatus ?? "");
  const controlsExpanded = controlsOpen ?? !snapshot.windowOpen;
  const extensionLabel = snapshot.extensionStatus === "not_connected" || !snapshot.extensionStatus ? "미연결" : snapshot.extensionStatus;
  const extensionSummary = ({ "미연결": "미연결", "연결됨": "연결됨", "connecting": "연결 중", "연결 중": "연결 중", "확장 로드 완료 · 공식 페이지 감지 확인 중": "확인 중", "공식 페이지 확장 감지됨 · 커넥터/고화질은 실제 재생으로 확인": "확장 감지됨" } as Record<string, string>)[extensionLabel] ?? "확인 필요";
  // One predicate owns behavior and visual readiness. Loading an extension is
  // not itself required: ordinary-quality playback can also be recorded.
  const canStart = active && desktop && pending === null && rights && snapshot.ready && snapshot.windowOpen
    && !hasRecording && !pollError && !extensionBusy;
  const requestedControl = snapshot.pendingControl;
  const control = desktop && active && view === "live" && !logoutConfirm && requestedControl
    && requestedControl.id !== dismissedControl && requestedControl.id !== expiredControl
    && ["record_start", "record_stop", "screenshot"].includes(requestedControl.action)
    && requestedControl.channelId === snapshot.channelId && Number.isFinite(requestedControl.expiresAt)
    && requestedControl.expiresAt > Date.now() ? requestedControl : null;
  const canApproveControl = control?.action === "record_start" ? canStart
    : control?.action === "record_stop" ? pending === null && hasRecording && snapshot.status !== "stopping"
    : !!control && pending === null && rights && snapshot.ready && snapshot.windowOpen && !pollError && !extensionBusy;
  const startHint = !desktop ? "데스크톱 앱에서 녹화할 수 있습니다."
    : hasRecording ? "탐색·배속·채널 변경 시 녹화가 중단됩니다."
    : pending === "start" ? "녹화를 준비하고 있습니다."
    : extensionBusy ? "고화질 연결이 끝나면 녹화할 수 있습니다."
    : pollError ? "연결 상태를 다시 확인하고 있습니다."
    : !snapshot.windowOpen ? "시청을 시작한 뒤 녹화할 수 있습니다."
    : !snapshot.ready ? "영상이 재생되면 녹화할 수 있습니다."
    : !rights ? "아래 저장 권한을 확인해 주세요." : null;

  useLayoutEffect(() => {
    if (!desktop) return;
    const owner = claimOfficialBrowserViewport(api, setViewportError);
    let frame: number | undefined;
    const measure = () => {
      frame = undefined;
      const visible = active && view === "live" && snapshot.windowOpen && !privacyMode
        && !document.hidden && !document.fullscreenElement;
      const measured = visible && stage.current ? measureOfficialBrowserViewport(stage.current) : hiddenOfficialBrowserViewport;
      // Native main-document reload increments this epoch. An old document's
      // delayed show must not reveal the child over a freshly loading UI.
      const viewport = { ...measured, occluded: measured.visible && (modalOcclusion.current || hasNativeOverlay()), epoch: snapshot.viewportEpoch ?? 0 };
      setNativeVisible(viewport.visible && !viewport.occluded);
      owner.update(viewport);
    };
    const schedule = () => {
      if (frame === undefined) frame = window.requestAnimationFrame(measure);
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
    window.addEventListener("resize", schedule);
    document.addEventListener("scroll", schedule, true);
    document.addEventListener("visibilitychange", schedule);
    document.addEventListener("fullscreenchange", schedule);
    measure();
    return () => {
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      resize?.disconnect();
      mutation.disconnect();
      window.removeEventListener("resize", schedule);
      document.removeEventListener("scroll", schedule, true);
      document.removeEventListener("visibilitychange", schedule);
      document.removeEventListener("fullscreenchange", schedule);
      owner.release();
    };
  }, [api, desktop, active, view, snapshot.windowOpen, snapshot.viewportEpoch, privacyMode]);

  useEffect(() => {
    if (!active || view !== "live") { setLogoutConfirm(false); setSettingsOpen(false); }
  }, [active, view]);
  useEffect(() => {
    if (settingsOpen && !control && !logoutConfirm) settingsClose.current?.focus();
  }, [settingsOpen, control?.id, logoutConfirm]);
  useEffect(() => {
    if (logoutConfirm) logoutCancel.current?.focus();
    else if (logoutWasOpen.current) logoutTrigger.current?.focus();
    logoutWasOpen.current = logoutConfirm;
  }, [logoutConfirm]);
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
    if (!active || !desktop || view !== "live" || privacyMode || control || logoutConfirm || pending !== null || !intent
      || intent.action !== "open_settings" || seenUiAction.current === intent.id) return;
    seenUiAction.current = intent.id;
    if (Number.isFinite(intent.expiresAt) && intent.expiresAt > Date.now()) {
      if (intent.action === "open_settings") { setControlsOpen(true); setSettingsOpen(true); }

    }
    // ACKs consume UI-only intents; never replay their possibly stale snapshot.
    void api.ackUiAction(intent.id).catch(() => {});
  }, [api, active, desktop, view, privacyMode, control?.id, logoutConfirm, pending, snapshot.pendingUiAction?.id]);
  useEffect(() => {
    if (!active || !desktop || view !== "live") return;
    const shortcut = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "s" || event.defaultPrevented || event.repeat || event.isComposing || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey
        || pending !== null || control || logoutConfirm || privacyMode || hasNativeOverlay() || !snapshot.ready || !snapshot.windowOpen) return;
      const element = event.target instanceof Element ? event.target : null;
      if (element?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="dialog"], [role="alertdialog"]')) return;
      event.preventDefault();
      void runAction("request-control", () => api.requestControl("screenshot"), acceptSnapshot);
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [api, active, desktop, view, pending, control?.id, logoutConfirm, privacyMode, snapshot.ready, snapshot.windowOpen]);
  const open = () => {
    if (!input.trim() || hasRecording) return;
    void runAction("open", () => api.open(input.trim()), (next) => { acceptSnapshot(next); if (next.windowOpen) { setControlsOpen(false); setSettingsOpen(false); } });
  };
  const start = () => {
    if (!canStart) return;
    void runAction("start", () => api.start({ rightsAcknowledged: rights, captureChat }), (next) => {
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
  const logout = () => {
    if (hasRecording) return;
    void runAction("logout", () => api.logout(), (next) => {
      acceptSnapshot(next); setRights(false); setLogoutConfirm(false);
      setMessage("공식 시청 프로필에서 로그아웃했습니다. 녹화 파일과 다른 브라우저 계정은 그대로 유지됩니다.");
    });
  };
  const confirmControl = (approve: boolean) => {
    if (!control || pending !== null || control.expiresAt <= Date.now() || (approve && !canApproveControl)) return;
    const requestId = control.id;
    void runAction("control", () => api.confirmControl({ requestId, approve, rightsAcknowledged: rights, captureChat }), (next) => {
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
  const logoutKeys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" && pending !== "logout") { event.preventDefault(); setLogoutConfirm(false); }
    if (event.key !== "Tab") return;
    const controls = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const first = controls[0], last = controls[controls.length - 1];
    if (!first || !last) { event.preventDefault(); return; }
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  };
  const settingsKeys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!settingsOpen || event.defaultPrevented) return;
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setSettingsOpen(false); }
    if (event.key !== "Tab") return;
    const controls = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), summary')]
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
    : `녹화 보관함 · ${recordings.length}개${selected ? ` · ${recordingLabels[selected.status]}` : ""}.`;
  const screenshotDetails = snapshot.lastScreenshot ? privacyMode ? " 최근 화면이 저장되었습니다." : ` 최근 화면 저장: ${snapshot.lastScreenshot.fileName.slice(0, 160)}.` : "";
  const screenshotJustSaved = snapshot.lastScreenshot && Date.now() - snapshot.lastScreenshot.createdAt >= 0 && Date.now() - snapshot.lastScreenshot.createdAt < 5000;
  const statusErrors = [pollError, viewportError, snapshot.error, actionError].filter((value): value is string => !!value);
  const recordingPhase = snapshot.status === "starting" ? "녹화 준비" : snapshot.status === "stopping" || pending === "stop" ? "저장 중" : "녹화 중";
  const compactStatus = statusErrors.length ? hasRecording ? `${recordingPhase} · 확인 필요` : "연결 확인 필요"
    : pending === "stop" ? "파일 마무리 중"
    : chatProblem ? `${recordingPhase} · 채팅 확인`
    : screenshotJustSaved ? hasRecording ? `${recordingPhase} · 화면 저장됨` : "화면 저장됨" : connectionLabel(snapshot);
  const liveNoticeClass = view === "live" ? "official-browser-sr-only" : "official-browser-error";
  const focusError = statusErrors.join(" ") || (chatProblem ? "채팅 기록에 누락이 있을 수 있습니다. 녹화 상태를 확인해 주세요." : "");

  if (!active) return null;
  return <section className={`official-browser-panel${view === "live" ? " is-live" : ""}`} aria-label="CHZZK 공식 시청">
    {view !== "live" ? <header className="official-browser-heading"><h2>녹화 보관함</h2></header>
      : <span className="official-browser-sr-only" role="status">{compactStatus}</span>}
    {!desktop && view !== "live" ? <p className="official-browser-note">미리보기입니다. 시청·녹화는 데스크톱 앱에서 이용해 주세요.</p> : null}
    {pollError ? <p className={liveNoticeClass} role="alert">{pollError} 마지막 확인 상태를 표시하고 있습니다.</p> : null}
    {viewportError ? <p className={liveNoticeClass} role="alert">{viewportError}</p> : null}
    {snapshot.error ? <p className={liveNoticeClass} role="alert">{snapshot.error}</p> : null}
    {actionError ? <p className={liveNoticeClass} role="alert">{actionError}</p> : null}
    {message ? <p className={view === "live" ? "official-browser-sr-only" : "official-browser-note"} role="status">{message}</p> : null}

    {view === "live" ? <div ref={stage} className="official-browser-stage" role="region" aria-label="공식 CHZZK 플레이어와 채팅" data-native-visible={nativeVisible}>
      {(!snapshot.windowOpen || settingsOpen) && !privacyMode ? <div className={`official-browser-setup${settingsOpen ? " is-modal" : ""}`} role={settingsOpen ? "dialog" : undefined} aria-modal={settingsOpen ? "true" : undefined} aria-label={settingsOpen ? "시청 설정" : "채널 연결"} data-native-overlay={settingsOpen} hidden={!!control || logoutConfirm} onKeyDown={settingsKeys}>
        <div className="official-browser-setup-card">
          <div className="official-browser-setup-heading"><h2>{settingsOpen ? "시청 설정" : "방송 연결"}</h2>{settingsOpen ? <button ref={settingsClose} type="button" onClick={() => setSettingsOpen(false)}>설정 닫기</button> : null}</div>
          <div className="official-browser-heading-actions"><span className="official-browser-resolution">{videoSize ? `${videoSize}${snapshot.videoPaused === true ? " · 일시정지" : ""}` : "영상 확인 전"}</span><span className={`official-browser-status${hasRecording ? " is-recording" : ""}${chatProblem ? " has-warning" : ""}${statusErrors.length ? " has-error" : ""}`} role={statusErrors.length || chatProblem ? "alert" : "status"}>{compactStatus}{chatProblem ? <span className="official-browser-chat-warning official-browser-sr-only" aria-label="채팅 기록 확인 필요"> !</span> : null}</span><Help label="녹화 상태" text={`${statusErrors.join(" ")} ${message ?? ""} ${recordingDetails}${screenshotDetails}`} /></div>
          {!desktop ? <p className="official-browser-note">미리보기입니다. 시청·녹화는 데스크톱 앱에서 이용해 주세요.</p> : null}
          {onMadoMode ? <button type="button" className="official-browser-mado" disabled={pending !== null} title="여러 방송과 채팅을 함께 봅니다" onClick={() => { setSettingsOpen(false); onMadoMode(); }}>마도</button> : null}
          <div className="official-browser-controls">
      <div className="official-browser-quickbar">
        <button type="button" disabled={!desktop || pending !== null || hasRecording || extensionBusy || !snapshot.windowOpen} onClick={() => void runAction("extension", () => api.connectExtension(), acceptSnapshot)}>{pending === "extension" || extensionBusy ? "연결 중…" : "고화질 연결"}</button>
        <span className="official-browser-extension-status" role="status">확장 · {extensionSummary}</span>
        <Help label="고화질 연결" text={`현재 상태: ${extensionLabel}. 설치한 네이버 확장을 Atsumi에 연결합니다. 설치만으로 연결되지는 않습니다. 성공한 연결은 다음 실행 때 다시 시도합니다. 연결 성공이 고화질 재생을 보장하지는 않으므로 화면 위 해상도를 확인하세요. 설치 안내는 설정에서 열 수 있습니다.`} />
        <Help label="시청 시작" text={snapshot.ready ? "채널 변경과 로그인은 설정에서 할 수 있습니다." : "공식 페이지에 확장 설치 안내가 나오면 ‘설치없이 일반 화질 시청’을 선택하거나 고화질 연결을 이용해 주세요. 영상이 재생되면 녹화할 수 있습니다."} />
      </div>
      <details className="official-browser-settings" open={controlsExpanded}>
        <summary onClick={(event) => { event.preventDefault(); setControlsOpen(!controlsExpanded); }}>{snapshot.windowOpen ? "설정" : "채널 연결"}</summary>
        <div className="official-browser-settings-body">
      <label className="official-browser-input-label" htmlFor="official-browser-channel">채널 주소 또는 ID</label>
      <div className="official-browser-channel-row">
        <input id="official-browser-channel" value={input} disabled={!desktop || pending !== null || hasRecording} onChange={(event) => setInput(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); open(); } }} placeholder="https://chzzk.naver.com/live/채널ID" spellCheck={false} autoComplete="off" />
        <button type="button" className="official-browser-primary" disabled={!desktop || pending !== null || !input.trim() || hasRecording} onClick={open}>{pending === "open" ? "연결 중…" : "시청 시작"}</button>
      </div>
      <div className="official-browser-account">
        <button type="button" disabled={!desktop || pending !== null || hasRecording || !snapshot.windowOpen} onClick={() => void runAction("login", () => api.login(), acceptSnapshot)}>{pending === "login" ? "로그인 여는 중…" : "로그인"}</button>
        <button ref={logoutTrigger} type="button" disabled={!desktop || pending !== null || hasRecording || !snapshot.windowOpen} onClick={() => setLogoutConfirm(true)}>로그아웃</button>
        <span>{loginLabel}</span>
        <Help label="로그인" text="로그인·본인인증은 임시 창에서 진행합니다. 치트키·타임머신은 계정 권한과 방송 설정에 따릅니다. Chrome과 별도인 CHZZK 전용 프로필에 로그인 세션을 보관합니다. 비밀번호·쿠키를 추출하는 기능은 없지만, 개발 중인 앱이므로 독립적인 보안 검증을 마친 브라우저와 같다고 보장하지 않습니다." />

      </div>
      <div className="official-browser-options">
        <label><input type="checkbox" checked={captureChat} disabled={!desktop || pending !== null || hasRecording} onChange={(event) => setCaptureChat(event.target.checked)} /> 채팅도 저장</label>
        <Help label="채팅 저장" text="녹화 시작 이후 공식 페이지가 수신한 일반 채팅을 저장합니다. 과거 기록·후원·숨김 메시지는 제외되며, 연결이 끊긴 동안의 채팅은 누락될 수 있습니다. 녹화 폴더의 chat.jsonl에서 확인할 수 있습니다." />
        <div className="official-browser-installer-actions"><span>확장 설치</span><button type="button" disabled={!desktop || pending !== null || hasRecording} onClick={() => void runAction("installer", () => api.openInstaller("chrome"))}>Chrome</button><button type="button" disabled={!desktop || pending !== null || hasRecording} onClick={() => void runAction("installer", () => api.openInstaller("edge"))}>Edge</button></div>
      </div>
      </div></details>
      <div className="official-browser-actions">
        {!hasRecording ? <button type="button" className="official-browser-record" data-ready={canStart} disabled={!canStart} aria-describedby={startHint ? "official-browser-start-hint" : undefined} onClick={start}>{pending === "start" ? "시작 중…" : "녹화 시작"}</button>
          : <button type="button" className="official-browser-stop" disabled={!desktop || pending !== null || snapshot.status === "stopping"} onClick={stop}>{pending === "stop" || snapshot.status === "stopping" ? "저장 중…" : "녹화 중지"}</button>}
        <Help label="녹화" text={`${startHint ? `${startHint} ` : ""}버튼을 눌러야 녹화가 시작됩니다. 과거 탐색·배속 변경·채널 변경·새로고침 시 녹화가 중단됩니다. 다른 메뉴로 이동하거나 창을 최소화해도 녹화는 계속됩니다. 재생 중인 화면·소리를 약 15초씩 나누어 저장하며, 원본과 화질 차이나 파일 사이 짧은 공백이 생길 수 있습니다.`} />
        {startHint ? <span id="official-browser-start-hint" className="official-browser-sr-only">{startHint}</span> : null}
      </div>
      <div className="official-browser-consent"><label><input type="checkbox" checked={rights} disabled={!desktop || pending !== null || hasRecording} onChange={(event) => setRights(event.target.checked)} /> 이 방송을 저장할 권한과 적용되는 이용 조건을 확인했습니다.</label></div>
          </div>
        </div>
      </div> : <div className="official-browser-stage-placeholder"><span aria-hidden="true">◉</span><strong>{privacyMode ? "프라이버시 모드" : "시청 화면을 잠시 가렸습니다"}</strong><p>진행 중인 녹화는 유지됩니다.</p></div>}
    </div> : null}
    {view === "recordings" && hasRecording ? <div className="official-browser-actions"><button type="button" className="official-browser-stop" disabled={!desktop || pending !== null || snapshot.status === "stopping"} onClick={stop}>{pending === "stop" || snapshot.status === "stopping" ? "저장 중…" : "녹화 중지"}</button></div> : null}

    {view === "live" && snapshot.windowOpen && focusError && focusError !== dismissedFocusError && !control && !settingsOpen && !logoutConfirm ? <div className="official-browser-focus-alert" role="alert" data-native-overlay="true"><span>{focusError}</span><button type="button" onClick={() => setDismissedFocusError(focusError)}>확인</button></div> : null}

    {view === "recordings" && recording ? <div className="official-browser-live-stats"><strong>{privacyMode ? "공식 CHZZK 녹화" : recording.title || recording.channelId}</strong><span>{duration(recording.durationSeconds)} · {bytes(recording.bytesWritten)} · 완료 파일 {recording.segmentCount}개</span>{snapshot.chatStatus ? <span>채팅 {chatLabels[snapshot.chatStatus] ?? snapshot.chatStatus}{snapshot.chatCount === undefined ? "" : ` · ${snapshot.chatCount}개`}</span> : null}</div> : null}
    {view === "recordings" ? <details className="official-browser-recordings" open>
      <summary>녹화 기록 <span>{recordings.length}개</span></summary>
      {recordings.length ? <div className="official-browser-library">
        <div className="official-browser-recording-list">{recordings.map((item) => <button type="button" key={item.id} className="official-browser-recording-select" aria-pressed={selected?.id === item.id} title={savedChatNote(item).text} aria-description={savedChatNote(item).text} onClick={() => setSelectedId(item.id)}><strong>{privacyMode ? "공식 CHZZK 녹화" : item.title || item.channelId}</strong><span>{recordingLabels[item.status]} · {duration(item.durationSeconds)} · {bytes(item.bytesWritten)}<SavedChatWarning recording={item} /></span><span>{recordingMergeLabel(item)}</span><small>{new Date(item.startedAt).toLocaleString("ko-KR")}</small></button>)}</div>
        {selected ? <section className="official-browser-files" aria-label="공식 녹화 파일">
          <div className="official-browser-files-heading"><h3>{privacyMode ? "선택한 공식 녹화" : selected.title || selected.channelId}</h3><button type="button" disabled={!desktop || pending !== null} onClick={() => openFolder(selected.id)}>녹화 폴더 열기</button></div>
          <p className="official-browser-muted" title={savedChatNote(selected).text}>{recordingLabels[selected.status]} · 완료 파일 {selected.segmentCount}개 · {duration(selected.durationSeconds)}<SavedChatWarning recording={selected} /></p>
          {selected.lastError ? <p className="official-browser-error">{selected.lastError}</p> : null}
          {selected.partial ? <p className="official-browser-note">마무리되지 않은 파일이 폴더에 보존되어 있습니다. 완성된 영상으로 재생되지 않을 수 있습니다.</p> : null}
          <RecordingPlayback key={selected.id} recording={selected} disabled={!desktop || pending !== null} privacyMode={privacyMode} retrying={pending === "merge"} opening={pending === "merged"} onOpenMerged={openMerged} onReplay={setReplayId} onRetryMerge={retryMerge} onOpenSegment={openSegment} />
        </section> : null}
      </div> : <p className="official-browser-empty">저장한 공식 화면 녹화가 없습니다. 공식 창에서 방송 재생을 확인한 뒤 녹화를 시작해 주세요.</p>}
    </details> : null}
    {control ? <div className="official-browser-dialog-backdrop" role="alertdialog" aria-modal="true" aria-labelledby="official-browser-control-title" aria-describedby="official-browser-control-description" onKeyDown={controlKeys}>
      <div className="official-browser-dialog"><h3 id="official-browser-control-title">{control.action === "record_start" ? "녹화를 시작할까요?" : control.action === "record_stop" ? "녹화를 중지할까요?" : "화면을 저장할까요?"}</h3>
        <p id="official-browser-control-description">{control.action === "record_stop" ? "플레이어에서 요청한 녹화를 마무리합니다." : "플레이어에서 요청한 동작입니다. 설정된 저장 위치에 파일을 저장합니다."}</p>
        {control.action !== "record_stop" ? <label className="official-browser-control-rights"><input type="checkbox" checked={rights} disabled={pending !== null} onChange={(event) => setRights(event.target.checked)} /> 이 방송을 저장할 권한과 적용되는 이용 조건을 확인했습니다.</label> : null}
        {control.action === "record_start" ? <p className="official-browser-muted">탐색·배속·채널 변경 시 녹화가 중단됩니다.</p> : null}
        {!canApproveControl && pending !== "control" ? <p className="official-browser-muted">{control.action !== "record_stop" && !rights ? "저장 권한을 확인해 주세요." : "현재 재생·녹화 상태를 확인해 주세요."}</p> : null}
        {actionError ? <p className="official-browser-error" role="alert">{actionError}</p> : null}
        <div><button ref={controlCancel} type="button" disabled={pending !== null} onClick={() => confirmControl(false)}>취소</button><button type="button" className="official-browser-primary" disabled={!canApproveControl} onClick={() => confirmControl(true)}>{pending === "control" ? "처리 중…" : control.action === "record_start" ? "시작 확인" : control.action === "record_stop" ? "중지 확인" : "저장 확인"}</button></div>
      </div>
    </div> : null}
    {logoutConfirm ? <div className="official-browser-dialog-backdrop" role="alertdialog" aria-modal="true" aria-labelledby="official-browser-logout-title" onKeyDown={logoutKeys}><div className="official-browser-dialog"><h3 id="official-browser-logout-title">공식 시청에서 로그아웃할까요?</h3><p>Atsumi의 CHZZK 시청 프로필 로그인 세션을 지웁니다. Chrome·Edge 계정과 저장된 녹화 파일은 변경하지 않습니다.</p>{actionError ? <p className="official-browser-error" role="alert">{actionError}</p> : null}<div><button ref={logoutCancel} type="button" disabled={pending === "logout"} onClick={() => setLogoutConfirm(false)}>취소</button><button type="button" disabled={pending !== null || hasRecording} onClick={logout}>{pending === "logout" ? "로그아웃 중…" : "로그아웃 확인"}</button></div></div></div> : null}
    {replayId && active ? <RecordingReplay key={replayId} recordingId={replayId} runtime={runtime} privacyMode={privacyMode} liveRecording={hasRecording} onClose={() => setReplayId(null)} api={replayApi} /> : null}
  </section>;
}
