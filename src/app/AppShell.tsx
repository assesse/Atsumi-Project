import { createContext, useCallback, useContext, useEffect, useMemo, useReducer, useRef, useState, type ReactNode } from "react";
import type { BackendClient } from "../api/backend";
import type { SettingsPatch } from "../api/contracts";
import { ExitConfirmDialog } from "../components/ExitConfirmDialog";
import { TutorialDialog } from "../components/TutorialDialog";
import { UpdateDialog } from "../components/UpdateDialog";
import { useSettings, type SettingsApi } from "../hooks/useSettings";
import { useWindowPlacement } from "../hooks/useWindowPlacement";
import { loadContentSource, saveContentSource } from "../state/sourcePreference";
import { isTutorialDismissed, setTutorialDismissed } from "../tutorial/tutorialPreference";
import { useAppUpdater } from "../update/useAppUpdater";
import { createShellState, shellReducer, type ShellState } from "./shellState";
import { useAppExit, type AppExitApi } from "./useAppExit";
import { usePreferenceQueue } from "./usePreferenceQueue";
import type { ContentSource } from "./workspaceRegistry";

export type AppShellApi = SettingsApi & AppExitApi & Pick<BackendClient, "runtime">;

type AppShellServices = ShellState & {
  selectSource: (source: ContentSource) => void;
  toggleRail: () => void;
  setSettingsOpen: (open: boolean) => void;
  setActivityOpen: (open: boolean) => void;
  showToast: (message: string) => void;
  settingsStore: ReturnType<typeof useSettings>;
  preferenceQueue: ReturnType<typeof usePreferenceQueue>;
  saveSettingsPatch: (patch: SettingsPatch) => Promise<boolean>;
  privacyModePending: boolean;
  togglePrivacyMode: () => Promise<void>;
  checkForUpdates: ReturnType<typeof useAppUpdater>["checkForUpdates"];
  exitConfirmOpen: boolean;
  openExitConfirm: () => void;
};

const AppShellContext = createContext<AppShellServices | null>(null);

export function useAppShell(): AppShellServices {
  const shell = useContext(AppShellContext);
  if (!shell) throw new Error("An Atsumi feature must be mounted inside AppShell");
  return shell;
}

/** One owner for app-wide services. Source switches never remount this provider. */
export function AppShell({ api, children }: { api: AppShellApi; children: ReactNode }) {
  const [state, dispatch] = useReducer(shellReducer, undefined, () => createShellState(loadContentSource()));
  const settingsStore = useSettings(api);
  const { settings, save: saveSettings } = settingsStore;
  const updater = useAppUpdater(api.runtime);
  const [tutorialOpen, setTutorialOpen] = useState(() => !isTutorialDismissed());
  const [toast, setToast] = useState<{ id: number; message: string } | null>(null);
  const toastTimer = useRef<number | undefined>(undefined);
  const toastSequence = useRef(0);
  const [privacyModePending, setPrivacyModePending] = useState(false);
  const privacyMutationPending = useRef(false);
  useWindowPlacement();

  const showToast = useCallback((message: string) => {
    window.clearTimeout(toastTimer.current);
    setToast({ id: ++toastSequence.current, message });
    toastTimer.current = window.setTimeout(() => setToast(null), 2400);
  }, []);
  useEffect(() => () => window.clearTimeout(toastTimer.current), []);

  const exit = useAppExit(api, showToast);
  const preferenceQueue = usePreferenceQueue(api, showToast);
  const selectSource = useCallback((source: ContentSource) => {
    saveContentSource(source);
    dispatch({ type: "source.select", source });
  }, []);
  const toggleRail = useCallback(() => dispatch({ type: "rail.toggle" }), []);
  const setSettingsOpen = useCallback((open: boolean) => dispatch({ type: "settings.set", open }), []);
  const setActivityOpen = useCallback((open: boolean) => dispatch({ type: "activity.set", open }), []);

  useEffect(() => {
    document.documentElement.dataset.privacyMode = settings.privacyMode ? "on" : "off";
    return () => { delete document.documentElement.dataset.privacyMode; };
  }, [settings.privacyMode]);

  const saveSettingsPatch = useCallback(async (patch: SettingsPatch) => {
    const result = await saveSettings(patch);
    showToast(result.ok ? "설정을 저장했습니다." : result.error.message);
    return result.ok;
  }, [saveSettings, showToast]);

  const togglePrivacyMode = useCallback(async () => {
    if (privacyMutationPending.current) return;
    privacyMutationPending.current = true;
    setPrivacyModePending(true);
    try {
      const result = await saveSettings({ privacyMode: !settings.privacyMode });
      showToast(result.ok ? result.data.privacyMode ? "프라이버시 모드 켬" : "프라이버시 모드 끔" : result.error.message);
    } finally {
      privacyMutationPending.current = false;
      setPrivacyModePending(false);
    }
  }, [saveSettings, settings.privacyMode, showToast]);

  const closeTutorial = useCallback((doNotShowAgain: boolean) => {
    if (doNotShowAgain) setTutorialDismissed(true);
    setTutorialOpen(false);
  }, []);

  const value = useMemo<AppShellServices>(() => ({
    ...state, selectSource, toggleRail, setSettingsOpen, setActivityOpen, showToast,
    settingsStore, preferenceQueue, saveSettingsPatch, privacyModePending, togglePrivacyMode,
    checkForUpdates: updater.checkForUpdates,
    exitConfirmOpen: exit.open, openExitConfirm: exit.openExitConfirm,
  }), [state, selectSource, toggleRail, setSettingsOpen, setActivityOpen, showToast, settingsStore, preferenceQueue,
    saveSettingsPatch, privacyModePending, togglePrivacyMode, updater.checkForUpdates, exit.open, exit.openExitConfirm]);

  return (
    <AppShellContext.Provider value={value}>
      {children}
      <UpdateDialog
        open={!tutorialOpen && updater.state.info !== null && ["available", "downloading", "installing", "error"].includes(updater.state.phase)}
        state={updater.state}
        onLater={updater.dismissUpdate}
        onInstall={() => void updater.installUpdate()}
      />
      <TutorialDialog open={tutorialOpen} onClose={closeTutorial} />
      <ExitConfirmDialog {...exit.dialogProps} />
      {toast ? <div key={toast.id} className="toast" role="status">{toast.message}</div> : null}
    </AppShellContext.Provider>
  );
}
