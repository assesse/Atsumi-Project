import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

export function useRecordingNotifications(runtime: string, showToast: (message: string) => void) {
  useEffect(() => {
    if (runtime !== "tauri") return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<string>("chzzk-recording:notice", ({ payload }) => {
      if (!disposed && typeof payload === "string" && payload.length <= 500) showToast(payload);
    }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; }).catch(() => {});
    return () => { disposed = true; stop?.(); };
  }, [runtime, showToast]);
}
