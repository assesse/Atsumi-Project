import { useCallback, useEffect, useState } from "react";
import { backend } from "../api/backend";
import type { ApiError, ApiResult, SettingsPatch, SettingsSnapshot } from "../api/contracts";

const fallback: SettingsSnapshot = {
  revision: 0,
  downloadRoot: "",
  folderNameTemplate: "[{artist}] {title} [{group}] {id}",
  autoFindHistoryMode: "include_all_history",
  downloadOverlapAutoMode: "off",
  explorePageSize: 50,
  maxColumns: 3,
  previewWidth: 220,
  relatedPreviewWidth: 240,
  privacyMode: false,
  cacheLimitGb: 10,
  concurrentImageRequests: 5,
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

export function useSettings() {
  const [settings, setSettings] = useState<SettingsSnapshot>(fallback);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<ApiError | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;
    void (async () => {
      let subscriptionError: ApiError | null = null;
      try {
        const cleanup = await backend.on("settings:changed", (snapshot) => {
          if (cancelled) return;
          setSettings((current) => snapshot.revision > current.revision ? snapshot : current);
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
        const result = await backend.settingsGet();
        if (cancelled) return;
        if (result.ok) {
          setSettings((current) => result.data.revision >= current.revision ? result.data : current);
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
  }, []);

  const save = useCallback(
    async (patch: SettingsPatch) => {
      let result: ApiResult<SettingsSnapshot>;
      try {
        result = await backend.settingsUpdate(patch, settings.revision);
      } catch {
        const error = runtimeError("설정을 저장하는");
        setError(error);
        return { ok: false, error } as const;
      }
      if (result.ok) {
        setSettings(result.data);
        setError(null);
      } else {
        setError(result.error);
        if (result.error.code === "REVISION_CONFLICT") {
          try {
            const refreshed = await backend.settingsGet();
            if (refreshed.ok) {
              setSettings((current) => refreshed.data.revision >= current.revision ? refreshed.data : current);
            }
          } catch {
            setError(runtimeError("최신 설정을 다시 불러오는"));
          }
        }
      }
      return result;
    },
    [settings.revision],
  );

  return { settings, loading, error, save };
}
