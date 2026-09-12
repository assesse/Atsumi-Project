import { useState } from "react";
import type { OfficialBrowserApi } from "../../api/officialBrowser";
import type { ContentSource, WorkspaceViewId } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { OfficialBrowserPanel } from "./OfficialBrowserPanel";
import { MadoWorkspace } from "./MadoWorkspace";
import "./StreamingWorkspace.css";

export type StreamingWorkspaceProps = {
  runtime: OfficialBrowserApi["runtime"];
  active: boolean;
  railCollapsed: boolean;
  onToggleRail: () => void;
  onSourceChange: (source: ContentSource) => void;
  privacyMode: boolean;
};

export function StreamingWorkspace({ runtime, active, railCollapsed, onToggleRail, onSourceChange, privacyMode }: StreamingWorkspaceProps) {
  const [view, setView] = useState<WorkspaceViewId<"chzzk">>("live");
  const [mado, setMado] = useState(false);
  if (!active) return null;
  return <div className={`app-shell streaming-shell${railCollapsed ? " sidebar-collapsed" : ""}`}>
    <SideRail source="chzzk" view={view} collapsed={railCollapsed} autoFindCount={0} attentionCount={0} sourceLabel="CHZZK" onNavigate={setView} onSourceChange={onSourceChange} onToggle={onToggleRail} />
    <main className={`streaming-workspace${view === "live" ? " is-official-view" : ""}`}>
      {mado && view === "live"
        ? <MadoWorkspace runtime={runtime} privacy={privacyMode} onLeave={() => setMado(false)} />
        : <OfficialBrowserPanel runtime={runtime} active={active} view={view} privacyMode={privacyMode} onMadoMode={() => setMado(true)} />}
    </main>
  </div>;
}
