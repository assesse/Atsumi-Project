import { invoke } from "@tauri-apps/api/core";

export type PinterestAccountStatus = "signedIn" | "signedOut" | "unknown";
export type PinterestAccountSnapshot = {
  status: PinterestAccountStatus;
  busy: boolean;
  open: boolean;
  generation: number;
  error?: string | null;
};
export type PinterestAccountApi = {
  status(): Promise<PinterestAccountSnapshot>;
  open(action: "login" | "logout"): Promise<PinterestAccountSnapshot>;
};

export function createPinterestAccountApi(runtime: "tauri" | "browser-mock"): PinterestAccountApi {
  if (runtime !== "tauri") {
    return {
      status: async () => ({ status: "unknown", busy: false, open: false, generation: 0 }),
      open: async () => { throw new Error("Pinterest 로그인은 데스크톱 앱에서 사용할 수 있습니다."); },
    };
  }
  return {
    status: () => invoke("pinterest_account_status"),
    open: action => invoke("pinterest_account_open", { action }),
  };
}
