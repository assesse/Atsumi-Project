import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type ReactNode } from "react";
import type { BackendClient } from "../api/backend";
import type { SettingsPatch } from "../api/contracts";
import { ExitConfirmDialog } from "../components/ExitConfirmDialog";
import { UpdateDialog } from "../components/UpdateDialog";
import { useSettings, type SettingsApi } from "../hooks/useSettings";
import { useWindowPlacement } from "../hooks/useWindowPlacement";
import { loadContentSource, saveContentSource } from "../state/sourcePreference";
import { isTutorialDismissed, setTutorialDismissed } from "../tutorial/tutorialPreference";
import { useAppUpdater, type UpdateInstallGuard } from "../update/useAppUpdater";
import { createShellState, shellReducer, type ShellState } from "./shellState";
import { useAppExit, type AppExitApi } from "./useAppExit";
import { usePreferenceQueue } from "./usePreferenceQueue";
import type { ContentSource } from "./workspaceRegistry";
import { useRecordingNotifications } from "./useRecordingNotifications";
import { useStartup } from "./useStartup";

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
  privacyMode: boolean;
  privacyModePending: boolean;
  togglePrivacyMode: () => Promise<void>;
  checkForUpdates: ReturnType<typeof useAppUpdater>["checkForUpdates"];
  exitConfirmOpen: boolean;
  openExitConfirm: () => void;
  backgroundReady: boolean;
  tutorialOpen: boolean;
  tutorialSource: ContentSource | null;
  replayTutorial: () => void;
  closeTutorial: () => void;
};

const AppShellContext = createContext<AppShellServices | null>(null);

export function useAppShell(): AppShellServices {
  const shell = useContext(AppShellContext);
  if (!shell) throw new Error("An Atsumi feature must be mounted inside AppShell");
  return shell;
}

