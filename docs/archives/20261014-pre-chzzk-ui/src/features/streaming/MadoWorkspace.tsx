import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { createMultiviewApi, emptyMultiview, multiviewEntries, type MultiviewApi, type MultiviewPane, type MultiviewSnapshot } from "../../api/multiview";
import { createOfficialBrowserApi, emptyOfficialBrowserSnapshot, hiddenOfficialBrowserViewport, type OfficialBrowserApi, type OfficialBrowserViewport } from "../../api/officialBrowser";
import { hasNativeOverlay, measureOfficialBrowserViewport, nativeModalOcclusion } from "./OfficialBrowserPanel";
import { ConnectionSetup, accountStatus, type ConnectionMode } from "./ConnectionSetup";
import { FluentIcon } from "../../components/FluentIcon";
import { hasBlockingNativeOverlay, observeNativeOverlayGeometry } from "./nativeOverlayGeometry";
import { LiveChannelPicker } from "./LiveChannelPicker";
import { LivePaneGrid } from "./LivePaneGrid";
import { tutorialStepEvent } from "../../tutorial/tourActions";
import "./MadoWorkspace.css";
import { usePanelHandleGesture } from "./usePanelHandleGesture";

type MadoLayout = { version: 1; direction: "horizontal" | "vertical"; horizontal: number; vertical: number };
const layoutKey = "atsumi.mado.layout.v1";
const defaultLayout: MadoLayout = { version: 1, direction: "horizontal", horizontal: 60, vertical: 50 };
const boundedRatio = (value: number) => Math.min(94, Math.max(10, Math.round(value)));
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

