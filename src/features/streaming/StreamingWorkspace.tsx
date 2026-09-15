import { useEffect, useState } from "react";
import type { OfficialBrowserApi } from "../../api/officialBrowser";
import type { ContentSource, WorkspaceViewId } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { OfficialBrowserPanel } from "./OfficialBrowserPanel";
import { MadoWorkspace } from "./MadoWorkspace";
import { AutoRecordingPanel } from "./AutoRecordingPanel";
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
  useEffect(() => {
    if (navigationRequest?.source !== "chzzk") return;
    const next = navigationRequest.view;
    if (next === "live" || next === "recordings" || next === "auto-record") setView(next);
  }, [navigationRequest]);
  if (!active) return null;
  return <div className={`app-shell streaming-shell${railCollapsed ? " sidebar-collapsed" : ""}`}>
    <SideRail source="chzzk" view={view} collapsed={railCollapsed} autoFindCount={0} attentionCount={0} sourceLabel="CHZZK" onNavigate={setView} onSourceChange={onSourceChange} onToggle={onToggleRail} />
    <main className={`streaming-workspace${view === "live" ? " is-official-view" : ""}`}>
      {view === "auto-record" ? <AutoRecordingPanel runtime={runtime} privacy={privacyMode} /> : mado && view === "live"
        ? <MadoWorkspace runtime={runtime} privacy={privacyMode} onLeave={() => setMado(false)} />
        : <OfficialBrowserPanel runtime={runtime} active={active} view={view} privacyMode={privacyMode} onMadoMode={() => setMado(true)} />}
    </main>
  </div>;
}
