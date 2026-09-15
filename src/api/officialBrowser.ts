import { invoke } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";

export type BrowserControlAction = "record_start" | "record_stop" | "screenshot";

export type BrowserRecordingSegment = {
  index: number;
  file: string;
  bytes: number;
  durationSeconds: number;
};

export type BrowserMerge = {
  status: "queued" | "merging" | "complete" | "blocked" | "failed";
  segmentCount: number;
  updatedAt: number;
  file?: string | null;
  timelineFile?: string | null;
  bytes?: number | null;
  durationSeconds?: number | null;
  lastError?: string | null;
  sourceCleanup?: {
    status: "pending" | "complete" | "blocked";
    deletedSegments: number;
    proofFile: string;
    proofSha256: string;
    lastError?: string | null;
  } | null;
};

export type BrowserRecording = {
  id: string;
  channelId: string;
  title: string;
  startedAt: number;
  updatedAt: number;
  status: "recording" | "stopped" | "interrupted" | "failed";
  mimeType: string;
  outputDir: string;
  segmentCount: number;
  bytesWritten: number;
  durationSeconds: number;
  lastError: string | null;
  segments: BrowserRecordingSegment[];
  partial?: Pick<BrowserRecordingSegment, "index" | "file" | "bytes"> | null;
  captureChat?: boolean;
  chatStatus?: string;
  chatCount?: number;
  merge?: BrowserMerge | null;
  deletionPending?: boolean;
};

export type BrowserDeleteReport = {
  deletedIds: string[];
  failures: { id: string; error: { code: string; message: string; retryable: boolean } }[];
};

export type OfficialBrowserSnapshot = {
  captureChatEnabled?: boolean;
  windowOpen: boolean;
  channelId: string | null;
  ready: boolean;
  status: string;
  error: string | null;
  recordingId: string | null;
  recordings: BrowserRecording[];
  extensionStatus?: string;
  chatStatus?: string;
  chatCount?: number;
  loginStatus?: string;
  authStatus?: "signed_in" | "signed_out" | "unknown" | "checking";
  authChecking?: boolean;
  authError?: string | null;
  accountBusy?: boolean;
  videoWidth?: number;
  videoHeight?: number;
  videoPaused?: boolean;
  viewportEpoch?: number;
  diagnostics?: unknown;
  captureDiagnostics?: { reason: string; installed: boolean; appendCount: number; appendBytes: number } | null;
  pendingControl?: { id: string; action: "record_start" | "record_stop" | "screenshot"; channelId: string; expiresAt: number } | null;
  pendingUiAction?: { id: string; action: "exit_focus" | "toggle_focus" | "open_settings"; expiresAt: number } | null;
  lastScreenshot?: { id: string; channelId: string; fileName: string; createdAt: number } | null;
};

/** CSS logical pixels relative to the main window client area. Clip is stage-local. */
export type OfficialBrowserViewport = {
  x: number;
  y: number;
  width: number;
  height: number;
  visible: boolean;
  /** Mask remote pixels/input for trusted popups without suspending playback. */
  occluded?: boolean;
  /** Retain background paint while disabling input throughout the native view. */
  preserveBackground?: boolean;
  /** At most eight stage-local popup rectangles subtracted from the scroll clip. */
  occlusions?: { x: number; y: number; width: number; height: number }[];
  epoch?: number;
  /** Native transport assigns a monotonic sequence within this main document. */
  requestSequence?: number;
  clip?: { x: number; y: number; width: number; height: number };
};

export interface OfficialBrowserApi {
  readonly runtime: "tauri" | "browser-mock";
  open(input: string): Promise<ApiResult<OfficialBrowserSnapshot>>;
  snapshot(refreshAuth?: boolean): Promise<ApiResult<OfficialBrowserSnapshot>>;
  start(options: { rightsAcknowledged: boolean; captureChat: boolean }): Promise<ApiResult<OfficialBrowserSnapshot>>;
  stop(): Promise<ApiResult<OfficialBrowserSnapshot>>;
  setCaptureChat?(enabled: boolean): Promise<ApiResult<OfficialBrowserSnapshot>>;
  connectExtension(): Promise<ApiResult<OfficialBrowserSnapshot>>;
  login(): Promise<ApiResult<OfficialBrowserSnapshot>>;
  logout(): Promise<ApiResult<OfficialBrowserSnapshot>>;
  openInstaller(browser?: "chrome" | "edge"): Promise<ApiResult<void>>;
  setViewport(viewport: OfficialBrowserViewport): Promise<ApiResult<void>>;
  confirmControl(options: { requestId: string; approve: boolean; rightsAcknowledged: boolean; captureChat: boolean }): Promise<ApiResult<OfficialBrowserSnapshot>>;
  requestControl(action: BrowserControlAction): Promise<ApiResult<OfficialBrowserSnapshot>>;
  ackUiAction(id: string): Promise<ApiResult<OfficialBrowserSnapshot>>;
  openFolder(recordingId: string): Promise<ApiResult<void>>;
  openSegment(recordingId: string, index: number): Promise<ApiResult<void>>;
  openMerged(recordingId: string): Promise<ApiResult<void>>;
  retryMerge(recordingId: string): Promise<ApiResult<OfficialBrowserSnapshot>>;
  deleteRecordings(recordingIds: string[]): Promise<ApiResult<BrowserDeleteReport>>;
}

