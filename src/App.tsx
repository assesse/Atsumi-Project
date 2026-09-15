import { useState, type ReactNode } from "react";
import { backend } from "./api/backend";
import { createDanbooruApi } from "./api/featureClients";
import { AppShell, useAppShell } from "./app/AppShell";
import type { ContentSource } from "./app/workspaceRegistry";
import { DanbooruWorkspace } from "./components/DanbooruWorkspace";
import { HitomiFeature } from "./features/hitomi/HitomiFeature";

import { StreamingWorkspace } from "./features/streaming/StreamingWorkspace";
import { createRecordingUpdateGuard } from "./api/recordingUpdateGuard";
import { CommonNavigationContext, type NavigationRequest } from "./app/CommonNavigation";
import { CommunityWorkspace } from "./features/community/CommunityWorkspace";
import type { WorkKey } from "./features/community/api";

const danbooruApi = createDanbooruApi(backend);

const updateGuard = createRecordingUpdateGuard(backend.runtime);

/** Composition only: shared services outlive presentation; feature rules stay inside features. */
function Workspaces() {
  const shell = useAppShell();
  const [communityOpen, setCommunityOpen] = useState(false);
  const [reviewWork, setReviewWork] = useState<WorkKey | null>(null);
  const [navigationRequest, setNavigationRequest] = useState<NavigationRequest | null>(null);
  const selectSource = (source: ContentSource) => { setCommunityOpen(false); setNavigationRequest(null); shell.selectSource(source); };
  const { settings, loading } = shell.settingsStore;
  return (
    <CommonNavigationContext.Provider value={{ communityOpen, openCommunity: (work) => { setReviewWork(work ?? null); setCommunityOpen(true); } }}>
    <HitomiFeature active={shell.source === "hitomi" && !communityOpen} navigationRequest={navigationRequest}>
      {(gallery) => {
        const workspaces = {
          hitomi: gallery.workspace,
          danbooru: (
            <DanbooruWorkspace
              navigationRequest={navigationRequest}
              backend={danbooruApi}
              railCollapsed={shell.railCollapsed}
              pageSize={settings.danbooruPageSize}
              previewWidth={settings.danbooruPreviewWidth}
              favoriteMetadata={gallery.favoriteMetadata}
              activityCount={gallery.activityCount}
              activityOpen={shell.activityOpen}
              privacyMode={settings.privacyMode}
              privacyModePending={shell.privacyModePending || loading}
              onToggleRail={shell.toggleRail}
              onSourceChange={selectSource}
              onActivity={gallery.onActivity}
              onPrivacyModeToggle={() => void shell.togglePrivacyMode()}
              onActivityRecord={gallery.onActivityRecord}
              onMetadataFavorite={gallery.onMetadataFavorite}
              onOpenSettings={() => shell.setSettingsOpen(true)}
            />
          ),
          chzzk: null,
        } satisfies Record<ContentSource, ReactNode>;
        return <>{communityOpen ? <CommunityWorkspace
          initialReview={reviewWork}
          source={shell.source} collapsed={shell.railCollapsed} attentionCount={gallery.activityCount}
          onToggleRail={shell.toggleRail} onSourceChange={selectSource}
          onNavigate={(view) => { setNavigationRequest((old) => ({ source: shell.source, view, sequence: (old?.sequence ?? 0) + 1 })); setCommunityOpen(false); }}
          onSettings={() => shell.setSettingsOpen(true)}
        /> : workspaces[shell.source]}<StreamingWorkspace
          runtime={backend.runtime}
          active={shell.source === "chzzk" && !communityOpen}
          navigationRequest={navigationRequest}
          railCollapsed={shell.railCollapsed}
          onToggleRail={shell.toggleRail}
          onSourceChange={selectSource}
          privacyMode={settings.privacyMode}

        /></>;
      }}
    </HitomiFeature>
    </CommonNavigationContext.Provider>
  );
}

export default function App() {
  return <AppShell api={backend} updateGuard={updateGuard}><Workspaces /></AppShell>;
}
