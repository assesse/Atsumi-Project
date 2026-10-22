import { useEffect, useState } from "react";
import type { OfficialBrowserApi } from "../../api/officialBrowser";
import type { ContentSource, WorkspaceViewId } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { OfficialBrowserPanel } from "./OfficialBrowserPanel";
import { MadoWorkspace } from "./MadoWorkspace";
import { LiveEdgePanel } from "./LiveEdgePanel";
import { ScreenshotLibrary } from "./ScreenshotLibrary";
import "./StreamingWorkspace.css";

type StreamingView = WorkspaceViewId<"chzzk">;
const viewStorageKey = "atsumi.chzzk.view.v1";
const normalizeStreamingView = (view: unknown): StreamingView | null => view === "recordings" || view === "screenshots" ? view : view === "live" || view === "auto-record" ? "live" : null;
function readStreamingView(): StreamingView {
  try {
    const saved = localStorage.getItem(viewStorageKey);
    const view = normalizeStreamingView(saved);
    if (view) return view;
  } catch { /* Optional presentation preference; navigation remains available. */ }
  return "live";
}

export type StreamingWorkspaceProps = {
  tutorialActive?: boolean;
  navigationRequest?: import("../../app/CommonNavigation").NavigationRequest | null;
  runtime: OfficialBrowserApi["runtime"];
  active: boolean;
  railCollapsed: boolean;
  onToggleRail: () => void;
  onSourceChange: (source: ContentSource) => void;
  onOpenSettings?: () => void;
  privacyMode?: boolean;
  privacyModePending?: boolean;
  onPrivacyModeToggle?: () => void;
};

export function StreamingWorkspace({ runtime, active, railCollapsed, onToggleRail, onSourceChange, navigationRequest, onOpenSettings, tutorialActive = false, privacyMode = false, privacyModePending = false, onPrivacyModeToggle }: StreamingWorkspaceProps) {
  const requestedView = navigationRequest?.source === "chzzk" ? normalizeStreamingView(navigationRequest.view) : null;
  const [view, setView] = useState<StreamingView>(() => requestedView ?? readStreamingView());
  const [watching, setWatching] = useState(false);
  const immersive = watching && view === "live" && !tutorialActive;
  // Live is the only presentation entry. Bookmarks and scheduled recordings
  // use that same channel selector; leaving it never owns recording lifetime.
  const navigate = (next: WorkspaceViewId<"chzzk">) => {
    const target = normalizeStreamingView(next) ?? "live";
    setView(target);
  };
  useEffect(() => {
    if (navigationRequest?.source !== "chzzk") return;
    const next = normalizeStreamingView(navigationRequest.view);
    if (next) setView(next);
  }, [navigationRequest]);
  useEffect(() => {
    try { localStorage.setItem(viewStorageKey, view); } catch { /* Keep navigation usable when preferences cannot be saved. */ }
  }, [view]);
  if (!active) return null;
  return <div className={`app-shell streaming-shell${railCollapsed && !immersive ? " sidebar-collapsed" : ""}${immersive ? " is-live-immersive" : ""}`} hidden={!active} style={active ? undefined : { display: "none" }}>
    <LiveEdgePanel edge="left" enabled={immersive} label="메뉴" closeKey={view} runtime={runtime}>
      <SideRail source="chzzk" view={view} collapsed={immersive ? false : railCollapsed} autoFindCount={0} attentionCount={0} onNavigate={navigate} onSourceChange={onSourceChange} onToggle={onToggleRail} onSettings={onOpenSettings} privacyMode={privacyMode} privacyModePending={privacyModePending} onPrivacyModeToggle={onPrivacyModeToggle} />
    </LiveEdgePanel>
    <main className={`streaming-workspace${view === "live" ? " is-official-view is-unified-live" : ""}`}>
      {view === "live" ? <MadoWorkspace runtime={runtime} privacy={privacyMode} tutorialActive={tutorialActive} onWatchingChange={setWatching} unifiedLive onLeave={() => {}} />
        : view === "screenshots" ? <ScreenshotLibrary runtime={runtime} privacy={privacyMode} />
        : <OfficialBrowserPanel runtime={runtime} active={active} view="recordings" privacyMode={privacyMode} />}
    </main>
  </div>;
}