export function NativeStreamingPane({ pane, epoch, api, privacy, suspended = false, inputBlocked = false, placeholder, onError }: { pane: MultiviewPane; epoch: number; api: Pick<MultiviewApi, "runtime" | "setViewport">; privacy: boolean; suspended?: boolean; inputBlocked?: boolean; placeholder?: string; onError(message: string): void }) {
  const container = useRef<HTMLDivElement>(null);
  const report = useRef(onError); report.current = onError;
  const blocked = useRef(inputBlocked), refresh = useRef<() => void>(() => {});
  // Dragging must not tear down this effect: cleanup parks the native window.
  useLayoutEffect(() => { blocked.current = inputBlocked; refresh.current(); }, [inputBlocked]);
  useEffect(() => {
    const element = container.current;
    if (!element || api.runtime !== "tauri") return;
    let disposed = false, frame = 0, latest: { viewport: OfficialBrowserViewport; revision: number } | null = null, lastKey = "", revision = 0;
    let visibleFlights = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const hidden = { ...hiddenOfficialBrowserViewport, ...(privacy ? { suspendAudio: true } : {}), epoch };
    const send = async (viewport: OfficialBrowserViewport, current: number, attempt = 0) => {
      if (disposed) return;
      clearTimeout(retry);
      retry = undefined;
      const serialVisible = viewport.visible;
      if (serialVisible) visibleFlights++;
      try {
        const result = await api.setViewport(pane.paneId, viewport);
        if (!disposed && current === revision && result.ok && viewport.visible && !viewport.occluded) report.current("");
        if (!disposed && current === revision && !result.ok) {
          lastKey = "";
          if (viewport.occluded || viewport.preserveBackground) void api.setViewport(pane.paneId, hidden).catch(() => {});
        }
        if (!disposed && current === revision && !result.ok && !["VIEWPORT_STALE", "MULTIVIEW_STALE"].includes(result.error.code)) {
          if (viewport.visible && ["VIEWPORT_BUSY", "MULTIVIEW_BUSY", "VIEWPORT_CLIP_FAILED"].includes(result.error.code) && attempt < 2) {
            retry = setTimeout(() => { if (!disposed && current === revision) void send(viewport, current, attempt + 1); }, attempt ? 320 : 120);
          } else report.current(result.error.message);
        }
      } catch {
        if (!disposed && current === revision) {
          lastKey = "";
          if (viewport.occluded || viewport.preserveBackground) void api.setViewport(pane.paneId, hidden).catch(() => {});
          report.current("시청 영역을 표시하지 못했습니다. 다시 적용해 주세요.");
        }
      }
      finally {
        if (serialVisible) {
          visibleFlights--;
          if (!disposed && !visibleFlights && latest) {
            const next = latest; latest = null;
            if (next.revision === revision) void send(next.viewport, next.revision);
          }
        }
      }
    };
    const update = () => {
      if (disposed) return;
      if (document.hidden && !privacy && !suspended) {
        // Windows minimize and tray hide both fire visibilitychange. Only the
        // latter is an explicit audio suspension; keep the receiver on minimize.
        if (frame) cancelAnimationFrame(frame);
        frame = 0;
        const hideForTray = () => {
          if (disposed || !document.hidden) return;
          const viewport = { ...hidden, suspendAudio: true };
          const key = JSON.stringify(viewport);
          if (key === lastKey) return;
          lastKey = key; latest = null;
          void send(viewport, ++revision);
        };
        void import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
          const window = getCurrentWindow();
          const [minimized, visible] = await Promise.all([window.isMinimized(), window.isVisible()]);
          if (!minimized || !visible) hideForTray();
        }).catch(hideForTray);
        return;
      }
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      const measured = privacy || suspended || document.hidden || document.fullscreenElement ? hidden : { ...measureOfficialBrowserViewport(element), epoch };
      const overlay = measured.visible ? nativeModalOcclusion(element, measured) : { occluded: false };
      const viewport: OfficialBrowserViewport = { ...measured, ...overlay };
      // Disable only the child HWND's input during a drag. Preserve its pixels,
      // sound, receiver and any existing dialog cut-outs. Privacy still wins.
      if (blocked.current && measured.visible && (!overlay.occluded || overlay.preserveBackground)) {
        Object.assign(viewport, { occluded: true, preserveBackground: true,
          clip: viewport.clip ?? { x: 0, y: 0, width: viewport.width, height: viewport.height },
          occlusions: viewport.occlusions ?? [] });
      }
      const key = JSON.stringify(viewport);
      if (key === lastKey) return;
      lastKey = key;
      const current = ++revision;
      clearTimeout(retry); retry = undefined;
      // Security masks preempt an outstanding move; ordinary drag geometry is
      // coalesced to the latest frame instead of flooding the native dispatcher.
      if (!viewport.visible || overlay.occluded || overlay.preserveBackground) { latest = null; void send(viewport, current); }
      else if (visibleFlights) latest = { viewport, revision: current };
      else void send(viewport, current);
    };
    refresh.current = update;
    const schedule = () => { if (!disposed && !frame) frame = requestAnimationFrame(update); };
    const mutation = new MutationObserver(() => { if (hasNativeOverlay()) update(); else schedule(); });
    mutation.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["open", "hidden", "aria-hidden", "aria-modal", "role", "class", "style", "data-native-overlay", "data-native-preserve-video", "data-native-interactive-background"] });
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    resize?.observe(element);
    const stopOverlayGeometry = observeNativeOverlayGeometry(update);
    window.addEventListener("resize", schedule); document.addEventListener("scroll", schedule, true); document.addEventListener("visibilitychange", update); document.addEventListener("fullscreenchange", update);
    update();
    return () => {
      refresh.current = () => {};
      disposed = true; revision++; latest = null; clearTimeout(retry); cancelAnimationFrame(frame); resize?.disconnect(); stopOverlayGeometry(); mutation.disconnect();
      window.removeEventListener("resize", schedule); document.removeEventListener("scroll", schedule, true); document.removeEventListener("visibilitychange", update); document.removeEventListener("fullscreenchange", update);
      void api.setViewport(pane.paneId, hidden).catch(() => {});
    };
  }, [api, epoch, pane.paneId, privacy, suspended]);
  return <div className={`mado-native-slot is-${pane.kind}`} ref={container} aria-label={pane.kind === "video" ? "방송 화면" : "채팅 화면"}>
    <span>{privacy ? "프라이버시 모드" : suspended ? placeholder || "연결 상태 확인 중" : pane.kind === "video" ? "방송 연결 중" : "채팅 연결 중"}</span>
  </div>;
}

