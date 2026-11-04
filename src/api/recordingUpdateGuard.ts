import { invoke } from "@tauri-apps/api/core";
import type { UpdateInstallGuard } from "../update/useAppUpdater";

/** Works without AppState; the native reservation still excludes all active work. */
export function createRecordingUpdateGuard(runtime: "tauri" | "browser-mock"): UpdateInstallGuard {
  return {
    async acquire() {
      if (runtime !== "tauri") return;
      try { await invoke("app_update_reserve"); }
      catch (error) {
        throw new Error(typeof error === "string" ? error
          : error instanceof Error ? error.message : "작업 상태를 확인하지 못해 업데이트 설치를 보류했습니다.");
      }
    },
    async release() {
      if (runtime === "tauri") await invoke("app_update_release");
    },
  };
}
