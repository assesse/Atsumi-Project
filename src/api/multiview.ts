import { invoke } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";
import type { BrowserControlAction, OfficialBrowserViewport } from "./officialBrowser";

export type MultiviewEntry = { channelId: string; video: boolean; chat: boolean };
export type MultiviewPane = { paneId: string; channelId: string; kind: "video" | "chat"; status: string;
  channelName?: string; audioEnabled?: boolean; ready?: boolean; recordingId?: string | null; recordingStatus?: string | null; error?: string | null;
  chatStatus?: string; chatCount?: number;
  lastScreenshot?: { id: string; channelId: string; fileName: string; createdAt: number } | null;
};
export type MultiviewSnapshot = { active: boolean; epoch: number; audioOwner: string | null; panes: MultiviewPane[];
  pendingControl?: { paneId: string; id: string; action: BrowserControlAction; channelId: string; expiresAt: number } | null;
  pendingUiAction?: { paneId: string; id: string; action: "exit_focus" } | null;
};
export const emptyMultiview = (): MultiviewSnapshot => ({ active: false, epoch: 0, audioOwner: null, panes: [] });
export interface MultiviewApi {
  readonly runtime: "tauri" | "browser-mock";
  configure(entries: MultiviewEntry[]): Promise<ApiResult<MultiviewSnapshot>>;
  snapshot(): Promise<ApiResult<MultiviewSnapshot>>;
  close(epoch: number): Promise<ApiResult<MultiviewSnapshot>>;
  setAudio(channelId: string | null, epoch: number): Promise<ApiResult<MultiviewSnapshot>>;
  setPaneAudio(paneId: string, enabled: boolean, epoch: number): Promise<ApiResult<MultiviewSnapshot>>;
  requestControl(paneId: string, action: BrowserControlAction, epoch: number): Promise<ApiResult<MultiviewSnapshot>>;
  confirmControl(options: { paneId: string; requestId: string; approve: boolean; rightsAcknowledged: boolean; captureChat: boolean; epoch: number }): Promise<ApiResult<MultiviewSnapshot>>;
  ackUiAction(paneId: string, id: string, epoch: number): Promise<ApiResult<MultiviewSnapshot>>;
  setViewport(paneId: string, viewport: OfficialBrowserViewport): Promise<ApiResult<void>>;
}
// A main-document sequence survives hot reload. Pane labels contain the native
// generation, so no sequence is reused for a different pane after reconfigure.
const transport = (import.meta.hot?.data?.multiviewTransport as { sequence: number } | undefined) ?? { sequence: 0 };
if (import.meta.hot?.data) import.meta.hot.data.multiviewTransport = transport;
export function createMultiviewApi(runtime: MultiviewApi["runtime"]): MultiviewApi {
  const call = <T>(command: string, args?: Record<string, unknown>): Promise<ApiResult<T>> => runtime === "tauri"
    ? invoke(command, args)
    : Promise.resolve({ ok: false, error: { code: "DESKTOP_REQUIRED", message: "데스크톱 앱에서 시청할 수 있습니다.", retryable: false } });
  return {
    runtime,
    configure: (entries) => call("chzzk_multiview_configure", { entries }),
    snapshot: () => call("chzzk_multiview_snapshot"),
    close: (epoch) => call("chzzk_multiview_close", { epoch }),
    setAudio: (channelId, epoch) => call("chzzk_multiview_set_audio", { channelId, epoch }),
    setPaneAudio: (paneId, enabled, epoch) => call("chzzk_multiview_set_pane_audio", { paneId, enabled, epoch }),
    requestControl: (paneId, action, epoch) => call("chzzk_multiview_request_control", { paneId, action, epoch }),
    confirmControl: (options) => call("chzzk_multiview_confirm_control", options),
    ackUiAction: (paneId, id, epoch) => call("chzzk_multiview_ack_ui_action", { paneId, id, epoch }),
    setViewport: (paneId, viewport) => call("chzzk_multiview_set_viewport", { paneId, viewport: { ...viewport, requestSequence: ++transport.sequence } }),
  };
}

export function normalizeMultiviewChannel(input: string): string | null {
  const text = input.trim();
  if (/^[a-f\d]{32}$/i.test(text)) return text.toLowerCase();
  try {
    const url = new URL(text);
    if (url.origin !== "https://chzzk.naver.com" || url.username || url.password || url.search || url.hash) return null;
    return /^\/live\/([a-f\d]{32})(?:\/chat)?\/?$/i.exec(url.pathname)?.[1]?.toLowerCase() ?? null;
  } catch { return null; }
}

export function multiviewEntries(inputs: string[], mode: "paired" | "chats", lead: number): MultiviewEntry[] | string {
  const filled = inputs.map((input, index) => ({ input: input.trim(), index })).filter((row) => row.input);
  if (!filled.length || filled.length > 4) return "방송 주소를 1~4개 입력해 주세요.";
  const result: MultiviewEntry[] = [];
  for (const row of filled) {
    const channelId = normalizeMultiviewChannel(row.input);
    if (!channelId) return `${row.index + 1}번 방송 주소를 확인해 주세요.`;
    if (result.some((entry) => entry.channelId === channelId)) return "같은 방송은 한 번만 추가해 주세요.";
    result.push({ channelId, video: mode === "paired" || row.index === lead, chat: true });
  }
  if (!result.some((entry) => entry.video)) return "영상으로 볼 방송을 선택해 주세요.";
  return result;
}