export const emptyOfficialBrowserSnapshot = (runtime: OfficialBrowserApi["runtime"]): OfficialBrowserSnapshot => ({
  windowOpen: false, channelId: null, ready: false,
  status: runtime === "browser-mock" ? "desktop_required" : "closed",
  error: null, recordingId: null, recordings: [], loginStatus: "unknown", viewportEpoch: 0,
});

const unavailable = <T>(): ApiResult<T> => ({
  ok: false,
  error: {
    code: "CHZZK_BROWSER_DESKTOP_REQUIRED",
    message: "공식 CHZZK 시청 창과 화면 녹화는 데스크톱 앱에서 사용할 수 있습니다.",
    retryable: false,
  },
});

async function call<T>(command: string, args: Record<string, unknown> = {}): Promise<ApiResult<T>> {
  try { return await invoke<ApiResult<T>>(command, args); }
  catch {
    return { ok: false, error: {
      code: "CHZZK_BROWSER_TRANSPORT_ERROR",
      message: "공식 시청 창의 상태를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.",
      retryable: true,
    } };
  }
}

// Account/window mutations share one queue across API lifetimes. Viewport writes
// use a separate fast lane: privacy/modal hides must not wait for account work.
// The native host retains the latest viewport even before a child exists.
let nativeMutationTail: Promise<unknown> = Promise.resolve();
type ViewportTransportSession = { tail: Promise<unknown>; generation: number; epoch: number };
// HMR is not a new main document: preserve its native sequence fence and the
// old closures' shared state. A real reload starts anew with a new native epoch.
const nativeViewportSession: ViewportTransportSession = import.meta.hot?.data?.officialViewportTransport
  ?? { tail: Promise.resolve(), generation: 0, epoch: 0 };
if (import.meta.hot?.data) import.meta.hot.data.officialViewportTransport = nativeViewportSession;
function mutate<T>(command: string, args: Record<string, unknown> = {}): Promise<ApiResult<T>> {
  const result = nativeMutationTail.then(() => call<T>(command, args));
  nativeMutationTail = result.then(() => undefined, () => undefined);
  return result;
}
function mutateViewport(viewport: OfficialBrowserViewport): Promise<ApiResult<void>> {
  const generation = ++nativeViewportSession.generation;
  if (viewport.epoch !== undefined) nativeViewportSession.epoch = Math.max(nativeViewportSession.epoch, viewport.epoch);
  const request = { ...viewport, epoch: viewport.epoch ?? nativeViewportSession.epoch, requestSequence: generation };
  // Privacy must invalidate an in-flight native layout without waiting for its
  // ACK. Also discard any not-yet-dispatched show superseded by that hide.
  if (!viewport.visible || viewport.occluded) return call<void>("chzzk_browser_set_viewport", { viewport: request });
  const result = nativeViewportSession.tail.then(() => generation === nativeViewportSession.generation
    ? call<void>("chzzk_browser_set_viewport", { viewport: request })
    : { ok: true as const, data: undefined });
  nativeViewportSession.tail = result.then(() => undefined, () => undefined);
  return result;
}

export const hiddenOfficialBrowserViewport: OfficialBrowserViewport = { x: 0, y: 0, width: 0, height: 0, visible: false };
type ViewportRequest = {
  viewport: OfficialBrowserViewport;
  key: string;
  owner: number;
  revision: number;
  attempt: number;
  inFlight: boolean;
  report?: (message: string | null) => void;
};
type ViewportCoordinator = {
  owner: number;
  revision: number;
  running: boolean;
  pending?: ViewportRequest;
  target?: ViewportRequest;
  retry?: ReturnType<typeof setTimeout>;
  last?: string;
};
const viewportCoordinators = new WeakMap<OfficialBrowserApi, ViewportCoordinator>();

