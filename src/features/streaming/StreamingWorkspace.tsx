import { useEffect, useState } from "react";
import type { OfficialBrowserApi } from "../../api/officialBrowser";
import type { ContentSource, WorkspaceViewId } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { OfficialBrowserPanel } from "./OfficialBrowserPanel";
import { MadoWorkspace } from "./MadoWorkspace";
import { AutoRecordingPanel } from "./AutoRecordingPanel";
import "./StreamingWorkspace.css";

type StreamingView = WorkspaceViewId<"chzzk">;
const viewStorageKey = "atsumi.chzzk.view.v1";
const isStreamingView = (view: unknown): view is StreamingView => view === "live" || view === "recordings" || view === "auto-record";
function readStreamingView(): StreamingView {
  try {
    const saved = localStorage.getItem(viewStorageKey);
    if (isStreamingView(saved)) return saved;
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
};

export function StreamingWorkspace({ runtime, active, railCollapsed, onToggleRail, onSourceChange, navigationRequest, onOpenSettings, tutorialActive = false }: StreamingWorkspaceProps) {
  const requestedView = navigationRequest?.source === "chzzk" && isStreamingView(navigationRequest.view) ? navigationRequest.view : null;
  const [view, setView] = useState<StreamingView>(() => requestedView ?? readStreamingView());
  // Live is the only presentation entry. Bookmarks and scheduled recordings
  // use that same channel selector; leaving it never owns recording lifetime.
  const navigate = (next: WorkspaceViewId<"chzzk">) => { setView(next); };
  useEffect(() => {
    if (navigationRequest?.source !== "chzzk") return;
    const next = navigationRequest.view;
    if (isStreamingView(next)) setView(next);
  }, [navigationRequest]);
  useEffect(() => {
    try { localStorage.setItem(viewStorageKey, view); } catch { /* Keep navigation usable when preferences cannot be saved. */ }
  }, [view]);
  if (!active) return null;
  return <div className={`app-shell streaming-shell${railCollapsed ? " sidebar-collapsed" : ""}`} hidden={!active} style={active ? undefined : { display: "none" }}>
    <SideRail source="chzzk" view={view} collapsed={railCollapsed} autoFindCount={0} attentionCount={0} sourceLabel="CHZZK" onNavigate={navigate} onSourceChange={onSourceChange} onToggle={onToggleRail} onSettings={onOpenSettings} />
    <main className={`streaming-workspace${view === "live" ? " is-official-view" : ""}`}>
      {view === "auto-record" ? <AutoRecordingPanel runtime={runtime} privacy={false} />
        : view === "live" ? <MadoWorkspace runtime={runtime} privacy={false} tutorialActive={tutorialActive} unifiedLive onLeave={() => {}} />
        : <OfficialBrowserPanel runtime={runtime} active={active} view="recordings" privacyMode={false} />}
    </main>
  </div>;
}
