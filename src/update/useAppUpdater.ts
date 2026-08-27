import { useCallback, useEffect, useRef, useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";

export type AppUpdateInfo = {
  currentVersion: string;
  version: string;
  date?: string;
  notes?: string;
};

export type AppUpdateCheckResult =
  | { status: "available"; info: AppUpdateInfo }
  | { status: "current" }
  | { status: "unavailable" }
  | { status: "failed"; message: string };

export type AppUpdateState = {
  phase: "idle" | "checking" | "available" | "downloading" | "installing" | "error";
  info: AppUpdateInfo | null;
  downloadedBytes: number;
  totalBytes?: number;
  error?: string;
};

type UpdateRuntime = "tauri" | "browser-mock";
type CheckReason = "startup" | "manual";

const initialState: AppUpdateState = {
  phase: "idle",
  info: null,
  downloadedBytes: 0,
};

const updateInfo = (update: Update): AppUpdateInfo => ({
  currentVersion: update.currentVersion,
  version: update.version,
  ...(update.date ? { date: update.date } : {}),
  ...(update.body?.trim() ? { notes: update.body.trim() } : {}),
});

const CHECK_FAILED_MESSAGE = "업데이트 정보를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.";
const INSTALL_FAILED_MESSAGE = "업데이트를 설치하지 못했습니다. 네트워크 연결을 확인한 뒤 다시 시도해 주세요.";

export function useAppUpdater(runtime: UpdateRuntime) {
  const [state, setState] = useState<AppUpdateState>(initialState);
  const updateRef = useRef<Update | null>(null);
  const checkInFlight = useRef<Promise<AppUpdateCheckResult> | null>(null);
  const startupAttempted = useRef(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const activeUpdate = updateRef.current;
      updateRef.current = null;
      if (activeUpdate) void activeUpdate.close().catch(() => undefined);
    };
  }, []);

  const checkForUpdates = useCallback((_reason: CheckReason = "manual"): Promise<AppUpdateCheckResult> => {
    if (runtime !== "tauri") return Promise.resolve({ status: "unavailable" });
    if (checkInFlight.current) return checkInFlight.current;

    const request = (async (): Promise<AppUpdateCheckResult> => {
      if (mounted.current) {
        setState((current) => ({
          ...current,
          phase: "checking",
          error: undefined,
        }));
      }
      try {
        const availableUpdate = await check({ timeout: 10_000 });
        if (!availableUpdate) {
          if (mounted.current) setState(initialState);
          return { status: "current" };
        }

        const previousUpdate = updateRef.current;
        updateRef.current = availableUpdate;
        if (previousUpdate && previousUpdate !== availableUpdate) {
          void previousUpdate.close().catch(() => undefined);
        }
        const info = updateInfo(availableUpdate);
        if (mounted.current) {
          setState({
            phase: "available",
            info,
            downloadedBytes: 0,
          });
        }
        return { status: "available", info };
      } catch {
        if (mounted.current) setState(initialState);
        return { status: "failed", message: CHECK_FAILED_MESSAGE };
      } finally {
        checkInFlight.current = null;
      }
    })();

    checkInFlight.current = request;
    return request;
  }, [runtime]);

  useEffect(() => {
    if (startupAttempted.current) return;
    startupAttempted.current = true;
    void checkForUpdates("startup");
  }, [checkForUpdates]);

  const dismissUpdate = useCallback(() => {
    if (state.phase === "downloading" || state.phase === "installing") return;
    const activeUpdate = updateRef.current;
    updateRef.current = null;
    if (activeUpdate) void activeUpdate.close().catch(() => undefined);
    setState(initialState);
  }, [state.phase]);

  const installUpdate = useCallback(async () => {
    const activeUpdate = updateRef.current;
    if (!activeUpdate || state.phase === "downloading" || state.phase === "installing") return;

    let downloadedBytes = 0;
    setState((current) => ({
      ...current,
      phase: "downloading",
      downloadedBytes: 0,
      totalBytes: undefined,
      error: undefined,
    }));

    try {
      await activeUpdate.downloadAndInstall((event: DownloadEvent) => {
        if (!mounted.current) return;
        if (event.event === "Started") {
          setState((current) => ({
            ...current,
            phase: "downloading",
            downloadedBytes: 0,
            ...(event.data.contentLength ? { totalBytes: event.data.contentLength } : {}),
          }));
        } else if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          setState((current) => ({ ...current, downloadedBytes }));
        } else if (event.event === "Finished") {
          setState((current) => ({ ...current, phase: "installing" }));
        }
      });
      await relaunch();
    } catch {
      if (!mounted.current) return;
      setState((current) => ({
        ...current,
        phase: "error",
        error: INSTALL_FAILED_MESSAGE,
      }));
    }
  }, [state.phase]);

  return {
    state,
    checkForUpdates,
    dismissUpdate,
    installUpdate,
  };
}
