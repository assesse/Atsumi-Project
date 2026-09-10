import { useCallback, useEffect, useMemo, useRef } from "react";
import type { BackendClient } from "../api/backend";
import type { SettingsPatch } from "../api/contracts";

export type PreferenceQueueApi = Pick<BackendClient, "settingsGet" | "settingsUpdate">;

type QueueSession = {
  api: PreferenceQueueApi;
  disposed: boolean;
  active: boolean;
  queued: SettingsPatch | null;
};

export function usePreferenceQueue(api: PreferenceQueueApi, showToast: (message: string) => void) {
  const sessionRef = useRef<QueueSession | null>(null);
  const showToastRef = useRef(showToast);

  useEffect(() => {
    showToastRef.current = showToast;
  }, [showToast]);

  useEffect(() => {
    const session: QueueSession = { api, disposed: false, active: false, queued: null };
    sessionRef.current = session;
    return () => {
      session.disposed = true;
      session.active = false;
      session.queued = null;
    };
  }, [api]);

  const enqueue = useCallback((patch: SettingsPatch) => {
    const session = sessionRef.current;
    if (!session || session.disposed) return;
    session.queued = { ...(session.queued ?? {}), ...patch };
    if (session.active) return;
    session.active = true;

    void (async () => {
      try {
        while (!session.disposed && session.queued !== null) {
          const desired = session.queued;
          session.queued = null;
          try {
            const current = await session.api.settingsGet();
            if (session.disposed) return;
            if (!current.ok) {
              showToastRef.current(`목록 표시 설정을 저장하지 못했습니다. ${current.error.message}`);
              continue;
            }
            const saved = await session.api.settingsUpdate(desired, current.data.revision);
            if (session.disposed) return;
            if (!saved.ok) showToastRef.current(`목록 표시 설정을 저장하지 못했습니다. ${saved.error.message}`);
          } catch {
            if (session.disposed) return;
            showToastRef.current("목록 표시 설정을 저장하지 못했습니다.");
          }
        }
      } finally {
        session.active = false;
      }
    })();
  }, []);

  const isPending = useCallback(() => {
    const session = sessionRef.current;
    return session !== null && !session.disposed && (session.active || session.queued !== null);
  }, []);

  return useMemo(() => ({ enqueue, isPending }), [enqueue, isPending]);
}
