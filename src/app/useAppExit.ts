import { useCallback, useEffect, useRef, useState, type ComponentProps } from "react";
import type { BackendClient, Unsubscribe } from "../api/backend";
import { hasActiveWork, type AppActiveWorkSnapshot, type AppExitRequestedEvent } from "../api/contracts";
import type { ExitConfirmDialog } from "../components/ExitConfirmDialog";

export type AppExitApi = Pick<BackendClient, "appActiveWorkSnapshot" | "appMinimizeToTray" | "appQuit"> & {
  on(event: "app:exit-requested", handler: (payload: AppExitRequestedEvent) => void): Promise<Unsubscribe>;
};

export function useAppExit(api: AppExitApi, showToast: (message: string) => void) {
  const [open, setOpen] = useState(false);
  const [snapshot, setSnapshot] = useState<AppActiveWorkSnapshot | null>(null);
  const [statusError, setStatusError] = useState(false);
  const [forceQuitArmed, setForceQuitArmed] = useState(false);
  const [actionPending, setActionPending] = useState(false);
  const mounted = useRef(false);
  const openRef = useRef(false);
  const actionPendingRef = useRef(false);
  const snapshotSequence = useRef(0);
  const actionSequence = useRef(0);
  const showToastRef = useRef(showToast);

  useEffect(() => {
    showToastRef.current = showToast;
  }, [showToast]);

  useEffect(() => {
    mounted.current = true;
    setOpen(false);
    setActionPending(false);
    return () => {
      mounted.current = false;
      openRef.current = false;
      actionPendingRef.current = false;
      snapshotSequence.current += 1;
      actionSequence.current += 1;
    };
  }, [api]);

  const refreshSnapshot = useCallback(async (armForceOnFailure = false): Promise<AppActiveWorkSnapshot | null> => {
    const sequence = ++snapshotSequence.current;
    const isCurrent = () => mounted.current && openRef.current && sequence === snapshotSequence.current;
    try {
      const result = await api.appActiveWorkSnapshot();
      if (!isCurrent()) return null;
      if (result.ok) {
        setSnapshot(result.data);
        setStatusError(false);
        setForceQuitArmed(false);
        return result.data;
      }
    } catch {
      if (!isCurrent()) return null;
    }
    setSnapshot(null);
    setStatusError(true);
    setForceQuitArmed(armForceOnFailure);
    return null;
  }, [api]);

  const openExitConfirm = useCallback(() => {
    if (!mounted.current || openRef.current || actionPendingRef.current) return;
    openRef.current = true;
    setSnapshot(null);
    setStatusError(false);
    setForceQuitArmed(false);
    actionPendingRef.current = false;
    setActionPending(false);
    setOpen(true);
    void refreshSnapshot();
  }, [refreshSnapshot]);

  const closeExitConfirm = useCallback(() => {
    if (!mounted.current || actionPendingRef.current) return;
    snapshotSequence.current += 1;
    openRef.current = false;
    setOpen(false);
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: Unsubscribe | undefined;
    void (async () => {
      try {
        const cleanup = await api.on("app:exit-requested", () => {
          if (!disposed) openExitConfirm();
        });
        if (disposed) cleanup();
        else unlisten = cleanup;
      } catch {
        if (!disposed) showToastRef.current("창 닫기 동작을 연결하지 못했습니다.");
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [api, openExitConfirm]);

  const setStatusUnavailable = useCallback(() => {
    snapshotSequence.current += 1;
    setSnapshot(null);
    setStatusError(true);
    setForceQuitArmed(false);
  }, []);

  const finishAction = useCallback(() => {
    actionPendingRef.current = false;
    setActionPending(false);
  }, []);

  const onMinimizeToTray = useCallback(() => {
    if (!mounted.current || !openRef.current || actionPendingRef.current) return;
    actionPendingRef.current = true;
    setActionPending(true);
    const sequence = ++actionSequence.current;
    const isCurrent = () => mounted.current && openRef.current && sequence === actionSequence.current;
    void (async () => {
      try {
        const result = await api.appMinimizeToTray();
        if (!isCurrent()) return;
        if (!result.ok) {
          if (result.error.code === "APP_ACTIVE_WORK_STATUS_UNAVAILABLE") setStatusUnavailable();
          finishAction();
          showToastRef.current(result.error.message);
        } else {
          finishAction();
          snapshotSequence.current += 1;
          openRef.current = false;
          setOpen(false);
        }
      } catch {
        if (!isCurrent()) return;
        finishAction();
        showToastRef.current("트레이로 최소화하지 못했습니다.");
      }
    })();
  }, [api, finishAction, setStatusUnavailable]);

  const onQuit = useCallback(() => {
    if (!mounted.current || !openRef.current || actionPendingRef.current) return;
    if (snapshot === null && !statusError) return;
    actionPendingRef.current = true;
    setActionPending(true);
    const sequence = ++actionSequence.current;
    const isCurrent = () => mounted.current && openRef.current && sequence === actionSequence.current;
    void (async () => {
      try {
        if (snapshot === null && !forceQuitArmed) {
          const refreshed = await refreshSnapshot(true);
          if (!isCurrent()) return;
          finishAction();
          if (!refreshed) showToastRef.current("작업 상태를 다시 확인하지 못했습니다. 트레이로 보내거나 상태 확인 없이 종료할 수 있습니다.");
          return;
        }

        const result = await api.appQuit(snapshot
          ? {
            expectedWorkSetFingerprint: snapshot.workSetFingerprint,
            confirmActiveWork: hasActiveWork(snapshot),
          }
          : {
            expectedWorkSetFingerprint: "",
            confirmActiveWork: true,
            forceWhenStatusUnknown: true,
          });
        if (!isCurrent()) return;
        if (!result.ok) {
          if (result.error.code === "APP_ACTIVE_WORK_STATUS_UNAVAILABLE") setStatusUnavailable();
          finishAction();
          showToastRef.current(result.error.message);
          return;
        }
        if (!result.data.accepted) {
          if (result.data.snapshot) {
            setSnapshot(result.data.snapshot);
            setStatusError(false);
            setForceQuitArmed(false);
          }
          finishAction();
          showToastRef.current(result.data.reason === "active_work_changed"
            ? "진행 작업이 변경되었습니다. 내용을 확인하고 다시 선택해 주세요."
            : "진행 중인 작업을 확인한 뒤 종료를 다시 선택해 주세요.");
        }
      } catch {
        if (!isCurrent()) return;
        finishAction();
        showToastRef.current("프로그램을 종료하지 못했습니다.");
      }
    })();
  }, [api, finishAction, forceQuitArmed, refreshSnapshot, setStatusUnavailable, snapshot, statusError]);

  const dialogProps: ComponentProps<typeof ExitConfirmDialog> = {
    open,
    snapshot,
    statusError,
    actionPending,
    forceQuitArmed,
    onClose: closeExitConfirm,
    onMinimizeToTray,
    onQuit,
  };

  return { open, openExitConfirm, closeExitConfirm, dialogProps };
}
