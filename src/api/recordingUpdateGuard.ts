import { invoke } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";
import type { UpdateInstallGuard } from "../update/useAppUpdater";

/** The backend reservation closes the start-vs-install race, not only a UI check. */
export function createRecordingUpdateGuard(runtime: "tauri" | "browser-mock"): UpdateInstallGuard {
  return {
    async acquire() {
      if (runtime !== "tauri") return;
      let result: ApiResult<void>;
      try { result = await invoke<ApiResult<void>>("streaming_update_reserve"); }
      catch { throw new Error("녹화 상태를 확인하지 못해 업데이트 설치를 보류했습니다."); }
      if (!result.ok) throw new Error(result.error.message);
    },
    async release() {
      if (runtime === "tauri") await invoke("streaming_update_release");
    },
  };
}