export function MadoWorkspace({ runtime, privacy, onLeave, unifiedLive = false, tutorialActive = false, onWatchingChange, connectionRequest = 0, onConnectionRequestHandled, onConnectionOpenChange, api: suppliedApi, officialApi: suppliedOfficialApi }: { runtime: MultiviewApi["runtime"]; privacy: boolean; onLeave(): void; unifiedLive?: boolean; tutorialActive?: boolean; onWatchingChange?(watching: boolean): void; connectionRequest?: number; onConnectionRequestHandled?(): void; onConnectionOpenChange?(open: boolean): void; api?: MultiviewApi; officialApi?: OfficialBrowserApi }) {
  const api = useMemo(() => suppliedApi ?? createMultiviewApi(runtime), [runtime, suppliedApi]);
  const officialApi = useMemo(() => suppliedOfficialApi ?? createOfficialBrowserApi(runtime), [runtime, suppliedOfficialApi]);
  const [account, setAccount] = useState(() => emptyOfficialBrowserSnapshot(runtime));
  const [connectionMode, setConnectionMode] = useState<ConnectionMode>("mado");
  const [inputs, setInputs] = useState(unifiedLive ? [""] : ["", "", "", ""]);
  const [mode, setMode] = useState<"paired" | "chats">("paired");
  const [appliedMode, setAppliedMode] = useState<"paired" | "chats">("paired");
  const [lead, setLead] = useState(0);
  const [snapshot, setSnapshot] = useState(emptyMultiview);
  const immersive = unifiedLive && snapshot.active && !tutorialActive;
  useEffect(() => { onWatchingChange?.(snapshot.active); }, [snapshot.active, onWatchingChange]);
  useEffect(() => () => onWatchingChange?.(false), [onWatchingChange]);
  const [settings, setSettings] = useState(false);
  const [inlineDraft, setInlineDraft] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const [layout, setLayout] = useState(readMadoLayout);
  const [arrangement, setArrangement] = useState<"channels" | "grouped">(() => {
    try { return localStorage.getItem("atsumi.live.arrangement.v1") === "grouped" ? "grouped" : "channels"; } catch { return "channels"; }
  });
  useEffect(() => { try { localStorage.setItem("atsumi.live.arrangement.v1", arrangement); } catch { /* Optional presentation preference. */ } }, [arrangement]);
  const [selectedChat, setSelectedChat] = useState<string | null>(null);
  const [chatHidden, setChatHidden] = useState(false);
  const [connectionDismissed, setConnectionDismissed] = useState(false);
  useEffect(() => {
    if (runtime !== "tauri") return;
    let disposed = false, unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/event").then(({ listen }) => listen<{ action: string; channelId: string }>("chzzk-presentation", async ({ payload }) => {
      if (disposed || !snapshotRef.current.active || !snapshotRef.current.panes.some(p => p.channelId === payload.channelId)) return;
      if (payload.action === "chat_toggle") setChatHidden(value => !value);
      if (payload.action === "fullscreen_toggle" || payload.action === "fullscreen_exit") {
        try { const { getCurrentWindow } = await import("@tauri-apps/api/window"); const window = getCurrentWindow(); await window.setFullscreen(payload.action === "fullscreen_exit" ? false : !await window.isFullscreen()); }
        catch { setError("전체화면을 전환하지 못했습니다."); }
      }
    })).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, [runtime]);
  const [resizing, setResizing] = useState(false);
  const [paneResizing, setPaneResizing] = useState(false);
  const captureChat = account.captureChatEnabled !== false;
  const [expiredControl, setExpiredControl] = useState<string | null>(null);
  const [dismissedControl, setDismissedControl] = useState<string | null>(null);
  const surfaces = useRef<HTMLDivElement>(null), cancelControl = useRef<HTMLButtonElement>(null);
  const settingsClose = useRef<HTMLButtonElement>(null);
  const settingsPanel = useRef<HTMLDivElement>(null);
  const bottomHandle = useRef<HTMLButtonElement>(null);
  const restoreHandleFocus = useRef(false);
  const settingsPriorFocus = useRef<HTMLElement | null>(null);
  const focusSettings = useRef(false);
  const drag = useRef<{ pointerId: number; direction: MadoLayout["direction"]; bounds: DOMRect } | null>(null);
  const dragFrame = useRef(0), nextRatio = useRef<{ value: number; direction: MadoLayout["direction"] } | null>(null);
  const seenUiAction = useRef<string | null>(null);
  const snapshotRef = useRef(snapshot); snapshotRef.current = snapshot;
  const lifetime = useRef(0), inFlight = useRef(false), requestRevision = useRef(0), edited = useRef(false);
  const activeRecording = snapshot.panes.some(isRecording);
  const layoutLocked = snapshot.active && snapshot.panes.some(pane => isRecording(pane) && !pane.receiverBacked);
  const requestedControl = snapshot.pendingControl;
  const controlPane = snapshot.panes.find((pane) => pane.paneId === requestedControl?.paneId && pane.channelId === requestedControl.channelId && pane.kind === "video");
  const control = api.runtime === "tauri" && requestedControl && controlPane && requestedControl.action === "screenshot"
    && requestedControl.id !== expiredControl && requestedControl.id !== dismissedControl && Number.isFinite(requestedControl.expiresAt) && requestedControl.expiresAt > Date.now() ? requestedControl : null;
  const canApprove = !!control && !pending && (control.action === "record_stop" ? isRecording(controlPane!) && controlPane!.recordingStatus !== "stopping"
    : !pollError && controlPane!.ready === true && (control.action !== "record_start" || !isRecording(controlPane!)));
  // The guide may explain connection controls without revealing or starting video/chat.
  const showingConnection = (!privacy || (tutorialActive && settings)) && !control && (settings || inlineDraft || !unifiedLive && !snapshot.active && !connectionDismissed);
  const editDraft = () => { edited.current = true; if (!settings) setInlineDraft(true); };
  const openSettings = () => {
    if (!unifiedLive) setConnectionMode("mado");
    if (settings) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // The sidebar slides away after opening settings. Restore keyboard focus
    // to its edge trigger rather than a hidden navigation button.
    settingsPriorFocus.current = opener?.closest('.live-edge-panel.is-enabled')?.querySelector<HTMLButtonElement>('.live-edge-trigger') ?? opener;
    focusSettings.current = true;
    setSettings(true);
  };
  const closeSettings = () => {
    setConnectionDismissed(true);
    if (settingsPanel.current?.contains(document.activeElement)) {
      if (settingsPriorFocus.current === bottomHandle.current) restoreHandleFocus.current = true;
      else if (settingsPriorFocus.current?.isConnected) settingsPriorFocus.current.focus();
    }
    focusSettings.current = false;
    setInlineDraft(false);
    setSettings(false);
  };
  const handleGesture = usePanelHandleGesture(showingConnection, "y", open => { if (open) openSettings(); else closeSettings(); });
  useEffect(() => {
    if (!showingConnection && restoreHandleFocus.current) {
      restoreHandleFocus.current = false;
      bottomHandle.current?.focus();
    }
  }, [showingConnection]);
  useEffect(() => {
    if (!connectionRequest) return;
    if (settings) closeSettings(); else openSettings();
    onConnectionRequestHandled?.();
  }, [connectionRequest, onConnectionRequestHandled]);
  useEffect(() => { onConnectionOpenChange?.(settings && showingConnection); }, [settings, showingConnection, onConnectionOpenChange]);
  useEffect(() => () => onConnectionOpenChange?.(false), [onConnectionOpenChange]);
  useEffect(() => {
    const advanceGuide = () => {
      if (!["chzzk-connection", "chzzk-more", "chzzk-auto", "chzzk-auto-info"].includes(document.documentElement.dataset.tutorialStep ?? "")) return;
      focusSettings.current = false;
      setInlineDraft(false);
      setSettings(true);
    };
    window.addEventListener(tutorialStepEvent, advanceGuide);
    return () => window.removeEventListener(tutorialStepEvent, advanceGuide);
  }, []);
  useEffect(() => {
    if (settings && !privacy && !control && focusSettings.current) {
      settingsClose.current?.focus();
      focusSettings.current = false;
    }
  }, [settings, privacy, control?.id]);
  useEffect(() => {
    if (!showingConnection || runtime !== "tauri") return;
    let disposed = false, refreshSequence = 0;
    const refresh = async () => {
      if (inFlight.current) return;
      const revision = requestRevision.current, sequence = ++refreshSequence;
      const current = () => !disposed && sequence === refreshSequence && revision === requestRevision.current && !inFlight.current;
      try {
        const result = await officialApi.snapshot();
        if (!current()) return;
        if (result.ok && result.data) setAccount(result.data);
        else if (!result.ok && result.error.code === "BROWSER_INITIALIZING") setAccount((previous) => ({ ...previous, authStatus: "unknown", authChecking: true, authError: null }));
        else setAccount((previous) => ({ ...previous, authStatus: "unknown", authChecking: false, authError: result.ok ? "로그인 상태 응답이 없습니다." : result.error.message }));
      } catch {
        if (current()) setAccount((previous) => ({ ...previous, authStatus: "unknown", authChecking: false, authError: "로그인 상태를 확인하지 못했습니다. 다시 확인해 주세요." }));
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 2000);
    return () => { disposed = true; clearInterval(timer); };
  }, [officialApi, runtime, showingConnection]);
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
              setMode(restoredMode);
              if (unifiedLive) setConnectionMode("mado");
              setInputs(Array.from({ length: unifiedLive ? Math.max(1, channels.length) : 4 }, (_, i) => channels[i] ?? ""));
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
    if (layoutLocked) { setError("기존 시청 방식의 녹화를 먼저 중지해 주세요."); return; }
    if (connectionMode === "general" && !unifiedLive) {
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
    const entries = multiviewEntries(connectionMode === "general" ? [inputs[0] ?? ""] : inputs, connectionMode === "general" ? "paired" : mode, connectionMode === "general" ? 0 : lead);
    if (typeof entries === "string") { setError(entries); return; }
    const generation = lifetime.current;
    let result = await api.configure(entries);
    // Contention is transient, not a successful configuration. Keep the old
    // panes alive and retry this exact draft, never a later edited selection.
    for (const delay of [250, 500, 1000]) {
      if (result.ok || result.error.code !== "MULTIVIEW_BUSY" || !result.error.retryable) break;
      await new Promise(resolve => setTimeout(resolve, delay));
      if (generation !== lifetime.current) return;
      result = await api.configure(entries);
    }
    if (generation !== lifetime.current) { if (result.ok) void api.close(result.data.epoch).catch(() => {}); return; }
    if (!result.ok) { setError(result.error.message); return; }
    setSnapshot(result.data); snapshotRef.current = result.data; setAppliedMode(connectionMode === "general" ? "paired" : mode); closeSettings();
  });
  const leave = () => void run(async (isCurrent) => {
    if (layoutLocked) { setError("기존 시청 방식의 녹화를 먼저 중지해 주세요."); return; }
    if (snapshot.active) { const result = await api.close(snapshot.epoch); if (!isCurrent()) return; if (!result.ok) { setError(result.error.message); return; } snapshotRef.current = result.data; setSnapshot(result.data); }
    if (!isCurrent()) return;
    // Keep the editable draft after closing. A late status response must never
    // restore the just-closed channels over the user's next broadcast selection.
    edited.current = true;
    closeSettings();
    if (unifiedLive) setSettings(true);
    if (!unifiedLive) onLeave();
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
      if (api.runtime !== "tauri" || privacy || pending || pollError || control || hasBlockingNativeOverlay() || event.key.toLowerCase() !== "s" || event.defaultPrevented || event.repeat || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
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
      nextRatio.current = { value: (offset - 5) / Math.max(1, size - 10) * 100, direction: current.direction };
      if (!dragFrame.current) dragFrame.current = requestAnimationFrame(() => { dragFrame.current = 0; const next = nextRatio.current; nextRatio.current = null; if (next) updateRatio(next.value, next.direction); });
    }
  };
  const endDivider = () => { cancelAnimationFrame(dragFrame.current); dragFrame.current = 0; const next = nextRatio.current; nextRatio.current = null; if (next) updateRatio(next.value, next.direction); drag.current = null; setResizing(false); };
  useEffect(() => {
    if (!resizing) return;
    const finish = () => endDivider();
    const visibility = () => { if (document.hidden) finish(); };
    window.addEventListener("blur", finish);
    document.addEventListener("visibilitychange", visibility);
    return () => { window.removeEventListener("blur", finish); document.removeEventListener("visibilitychange", visibility); };
  }, [resizing]);
  const dividerKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const down = layout.direction === "horizontal" ? "ArrowLeft" : "ArrowUp", up = layout.direction === "horizontal" ? "ArrowRight" : "ArrowDown";
    if (![down, up, "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    updateRatio(event.key === "Home" ? 10 : event.key === "End" ? 94 : layout[layout.direction] + (event.key === down ? -1 : 1) * (event.shiftKey ? 10 : 2));
  };
  const channels = [...new Set(snapshot.panes.map((pane) => pane.channelId))];
  const activeChat = selectedChat && channels.includes(selectedChat) ? selectedChat : channels[0] ?? "";
  const number = (channelId: string) => channels.indexOf(channelId) + 1;
  const label = (channelId: string) => `${number(channelId)}. ${privacy ? "방송" : channelName(snapshot.panes.find((pane) => pane.channelId === channelId))}`;
  const renderPane = (pane: MultiviewPane) => <NativeStreamingPane key={pane.paneId} pane={pane} epoch={snapshot.epoch} api={api} privacy={privacy} inputBlocked={resizing || paneResizing} onError={setError} />;
  const connectionSetup = <ConnectionSetup mode={connectionMode} onMode={value => { editDraft(); setConnectionMode(value); }} modeDisabled={pending || layoutLocked}
      runtime={runtime} privacy={privacy}
      knownChannelNames={Object.fromEntries(snapshot.panes.filter(pane => pane.channelName).map(pane => [pane.channelId, pane.channelName!]))}
      unified={unifiedLive}
      watching={snapshot.active} onStop={leave}
      connectDisabled={layoutLocked}
      onAddChannel={() => { editDraft(); setInputs(rows => rows.length < 4 ? [...rows, ""] : rows); }}
      onRemoveChannel={index => { editDraft(); setInputs(rows => rows.length > 1 ? rows.filter((_, i) => i !== index) : rows); setLead(0); }}
      receiverBacked={unifiedLive}
      channelPicker={unifiedLive ? <LiveChannelPicker runtime={runtime} inputs={inputs} multiple privacy={privacy} disabled={pending || runtime !== "tauri"} onInputs={next => { editDraft(); setInputs(next); }} /> : undefined}
      inputs={inputs} onInput={(index, value) => { editDraft(); setInputs((rows) => rows.map((row, i) => i === index ? value : row)); }} disabled={pending || runtime !== "tauri"} pending={pending} onConnect={apply} onClose={settings || unifiedLive ? closeSettings : undefined} closeRef={settingsClose}
      auth={accountStatus(account)} accountDisabled={pending || account.accountBusy === true || runtime !== "tauri"} logoutDisabled={activeRecording}
      accountReason={activeRecording ? "녹화 중에도 로그인할 수 있습니다. 로그아웃은 녹화 종료 후 가능합니다." : undefined}
      onLogin={() => void run(async () => { const result = await officialApi.login(); if (result.ok) setAccount(result.data); else setError(result.error.message); })}
      onLogout={() => void run(async () => { const result = await officialApi.logout(); if (result.ok) setAccount(result.data); else setError(result.error.message); })}
      authChecking={account.authChecking} authError={account.authError}
      onGrid={() => void run(async () => { const result = await officialApi.connectExtension(); if (result.ok) setAccount(result.data); else setError(result.error.message); })}
      gridDisabled={pending || activeRecording || runtime !== "tauri" || (!account.windowOpen && !snapshot.active)} gridStatus={account.extensionStatus === "not_connected" ? "미연결" : account.extensionStatus ?? "상태 확인 전"}
      onInstaller={(browser) => void run(async () => { const result = await officialApi.openInstaller(browser); if (!result.ok) setError(result.error.message); })} installerDisabled={pending || activeRecording || runtime !== "tauri"}
      options={names => <><div className="mado-layout-options" role="group" aria-label="영상 표시" data-mode={mode}><button type="button" aria-pressed={mode === "paired"} disabled={pending} onClick={() => { editDraft(); setMode("paired"); }}>모두 보기</button><button type="button" aria-pressed={mode === "chats"} disabled={pending} onClick={() => { editDraft(); setMode("chats"); if (!inputs[lead]?.trim()) setLead(Math.max(0, inputs.findIndex(input => !!input.trim()))); }}>선택 보기</button></div>{mode === "chats" ? <label className="mado-broadcast-select">
        <span>영상 방송</span>
        <span className="mado-broadcast-select-control">
          <select value={lead} disabled={pending} onChange={(event) => { editDraft(); setLead(Number(event.target.value)); }}>{names.map((name, index) => <option key={index} value={index} disabled={!inputs[index]?.trim()}>{name}</option>)}</select>
          <FluentIcon glyph="\uE70D" />
        </span>
      </label> : <label className="mado-broadcast-select"><span>화면 배치</span><span className="mado-broadcast-select-control"><select aria-label="화면 배치" value={arrangement} onChange={event => setArrangement(event.target.value as "channels" | "grouped")}><option value="channels">방송 · 채팅 / 방송 · 채팅</option><option value="grouped">방송 모아 보기 / 채팅 모아 보기</option></select><FluentIcon glyph="\uE70D" /></span></label>}</>}
    />;
  return <section className={`mado-workspace${immersive ? " is-immersive" : ""}`} aria-label={unifiedLive ? "라이브 시청" : "마도 모드"}>
    {!unifiedLive ? <header className="mado-toolbar"><strong>마도</strong>
      <button type="button" data-tour="chzzk-channels" onClick={openSettings}>연결 설정</button>
      {snapshot.active ? <button type="button" onClick={() => setLayout((current) => ({ ...current, direction: current.direction === "horizontal" ? "vertical" : "horizontal" }))} title="영상과 채팅의 배치 방향을 바꿉니다. 분할선은 끌거나 방향키로 조절할 수 있습니다.">{layout.direction === "horizontal" ? "위아래 배치" : "좌우 배치"}</button> : null}
    </header> : null}
    {unifiedLive ? <div className="mado-connection-dock"><button ref={bottomHandle} type="button" hidden={!!control}
      className="live-bottom-handle"
      data-native-overlay="true" data-native-preserve-video="true" data-native-interactive-background="true" data-tour="chzzk-channels"
      aria-label={`${snapshot.active ? "방송 조작" : "방송 보기"} ${showingConnection ? "닫기" : "열기"}`} aria-expanded={showingConnection} aria-controls={showingConnection ? "chzzk-connection-panel" : undefined}
      title={showingConnection ? "누르거나 아래로 끌어 닫기" : "누르거나 위로 끌어 열기"} {...handleGesture}
      onKeyDown={event => { if (event.key === "Escape" && showingConnection) { event.preventDefault(); event.stopPropagation(); closeSettings(); } }}>
      <svg viewBox="0 0 24 16" aria-hidden="true"><path d={showingConnection ? "m6 5 6 5 6-5" : "m6 11 6-5 6 5"} /></svg>
    </button>
    {showingConnection ? <div id="chzzk-connection-panel" ref={settingsPanel} className="official-browser-dialog-backdrop mado-connection-panel" role="dialog" aria-modal="false" data-native-overlay="true" data-native-preserve-video="true" data-native-interactive-background="true" aria-label="연결 설정" onKeyDown={event => {
      if (event.key === "Escape" && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); closeSettings(); }
    }}><div className="live-connection-surface" data-native-dialog-surface="true">{connectionSetup}</div></div> : null}
    </div> : null}
    {!unifiedLive && showingConnection ? settings ? <div id="chzzk-connection-panel" ref={settingsPanel} className="official-browser-dialog-backdrop mado-connection-panel" role="dialog" aria-modal="false" data-native-overlay="true" data-native-preserve-video="true" data-native-interactive-background="true" aria-label="연결 설정" onKeyDown={(event) => {
      if (event.key === "Escape" && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); closeSettings(); }
    }}>{connectionSetup}</div> : <div className="mado-connection-inline">
      <p className="mado-connection-intro">{unifiedLive ? "저장한 채널을 선택하거나 방송 주소를 입력해 시청을 시작하세요. 저장한 영상은 녹화 목록에서 볼 수 있습니다." : "방송 주소를 입력하고 원하는 배치로 시청을 시작하세요."}</p>
      {connectionSetup}
    </div> : null}
    {(error || pollError) && !control ? <div className="mado-error" role="alert" data-native-overlay="true"><span>{error || pollError}</span>{error ? <button type="button" aria-label="오류 닫기" onClick={() => setError(null)}>×</button> : null}</div> : null}
    <div className={`mado-content-surfaces${chatHidden ? " chat-hidden" : ""}`}>
    {snapshot.active && appliedMode === "paired" && channels.length > 1 && arrangement === "channels" ?
      <LivePaneGrid kind="pair" className={`mado-surfaces live-channel-pairs${paneResizing ? " is-resizing" : ""}`} onResizing={setPaneResizing} accessory={
        <div className="mado-chat-tabs live-pair-tabs" role="tablist" aria-label="볼 채팅" onKeyDown={event => {
          if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
          event.preventDefault(); const index = channels.indexOf(activeChat);
          const next = event.key === "Home" ? 0 : event.key === "End" ? channels.length - 1 : (index + (event.key === "ArrowLeft" ? -1 : 1) + channels.length) % channels.length;
          setSelectedChat(channels[next] ?? null); event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
        }}>{channels.map(channelId => <button key={channelId} type="button" role="tab" aria-selected={activeChat === channelId} aria-controls={`mado-chat-panel-${channelId}`} tabIndex={activeChat === channelId ? 0 : -1} title={label(channelId)} onClick={() => setSelectedChat(channelId)}>{label(channelId)}</button>)}</div>
      }>{channels.map((channelId, index) => <div className="live-channel-pair" key={channelId} data-channel-index={index} aria-label={label(channelId)}>
        <LivePaneGrid kind="channel" storageId={`channel-${index}`} label={`${index + 1}번 방송과 채팅`} className="live-channel-pair-content" onResizing={setPaneResizing}>{snapshot.panes.filter(pane => pane.channelId === channelId).sort((a, b) => a.kind === b.kind ? 0 : a.kind === "video" ? -1 : 1).map(pane =>
          <div key={pane.paneId} className={pane.kind === "video" ? "mado-lead-inner" : "mado-chat-cell"} id={pane.kind === "chat" ? `mado-chat-panel-${channelId}` : undefined} data-chat-selected={pane.kind === "chat" ? activeChat === channelId : undefined}>{renderPane(pane)}</div>
        )}</LivePaneGrid>
      </div>)}</LivePaneGrid>
    : snapshot.active ? <div ref={surfaces} className={`mado-surfaces is-${appliedMode} direction-${layout.direction}${resizing || paneResizing ? " is-resizing" : ""}`} data-channels={channels.length} style={{ "--mado-main-share": `${layout[layout.direction]}fr`, "--mado-chat-share": `${100 - layout[layout.direction]}fr` } as CSSProperties}>
        <LivePaneGrid kind="video" className="mado-lead" onResizing={setPaneResizing}>{snapshot.panes.filter((pane) => pane.kind === "video").map((pane) => <div className="mado-lead-inner" key={pane.paneId}>{renderPane(pane)}</div>)}</LivePaneGrid>
        <div className="mado-divider" role="separator" tabIndex={0} aria-label="영상과 채팅 크기" aria-orientation={layout.direction === "horizontal" ? "vertical" : "horizontal"} aria-valuemin={10} aria-valuemax={94} aria-valuenow={layout[layout.direction]} aria-valuetext={`영상 ${layout[layout.direction]}%, 채팅 ${100 - layout[layout.direction]}%`} onKeyDown={dividerKey}
          onPointerDown={(event) => { if (event.button !== 0 || !surfaces.current) return; event.preventDefault(); drag.current = { pointerId: event.pointerId, direction: layout.direction, bounds: surfaces.current.getBoundingClientRect() }; event.currentTarget.setPointerCapture?.(event.pointerId); setResizing(true); }}
          onPointerMove={moveDivider} onPointerUp={endDivider} onPointerCancel={endDivider} onLostPointerCapture={endDivider} />
        <div className="mado-chat-area">
          <div className="mado-chat-tabs" role="tablist" aria-label="볼 채팅" onKeyDown={event => {
            if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
            event.preventDefault();
            const index = channels.indexOf(activeChat), next = event.key === "Home" ? 0 : event.key === "End" ? channels.length - 1 : (index + (event.key === "ArrowLeft" ? -1 : 1) + channels.length) % channels.length;
            setSelectedChat(channels[next] ?? null);
            event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
          }}>{channels.map(channelId => <button type="button" key={channelId} id={`mado-chat-tab-${channelId}`} role="tab" aria-selected={activeChat === channelId} aria-controls={`mado-chat-panel-${channelId}`} tabIndex={activeChat === channelId ? 0 : -1} title={label(channelId)} onClick={() => setSelectedChat(channelId)}>{label(channelId)}</button>)}</div>
          <LivePaneGrid kind="chat" className="mado-chat-wall" onResizing={setPaneResizing}>{snapshot.panes.filter((pane) => pane.kind === "chat").map((pane) => <div className="mado-chat-cell" key={pane.paneId} id={`mado-chat-panel-${pane.channelId}`} data-chat-selected={activeChat === pane.channelId} aria-label={label(pane.channelId)}>{renderPane(pane)}</div>)}</LivePaneGrid>
        </div>
    </div> : unifiedLive && !showingConnection && !control ? <div className="mado-live-empty">
      <div className="live-empty-copy">
        <svg className="live-empty-icon" viewBox="0 0 32 32" aria-hidden="true"><rect x="3" y="5" width="26" height="19" rx="3" /><path d="m13 10 8 4.5-8 4.5zM11 28h10" /></svg>
        <h2>방송을 시청해 보세요</h2>
        <p>{privacy ? <>프라이버시 모드를 끈 뒤<br />아래 손잡이를 눌러주세요.</> : <>아래 손잡이를 누르거나 끌어올려 방송을 선택하세요.</>}</p>
      </div>
      <svg className="live-empty-direction" viewBox="0 0 24 64" aria-hidden="true"><path d="M12 2v56m-7-7 7 7 7-7" /></svg>
    </div> : privacy ? <p className="mado-empty">프라이버시 모드</p> : null}
    </div>
    {control ? <div className="official-browser-dialog-backdrop" role="alertdialog" aria-modal="true" data-native-preserve-video="true" aria-labelledby="mado-control-title" onKeyDown={controlKeys}>
      <div className="official-browser-dialog" data-native-dialog-surface="true"><h3 id="mado-control-title">화면을 저장할까요?</h3><p>{label(control.channelId)} · 스크린샷을 저장합니다.</p>
        {error || pollError ? <p role="alert" className="official-browser-error">{error || pollError}</p> : null}
        <div><button ref={cancelControl} type="button" disabled={pending} onClick={() => confirm(false)}>취소</button><button type="button" disabled={!canApprove} onClick={() => confirm(true)}>{pending ? "처리 중…" : "저장 확인"}</button></div>
      </div>
    </div> : null}
  </section>;
}