/** One owner for app-wide services. Source switches never remount this provider. */
export function AppShell({ api, children, updateGuard }: { api: AppShellApi; children: ReactNode; updateGuard?: UpdateInstallGuard }) {
  const [state, dispatch] = useReducer(shellReducer, undefined, () => createShellState(loadContentSource()));
  const settingsStore = useSettings(api);
  const { settings, save: saveSettings } = settingsStore;
  const startup = useStartup(api.runtime, settingsStore.hasSnapshot);
  const updater = useAppUpdater(api.runtime, updateGuard);
  const [tutorialSource, setTutorialSource] = useState<ContentSource | null>(() => isTutorialDismissed(state.source) ? null : state.source);
  const tutorialOpen = tutorialSource !== null;
  const shownTutorials = useRef(new Set<ContentSource>());
  // Remember first display, not completion: skipping or closing the app must not replay it.
  useEffect(() => {
    if (!tutorialSource) return;
    shownTutorials.current.add(tutorialSource);
    setTutorialDismissed(true, tutorialSource);
  }, [tutorialSource]);
  const [toast, setToast] = useState<{ id: number; message: string } | null>(null);
  const toastTimer = useRef<number | undefined>(undefined);
  const toastSequence = useRef(0);
  const [privacyMode, setPrivacyMode] = useState(true);
  const privacyInitialized = useRef(false);
  const privacyModePending = !settingsStore.hasSnapshot;
  useLayoutEffect(() => {
    if (settingsStore.hasSnapshot && !privacyInitialized.current) {
      privacyInitialized.current = true;
      setPrivacyMode(settings.privacyOnStartup ?? true);
    }
  }, [settingsStore.hasSnapshot, settings.privacyOnStartup]);
  useWindowPlacement();

  const showToast = useCallback((message: string) => {
    window.clearTimeout(toastTimer.current);
    setToast({ id: ++toastSequence.current, message });
    toastTimer.current = window.setTimeout(() => setToast(null), 2400);
  }, []);
  useEffect(() => () => window.clearTimeout(toastTimer.current), []);
  useRecordingNotifications(api.runtime, showToast);

  const exit = useAppExit(api, showToast);
  // A native close request takes priority; the tour must never trap the exit dialog.
  useEffect(() => { if (exit.open) setTutorialSource(null); }, [exit.open]);
  const preferenceQueue = usePreferenceQueue(api, showToast);
  const selectSource = useCallback((source: ContentSource) => {
    saveContentSource(source);
    dispatch({ type: "source.select", source });
    setTutorialSource(current => {
      if (exit.open) return null;
      if (current === source) return current;
      return shownTutorials.current.has(source) || isTutorialDismissed(source) ? null : source;
    });
  }, [exit.open]);
  const toggleRail = useCallback(() => dispatch({ type: "rail.toggle" }), []);
  const setSettingsOpen = useCallback((open: boolean) => dispatch({ type: "settings.set", open }), []);
  const setActivityOpen = useCallback((open: boolean) => dispatch({ type: "activity.set", open }), []);

  useLayoutEffect(() => {
    document.documentElement.dataset.privacyMode = privacyMode && state.source !== "chzzk" ? "on" : "off";
  }, [privacyMode, state.source]);

  const saveSettingsPatch = useCallback(async (patch: SettingsPatch) => {
    const result = await saveSettings(patch);
    showToast(result.ok ? "설정을 저장했습니다." : result.error.message);
    return result.ok;
  }, [saveSettings, showToast]);

  const togglePrivacyMode = useCallback(async () => {
    if (privacyModePending) return;
    setPrivacyMode((current) => !current);
  }, [privacyModePending]);

  const replayTutorial = useCallback(() => {
    // This opens only the current session; never clear the first-display marker.
    setSettingsOpen(false);
    if (!exit.open) setTutorialSource(state.source);
  }, [setSettingsOpen, state.source, exit.open]);
  const closeTutorial = useCallback(() => {
    setTutorialSource(null);
  }, []);

  const value = useMemo<AppShellServices>(() => ({
    ...state, selectSource, toggleRail, setSettingsOpen, setActivityOpen, showToast,
    settingsStore, preferenceQueue, saveSettingsPatch, privacyMode, privacyModePending, togglePrivacyMode,
    checkForUpdates: updater.checkForUpdates,
    exitConfirmOpen: exit.open, openExitConfirm: exit.openExitConfirm,
    backgroundReady: startup.backgroundReady,
    tutorialOpen, tutorialSource, replayTutorial, closeTutorial,
  }), [state, selectSource, toggleRail, setSettingsOpen, setActivityOpen, showToast, settingsStore, preferenceQueue,
    saveSettingsPatch, privacyMode, privacyModePending, togglePrivacyMode, updater.checkForUpdates, exit.open, exit.openExitConfirm, startup.backgroundReady, tutorialOpen, tutorialSource, replayTutorial, closeTutorial]);

  return (
    <AppShellContext.Provider value={value}>
      {children}
      {startup.phase !== "ready" ? <aside className={`startup-status is-${startup.phase === "failed" ? "error" : startup.statusError ? "warning" : "loading"}`} role={startup.phase === "failed" || startup.statusError ? "alert" : "status"}>
        <i className="startup-status-icon" aria-hidden="true">{startup.phase === "failed" ? "×" : startup.statusError ? "!" : ""}</i>
        <span>{startup.phase === "failed" ? "앱 데이터 준비 실패 · 기존 데이터는 보존됩니다."
          : startup.statusError ? startup.statusError
          : startup.phase === "cancelling" ? "시작을 취소하고 작업을 안전하게 정리하는 중입니다."
          : "저장된 설정·작업 준비 중 · 다른 화면을 이용할 수 있습니다."}</span>
        {startup.phase !== "cancelling" ? <button type="button" onClick={() => void startup.cancel()}>시작 취소 및 종료</button> : null}
      </aside> : null}
      <UpdateDialog
        open={!tutorialOpen && updater.state.info !== null && ["available", "downloading", "installing", "error"].includes(updater.state.phase)}
        state={updater.state}
        onLater={updater.dismissUpdate}
        onInstall={() => void updater.installUpdate()}
      />
      <ExitConfirmDialog {...exit.dialogProps} />
      {toast ? <div key={toast.id} className="toast" role="status">{toast.message}</div> : null}
    </AppShellContext.Provider>
  );
}
