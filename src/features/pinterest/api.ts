import { Channel, invoke } from "@tauri-apps/api/core";
import type { PinterestApi, PinterestDiscovery, PinterestDownload, PinterestDownloadOutcome, PinterestPin, PinterestRemoved } from "./types";

export const pinterestRequestId = () => `pinterest-${globalThis.crypto.randomUUID()}`;

export function createPinterestApi(runtime: "tauri" | "browser-mock"): PinterestApi {
  const unavailable = async (): Promise<never> => { throw new Error("Pinterest 불러오기와 원본 저장은 데스크톱 앱에서 사용할 수 있습니다."); };
  if (runtime !== "tauri") return { removed: unavailable, discover: unavailable, libraryAction: unavailable, get: unavailable, preview: unavailable, download: unavailable, downloadBatch: unavailable, downloads: async () => [], cancel: async () => {}, openExternal: async pinId => { window.open(pinId ? `https://www.pinterest.com/pin/${encodeURIComponent(pinId)}/` : "https://www.pinterest.com/search/pins/", "_blank", "noopener,noreferrer"); } };
  return {
    discover: (input, kind, requestId, cursor) => invoke<PinterestDiscovery>("pinterest_discover", { input, kind, requestId, cursor: cursor ?? null }),
    libraryAction: (action, id, requestId) => invoke<string>("pinterest_library_action", { action, id, requestId }),
    get: (input, requestId) => invoke<PinterestPin>("pinterest_pin_get", { input, requestId }),
    preview: (pinId, page, requestId, thumbnail, removedToken) => invoke<string>("pinterest_preview_get", { pinId, page, requestId, thumbnail, removedToken: removedToken ?? null }),
    download: (pinId, requestId) => invoke<PinterestDownload>("pinterest_download", { pinId, requestId }),
    downloadBatch: (pinIds, requestId, onProgress) => {
      const channel = new Channel<PinterestDownloadOutcome>();
      channel.onmessage = onProgress;
      return invoke<PinterestDownloadOutcome[]>("pinterest_download_batch", { pinIds, requestId, onProgress: channel });
    },
    downloads: () => invoke<PinterestDownload[]>("pinterest_downloads_list"),
    removed: (requestId) => invoke<PinterestRemoved[]>("pinterest_removed_list", { requestId }),
    cancel: (requestId) => invoke<void>("pinterest_cancel", { requestId }),
    openExternal: (pinId) => invoke<void>("pinterest_open_external", { pinId: pinId ?? null }),
  };
}

export const pinterestError = (error: unknown) => error instanceof Error ? error.message : typeof error === "string" ? error : "Pinterest 요청을 완료하지 못했습니다. 다시 시도해 주세요.";