/** One visible write + latest rectangle; hides preempt it, retries are bounded. */
export function claimOfficialBrowserViewport(api: OfficialBrowserApi, report?: (message: string | null) => void) {
  let coordinator = viewportCoordinators.get(api);
  if (!coordinator) {
    coordinator = { owner: 0, revision: 0, running: false };
    viewportCoordinators.set(api, coordinator);
  }
  const state = coordinator;
  const owner = ++state.owner;
  state.revision += 1;
  clearTimeout(state.retry);
  state.retry = undefined;
  let released = false;
  const current = (next: ViewportRequest) => state.owner === next.owner && state.revision === next.revision;
  const write = async (next: ViewportRequest) => {
    next.inFlight = true;
    if (current(next)) state.last = undefined;
    try {
      const result = await api.setViewport(next.viewport);
      if (!current(next)) return; // An old ACK cannot clear a newer hide/error.
      if (!result.ok && next.viewport.occluded) {
        void api.setViewport({ ...hiddenOfficialBrowserViewport, epoch: next.viewport.epoch }).catch(() => {});
      }
      state.last = result.ok ? next.key : undefined;
      if (!result.ok && next.viewport.visible && ["VIEWPORT_BUSY", "VIEWPORT_CLIP_FAILED"].includes(result.error.code) && next.attempt < 2) {
        state.retry = setTimeout(() => {
          state.retry = undefined;
          if (!current(next)) return;
          const retry = { ...next, attempt: next.attempt + 1, inFlight: false };
          state.target = retry;
          state.pending = retry;
          void drain();
        }, next.attempt === 0 ? 120 : 320);
      }
      next.report?.(result.ok ? null : result.error.message);
    } catch {
      if (current(next) && next.viewport.occluded) void api.setViewport({ ...hiddenOfficialBrowserViewport, epoch: next.viewport.epoch }).catch(() => {});
      if (current(next)) { state.last = undefined; next.report?.("시청 화면을 표시하지 못했습니다. 화면을 다시 열어 주세요."); }
    } finally { next.inFlight = false; }
  };
  const drain = async () => {
    if (state.running) return;
    state.running = true;
    try {
      while (state.pending) {
        const next = state.pending;
        state.pending = undefined;
        if (current(next)) await write(next);
      }
    } finally { state.running = false; }
  };
  const submit = (viewport: OfficialBrowserViewport, requestOwner: number, notify?: (message: string | null) => void) => {
    clearTimeout(state.retry);
    state.retry = undefined;
    const next: ViewportRequest = { viewport, key: JSON.stringify(viewport), owner: requestOwner, revision: ++state.revision, attempt: 0, inFlight: false, report: notify };
    state.target = next;
    state.pending = undefined;
    state.last = undefined;
    if (!viewport.visible || viewport.occluded) void write(next);
    else { state.pending = next; void drain(); }
  };
  return {
    update(viewport: OfficialBrowserViewport) {
      if (released || state.owner !== owner || api.runtime !== "tauri") return;
      const key = JSON.stringify(viewport);
      if (state.target?.owner === owner && state.target.key === key
        && (state.target.inFlight || state.pending || state.retry || state.last === key)) {
        if (state.last === key) report?.(null);
        return;
      }
      submit(viewport, owner, report);
    },
    release() {
      if (released) return;
      released = true;
      if (state.owner !== owner || api.runtime !== "tauri") return;
      const epoch = state.target?.viewport.epoch;
      state.owner += 1; // Invalidate callbacks belonging to the departing component.
      submit(epoch === undefined ? hiddenOfficialBrowserViewport : { ...hiddenOfficialBrowserViewport, epoch }, state.owner);
    },
  };
}

let desktopApi: OfficialBrowserApi | undefined;

export function createOfficialBrowserApi(runtime: OfficialBrowserApi["runtime"]): OfficialBrowserApi {
  if (runtime === "browser-mock") return {
    runtime,
    open: async () => unavailable(),
    snapshot: async () => ({ ok: true, data: emptyOfficialBrowserSnapshot(runtime) }),
    start: async () => unavailable(),
    stop: async () => unavailable(),
    connectExtension: async () => unavailable(),
    login: async () => unavailable(),
    logout: async () => unavailable(),
    openInstaller: async () => unavailable(),
    setViewport: async () => unavailable(),
    confirmControl: async () => unavailable(),
    requestControl: async () => unavailable(),
    ackUiAction: async () => unavailable(),
    openFolder: async () => unavailable(),
    openSegment: async () => unavailable(),
    openMerged: async () => unavailable(),
    retryMerge: async () => unavailable(),
    deleteRecordings: async () => unavailable(),
  };
  if (desktopApi) return desktopApi;
  desktopApi = {
    runtime,
    open: (input) => mutate("chzzk_browser_open", { input }),
    snapshot: (refreshAuth = false) => call("chzzk_browser_snapshot", refreshAuth ? { refreshAuth: true } : {}),
    start: (options) => mutate("chzzk_browser_start", options),
    stop: () => mutate("chzzk_browser_stop"),
    setCaptureChat: (enabled) => mutate("chzzk_browser_capture_chat", { enabled }),
    connectExtension: () => mutate("chzzk_browser_connect_extension"),
    login: () => mutate("chzzk_browser_login"),
    logout: () => mutate("chzzk_browser_logout"),
    openInstaller: (browser = "chrome") => mutate("chzzk_browser_open_installer", { browser }),
    setViewport: mutateViewport,
    confirmControl: (options) => mutate("chzzk_browser_confirm_control", options),
    requestControl: (action) => mutate("chzzk_browser_request_control", { action }),
    ackUiAction: (id) => mutate("chzzk_browser_ack_ui_action", { id }),
    openFolder: (recordingId) => mutate("chzzk_browser_open_folder", { recordingId }),
    openSegment: (recordingId, index) => mutate("chzzk_browser_open_segment", { recordingId, index }),
    openMerged: (recordingId) => mutate("chzzk_browser_open_merged", { recordingId }),
    retryMerge: (recordingId) => mutate("chzzk_browser_retry_merge", { recordingId }),
    deleteRecordings: (recordingIds) => mutate("chzzk_browser_delete_recordings", { recordingIds }),
  };
  return desktopApi;
}
