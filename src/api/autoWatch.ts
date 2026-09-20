import { invoke } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";
import { createOfficialBrowserApi, type OfficialBrowserApi, type OfficialBrowserSnapshot, type OfficialBrowserViewport } from "./officialBrowser";

export type AutoWatchTarget = {
  kind: "automatic" | "live" | "mado"; watchId: string | null;
  channelId: string; recordingId: string; channelName: string; epoch: number;
};
export type AutoWatchSnapshot = { recording: boolean; status: string; error: string | null; audioEnabled: boolean; chatCount: number };
export interface AutoWatchApi {
  readonly runtime: "tauri" | "browser-mock";
  open(channelId: string, recordingId: string): Promise<ApiResult<AutoWatchTarget>>;
  snapshot(watchId: string): Promise<ApiResult<AutoWatchSnapshot>>;
  setViewport(watchId: string, viewport: OfficialBrowserViewport): Promise<ApiResult<void>>;
  setAudio(watchId: string, enabled: boolean): Promise<ApiResult<AutoWatchSnapshot>>;
  close(watchId: string): Promise<ApiResult<void>>;
}
const transport: { sequence: number } = import.meta.hot?.data?.autoWatchTransport ?? { sequence: 0 };
if (import.meta.hot?.data) import.meta.hot.data.autoWatchTransport = transport;
export function createAutoWatchApi(runtime: AutoWatchApi["runtime"]): AutoWatchApi {
  async function call<T>(command: string, args: Record<string, unknown>): Promise<ApiResult<T>> {
    if (runtime !== "tauri") return { ok: false, error: { code: "DESKTOP_REQUIRED", message: "데스크톱 앱에서 실시간으로 볼 수 있습니다.", retryable: false } };
    try { return await invoke(command, args); }
    catch { return { ok: false, error: { code: "AUTO_WATCH_TRANSPORT", message: "실시간 보기 화면에 연결하지 못했습니다. 녹화 상태는 자동 녹화 목록에서 확인해 주세요.", retryable: true } }; }
  }
  return {
    runtime,
    open: (channelId, recordingId) => call("chzzk_auto_watch_open", { channelId, recordingId }),
    snapshot: (watchId) => call("chzzk_auto_watch_snapshot", { watchId }),
    setViewport: (watchId, viewport) => call("chzzk_auto_watch_viewport", { watchId, viewport: { ...viewport, requestSequence: ++transport.sequence } }),
    setAudio: (watchId, enabled) => call("chzzk_auto_watch_audio", { watchId, enabled }),
    close: (watchId) => call("chzzk_auto_watch_close", { watchId }),
  };
}

/** Bind the common live controller to an existing receiver. No playback
 * command may fall back to the unrelated primary live player. */
export function createAutoWatchLiveApi(runtime: AutoWatchApi["runtime"], watchId: string, watch = createAutoWatchApi(runtime)): OfficialBrowserApi {
  const common = createOfficialBrowserApi(runtime);
  const operation = async (value: Record<string, unknown>): Promise<ApiResult<OfficialBrowserSnapshot>> => {
    if (runtime !== "tauri") return { ok: false, error: { code: "DESKTOP_REQUIRED", message: "데스크톱 앱에서 시청할 수 있습니다.", retryable: false } };
    try { return await invoke("chzzk_auto_watch_browser", { watchId, operation: value }); }
    catch { return { ok: false, error: { code: "AUTO_WATCH_TRANSPORT", message: "현재 시청 세션에 연결하지 못했습니다. 자동 녹화 목록에서 상태를 확인해 주세요.", retryable: true } }; }
  };
  const snapshot = async (refreshAuth = false) => {
    if (refreshAuth) { const result = await common.snapshot(true); if (!result.ok) return result; }
    return operation({ kind: "snapshot" });
  };
  const profile = async (action: () => Promise<ApiResult<OfficialBrowserSnapshot>>) => {
    const result = await action(); return result.ok ? snapshot() : result;
  };
  const connectionLocked = async (): Promise<ApiResult<OfficialBrowserSnapshot>> => ({ ok: false, error: {
    code: "LIVE_SESSION_ATTACHED", message: "현재 시청을 닫은 뒤 일반 라이브에서 방송·연결을 변경해 주세요. 기존 녹화는 유지됩니다.", retryable: false,
  } });
  return {
    ...common, runtime, snapshot,
    open: connectionLocked, connectExtension: connectionLocked,
    start: (options) => operation({ kind: "start", ...options }),
    stop: () => operation({ kind: "stop" }),
    requestControl: (action) => operation({ kind: "requestControl", action }),
    confirmControl: (options) => operation({ kind: "confirmControl", ...options }),
    ackUiAction: (id) => operation({ kind: "ackUiAction", id }),
    setViewport: (viewport) => watch.setViewport(watchId, viewport),
    login: () => profile(() => common.login()), logout: () => profile(() => common.logout()),
    setCaptureChat: (enabled) => profile(() => common.setCaptureChat!(enabled)),
    retryMerge: (id) => profile(() => common.retryMerge(id)),
  };
}
