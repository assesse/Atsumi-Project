import { invoke } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";
export type ScreenshotItem = { id: string; channelId: string; fileName: string; createdAt: number; bytes: number; width: number; height: number };
export type ScreenshotDeleteReport = { deleted: string[]; failures: { id: string; message: string }[] };
export function createScreenshotsApi(runtime: "tauri" | "browser-mock") {
  async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    let result: ApiResult<T>;
    try { result = await invoke<ApiResult<T>>(command, args); }
    catch { throw new Error("스크린샷에 연결하지 못했습니다. 다시 시도해 주세요."); }
    if (!result.ok) throw new Error(result.error.message);
    return result.data;
  }
  return {
    list: () => runtime === "tauri" ? call<ScreenshotItem[]>("chzzk_screenshots_list") : Promise.resolve([]),
    image: (id: string, full = false) => call<string>("chzzk_screenshot_image", { id, full }),
    delete: (ids: string[]) => call<ScreenshotDeleteReport>("chzzk_screenshots_delete", { ids }),
    folder: (id?: string) => call<void>("chzzk_screenshots_folder", { id: id ?? null }),
  };
}
export type ScreenshotsApi = ReturnType<typeof createScreenshotsApi>;
