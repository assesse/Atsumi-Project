import { useState } from "react";
import { useAppShell } from "../../app/AppShell";
import type { ContentSource } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { LiveEdgePanel } from "../streaming/LiveEdgePanel";
import { ReloadableView } from "./ReloadableView";
import "./PlatformWorkspace.css";

const loadYouTube = () => import("../youtube/YouTubeWorkspace").then(module => ({ default: module.YouTubeWorkspace }));
const loadSoop = () => import("../soop/SoopWorkspace").then(module => ({ default: module.SoopWorkspace }));
const loadPinterest = () => import("../pinterest/PinterestWorkspace").then(module => ({ default: module.PinterestWorkspace }));
const loadPixiv = () => import("../pixiv/PixivWorkspace").then(module => ({ default: module.PixivWorkspace }));

export const platformSources = ["youtube", "soop", "pinterest", "pixiv"] as const;
export type PlatformSource = typeof platformSources[number];
export const isPlatformSource = (source: ContentSource): source is PlatformSource => platformSources.some(item => item === source);

/** Only presentation is shared. Each provider owns its data and interaction state. */
export function PlatformWorkspace({ source, active, runtime, onSourceChange }: {
  source: PlatformSource; active: boolean; runtime: "tauri" | "browser-mock"; onSourceChange: (source: ContentSource) => void;
}) {
  const shell = useAppShell();
  const [videoView, setVideoView] = useState<"explore" | "live" | "favorites" | "screenshots" | "downloads">("explore");
  const [imageView, setImageView] = useState<"explore" | "favorites" | "downloads">("explore");
  const [watching, setWatching] = useState(false);
  const videoSource = source === "youtube" || source === "soop";
  const immersive = videoSource && watching && videoView === "live" && !shell.tutorialOpen;
  const common = { active, runtime, privacyMode: shell.privacyMode, onNotice: shell.showToast };
  const rail = {
    collapsed: immersive ? false : shell.railCollapsed, autoFindCount: 0, attentionCount: 0,
    onSourceChange, onToggle: shell.toggleRail, onSettings: () => shell.setSettingsOpen(true),
    privacyMode: shell.privacyMode, privacyModePending: shell.privacyModePending,
    onPrivacyModeToggle: () => void shell.togglePrivacyMode(),
  };
  return <div className={`app-shell platform-shell${shell.railCollapsed && !immersive ? " sidebar-collapsed" : ""}${immersive ? " is-live-immersive" : ""}`} hidden={!active} inert={!active} style={active ? undefined : { display: "none" }}>
    <LiveEdgePanel edge="left" enabled={immersive} label="메뉴" closeKey={`${videoView}:${active}`} runtime={runtime}>
      {source === "youtube" || source === "soop" ? <SideRail {...rail} source={source} view={videoView} onNavigate={setVideoView} /> : <SideRail {...rail} source={source} view={imageView} onNavigate={setImageView} />}
    </LiveEdgePanel>
    <main className="platform-main">
      {source === "youtube" ? <ReloadableView load={loadYouTube} viewProps={{ ...common, view: videoView, onViewChange: setVideoView, onWatchingChange: setWatching }} />
        : source === "soop" ? <ReloadableView load={loadSoop} viewProps={{ ...common, view: videoView, onViewChange: setVideoView, onWatchingChange: setWatching }} />
        : source === "pinterest" ? <ReloadableView load={loadPinterest} viewProps={{ ...common, view: imageView }} />
        : <ReloadableView load={loadPixiv} viewProps={{ ...common, view: imageView, onViewChange: setImageView }} />}
    </main>
  </div>;
}
