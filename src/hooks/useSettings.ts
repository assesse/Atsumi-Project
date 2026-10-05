import { useCallback, useEffect, useRef, useState } from "react";
import { backend, type BackendClient, type Unsubscribe } from "../api/backend";
import type { ApiError, ApiResult, SettingsPatch, SettingsSnapshot } from "../api/contracts";

const fallback: SettingsSnapshot = {
  chzzkSsdStaging: false,
  highPerformanceProcessing: false,
  revision: 0,
  downloadRoot: "",
  folderNameTemplate: "[{artist}] {title} [{group}] {id}",
  autoFindHistoryMode: "newer_than_latest_owned",
  downloadOverlapAutoMode: "off",
  explorePageSize: 50,
  danbooruPageSize: 60,
  maxColumns: 3,
  previewWidth: 220,
  danbooruPreviewWidth: 190,
  relatedPreviewWidth: 240,
  privacyMode: false,
  privacyOnStartup: true,
  cacheLimitGb: 10,
  concurrentImageRequests: 5,
  downloadAdaptiveConcurrency: true,
  downloadAdaptiveMaxRequests: 8,
  requestStartIntervalMs: 25,
  autoFindGrouping: "all",
  downloadsGrouping: "all",
  exploreDisplayMode: "detail",
  autoFindDisplayMode: "detail",
  downloadsDisplayMode: "detail",
  collapsedGroupKeys: [],
  searchIncludeTags: [],
  searchExcludeTags: [],
};

const runtimeError = (operation: string): ApiError => ({
  code: "BACKEND_UNAVAILABLE",
  message: `${operation} 중 backend에 연결하지 못했습니다.`,
  retryable: true,
  action: "retry",
});

export type SettingsApi = Pick<BackendClient, "settingsGet" | "settingsUpdate"> & {
  on(event: "settings:changed", handler: (snapshot: SettingsSnapshot) => void): Promise<Unsubscribe>;
};

export function useSettings(api: SettingsApi = backend) {
  const [settings, setSettings] = useState<SettingsSnapshot>(fallback);
  const [loading, setLoading] = useState(true);
  const [hasSnapshot, setHasSnapshot] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);
  const latest = useRef(settings);
  const writes = useRef<Promise<unknown>>(Promise.resolve());
  const accept = useCallback((snapshot: SettingsSnapshot) => {
    if (snapshot.revision >= latest.current.revision) {
      latest.current = snapshot;
      setSettings(snapshot);
    }
    setHasSnapshot(true);
  }, []);

  useEffect(() => {
    setLoading(true);
    setHasSnapshot(false);
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;
    void (async () => {
      let subscriptionError: ApiError | null = null;
      try {
        const cleanup = await api.on("settings:changed", (snapshot) => {
          if (cancelled) return;
          accept(snapshot);
          setError(null);
        });
        if (cancelled) {
          cleanup();
          return;
        }
        unsubscribe = cleanup;
      } catch {
        subscriptionError = runtimeError("설정 변경을 구독하는");
      }

      try {
        const result = await api.settingsGet();
        if (cancelled) return;
        if (result.ok) {
          accept(result.data);
          setError(subscriptionError);
        } else {
          setError(result.error);
        }
      } catch {
        if (!cancelled) setError(runtimeError("설정을 불러오는"));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  }, [api, accept]);

  const save = useCallback(
    (patch: SettingsPatch) => {
      const operation = writes.current.then(async () => {
        let result: ApiResult<SettingsSnapshot>;
        try {
          result = await api.settingsUpdate(patch, latest.current.revision);
          // Other controls may save concurrently. Retry only our partial edit once.
          if (!result.ok && result.error.code === "REVISION_CONFLICT") {
            const refreshed = await api.settingsGet();
            if (refreshed.ok) {
              accept(refreshed.data);
              result = await api.settingsUpdate(patch, latest.current.revision);
            }
          }
        } catch {
          const error = runtimeError("설정을 저장하는");
          setError(error);
          return { ok: false, error } as const;
        }
        if (result.ok) {
          accept(result.data);
          setError(null);
        } else {
          setError(result.error);
        }
        return result;
      });
      writes.current = operation.catch(() => undefined);
      return operation;
    },
    [api, accept],
  );

  return { settings, loading, hasSnapshot, error, save };
}
