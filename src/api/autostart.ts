import { invoke, isTauri } from "@tauri-apps/api/core";
import type { ApiResult } from "./contracts";

/** OS registration is authoritative; this is not a preference stored in the DB. */
export type AutostartStatus = {
  supported: boolean;
  /** An Atsumi-owned startup entry exists; Windows can separately disable it. */
  enabled: boolean;
  launchMode: "development" | "installed" | "unsupported";
  needsRepair: boolean;
  disabledByWindows: boolean;
};

const unsupported: AutostartStatus = {
  supported: false,
  enabled: false,
  launchMode: "unsupported",
  needsRepair: false,
  disabledByWindows: false,
};

async function request(
  command: "autostart_status_get" | "autostart_enabled_set",
  args?: { enabled: boolean },
): Promise<ApiResult<AutostartStatus>> {
  try {
    return { ok: true, data: await invoke<AutostartStatus>(command, args) };
  } catch (error) {
    return {
      ok: false,
      error: {
        code: command === "autostart_status_get" ? "AUTOSTART_READ_FAILED" : "AUTOSTART_UPDATE_FAILED",
        message: typeof error === "string" && error.trim()
          ? error
          : "Windows 자동 실행 설정을 처리하지 못했습니다. 잠시 후 다시 시도해 주세요.",
        retryable: true,
      },
    };
  }
}

export async function getAutostartStatus(): Promise<ApiResult<AutostartStatus>> {
  if (!isTauri()) return { ok: true, data: { ...unsupported } };
  return request("autostart_status_get");
}

export async function setAutostartEnabled(enabled: boolean): Promise<ApiResult<AutostartStatus>> {
  if (!isTauri()) {
    return {
      ok: false,
      error: {
        code: "AUTOSTART_UNSUPPORTED",
        message: "Windows용 Atsumi 앱에서 자동 실행을 설정할 수 있습니다.",
        retryable: false,
      },
    };
  }
  return request("autostart_enabled_set", { enabled });
}
