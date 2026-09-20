import { useEffect, useState } from "react";
import type { OfficialBrowserApi } from "../../api/officialBrowser";
import type { ContentSource, WorkspaceViewId } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { OfficialBrowserPanel } from "./OfficialBrowserPanel";
import { MadoWorkspace } from "./MadoWorkspace";
import { AutoRecordingPanel } from "./AutoRecordingPanel";
import { AutoRecordingLiveView } from "./AutoRecordingLiveView";
import type { AutoWatchTarget } from "../../api/autoWatch";
import "./StreamingWorkspace.css";

export type StreamingWorkspaceProps = {
  navigationRequest?: import("../../app/CommonNavigation").NavigationRequest | null;
  runtime: OfficialBrowserApi["runtime"];
  active: boolean;
  railCollapsed: boolean;
  onToggleRail: () => void;
  onSourceChange: (source: ContentSource) => void;
  privacyMode: boolean;
};

export function StreamingWorkspace({ runtime, active, railCollapsed, onToggleRail, onSourceChange, privacyMode, navigationRequest }: StreamingWorkspaceProps) {
  const [view, setView] = useState<WorkspaceViewId<"chzzk">>("live");
  const [mado, setMado] = useState(false);
  const [autoWatch, setAutoWatch] = useState<AutoWatchTarget | null>(null);
  // Navigation changes presentation, not the lifetime of a borrowed receiver.
  const navigate = (next: WorkspaceViewId<"chzzk">) => { setView(next); };
  const watch = (target: AutoWatchTarget) => {
    if (target.kind === "automatic" && target.watchId) setAutoWatch(target);
    else { setAutoWatch(null); setMado(target.kind === "mado"); }
    setView("live");
  };
  useEffect(() => {
    if (navigationRequest?.source !== "chzzk") return;
    const next = navigationRequest.view;
    if (next === "live" || next === "recordings" || next === "auto-record") setView(next);
  }, [navigationRequest]);
  if (!active && !autoWatch) return null;
  return <div className={`app-shell streaming-shell${railCollapsed ? " sidebar-collapsed" : ""}`} hidden={!active} style={active ? undefined : { display: "none" }}>
    <SideRail source="chzzk" view={view} collapsed={railCollapsed} autoFindCount={0} attentionCount={0} sourceLabel="CHZZK" onNavigate={navigate} onSourceChange={onSourceChange} onToggle={onToggleRail} />
    <main className={`streaming-workspace${view === "live" ? " is-official-view" : ""}`}>
      {autoWatch ? <AutoRecordingLiveView target={autoWatch} runtime={runtime} active={active && view === "live"} privacy={privacyMode}
        onLeave={() => { setAutoWatch(null); navigate("auto-record"); }} /> : null}
      {!active ? null : view === "auto-record" ? <AutoRecordingPanel runtime={runtime} privacy={privacyMode} onWatch={watch} /> : autoWatch && view === "live"
        ? null : mado && view === "live"
        ? <MadoWorkspace runtime={runtime} privacy={privacyMode} onLeave={() => setMado(false)} />
        : <OfficialBrowserPanel runtime={runtime} active={active} view={view} privacyMode={privacyMode} onMadoMode={() => setMado(true)} onRecordOnly={() => navigate("recordings")} />}
    </main>
  </div>;
}
