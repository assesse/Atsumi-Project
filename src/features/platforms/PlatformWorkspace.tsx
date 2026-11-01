import { useEffect, useState } from "react";
import type { SupportedLinkRequest } from "../../app/supportedLinkNavigation";
import { useAppShell } from "../../app/AppShell";
import type { ContentSource, WorkspaceViewId } from "../../app/workspaceRegistry";
import { SideRail } from "../../components/SideRail";
import { LiveEdgePanel } from "../streaming/LiveEdgePanel";
import { ReloadableView } from "./ReloadableView";
import { useVideoPreviewWidth } from "./previewPreferences";
import { useImagePreviewWidth } from "./imagePreviewPreferences";
import "./PlatformWorkspace.css";

const loadYouTube = () => import("../youtube/YouTubeWorkspace").then(module => ({ default: module.YouTubeWorkspace }));
const loadSoop = () => import("../soop/SoopWorkspace").then(module => ({ default: module.SoopWorkspace }));
const loadPinterest = () => import("../pinterest/PinterestWorkspace").then(module => ({ default: module.PinterestWorkspace }));
const loadPixiv = () => import("../pixiv/PixivWorkspace").then(module => ({ default: module.PixivWorkspace }));
const loadX = () => import("../x/XWorkspace").then(module => ({ default: module.XWorkspace }));

export const platformSources = ["youtube", "soop", "pinterest", "pixiv", "x"] as const;
export type PlatformSource = typeof platformSources[number];
export const isPlatformSource = (source: ContentSource): source is PlatformSource => platformSources.some(item => item === source);

/** Only presentation is shared. Each provider owns its data and interaction state. */
export function PlatformWorkspace({ source, active, runtime, onSourceChange, supportedLinkRequest }: {
  source: PlatformSource; active: boolean; runtime: "tauri" | "browser-mock"; onSourceChange: (source: ContentSource) => void;
  supportedLinkRequest?: SupportedLinkRequest | null;
}) {
  const shell = useAppShell();
  const previewWidth = useVideoPreviewWidth(source === "youtube" || source === "soop" ? source : null);
  const imagePreviewWidth = useImagePreviewWidth(source === "pinterest" || source === "pixiv" ? source : null);
  // Retain the legacy route as an input for provider-owned saved-video migration.
  // It is no longer a separate navigation entry.
  const [videoView, setVideoView] = useState<"explore" | "live" | "favorites" | "screenshots" | "downloads">("live");
  const [imageView, setImageView] = useState<"explore" | "favorites" | "downloads">("explore");
  const [xView, setXView] = useState<WorkspaceViewId<"x">>("explore");
  const [xAccountTarget, setXAccountTarget] = useState<HTMLDivElement | null>(null);
  const [watching, setWatching] = useState(false);
  useEffect(() => {
    if (!active || supportedLinkRequest?.source !== source) return;
    if (source === "youtube" || source === "soop") setVideoView("explore");
    else if (source === "x") setXView("explore");
    else setImageView("explore");
  }, [active, source, supportedLinkRequest]);
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
      {source === "x" ? <SideRail {...rail} source="x" view={xView} onNavigate={setXView} footer={<div className="x-sidebar-slot" ref={setXAccountTarget}/>} /> : source === "youtube" || source === "soop" ? <SideRail {...rail} source={source} view={videoView === "favorites" ? "explore" : videoView} onNavigate={setVideoView} /> : <SideRail {...rail} source={source} view={imageView} onNavigate={setImageView} />}
    </LiveEdgePanel>
    <main className="platform-main">
      {source === "youtube" ? <ReloadableView load={loadYouTube} viewProps={{ ...common, previewWidth, view: videoView, onViewChange: setVideoView, onWatchingChange: setWatching, supportedLinkRequest }} />
        : source === "x" ? <ReloadableView load={loadX} viewProps={{ ...common, view: xView, accountPortalTarget: xAccountTarget, supportedLinkRequest }} />
        : source === "soop" ? <ReloadableView load={loadSoop} viewProps={{ ...common, previewWidth, view: videoView, onViewChange: setVideoView, onWatchingChange: setWatching, supportedLinkRequest }} />
        : source === "pinterest" ? <ReloadableView load={loadPinterest} viewProps={{ ...common, previewWidth: imagePreviewWidth, view: imageView, supportedLinkRequest }} />
        : <ReloadableView load={loadPixiv} viewProps={{ ...common, previewWidth: imagePreviewWidth, view: imageView, onViewChange: setImageView, supportedLinkRequest }} />}
    </main>
  </div>;
}
