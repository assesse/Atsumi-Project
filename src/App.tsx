import type { ReactNode } from "react";
import { backend } from "./api/backend";
import { createDanbooruApi } from "./api/featureClients";
import { AppShell, useAppShell } from "./app/AppShell";
import type { ContentSource } from "./app/workspaceRegistry";
import { DanbooruWorkspace } from "./components/DanbooruWorkspace";
import { HitomiFeature } from "./features/hitomi/HitomiFeature";

import { StreamingWorkspace } from "./features/streaming/StreamingWorkspace";
import { createRecordingUpdateGuard } from "./api/recordingUpdateGuard";

const danbooruApi = createDanbooruApi(backend);

const updateGuard = createRecordingUpdateGuard(backend.runtime);

/** Composition only: shared services outlive presentation; feature rules stay inside features. */
function Workspaces() {
  const shell = useAppShell();
  const { settings, loading } = shell.settingsStore;
  return (
    <HitomiFeature active={shell.source === "hitomi"}>
      {(gallery) => {
        const workspaces = {
          hitomi: gallery.workspace,
          danbooru: (
            <DanbooruWorkspace
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
              onSourceChange={shell.selectSource}
              onActivity={gallery.onActivity}
              onPrivacyModeToggle={() => void shell.togglePrivacyMode()}
              onActivityRecord={gallery.onActivityRecord}
              onMetadataFavorite={gallery.onMetadataFavorite}
              onOpenSettings={() => shell.setSettingsOpen(true)}
            />
          ),
          chzzk: null,
        } satisfies Record<ContentSource, ReactNode>;
        return <>{workspaces[shell.source]}<StreamingWorkspace
          runtime={backend.runtime}
          active={shell.source === "chzzk"}
          railCollapsed={shell.railCollapsed}
          onToggleRail={shell.toggleRail}
          onSourceChange={shell.selectSource}
          privacyMode={settings.privacyMode}

        /></>;
      }}
    </HitomiFeature>
  );
}

export default function App() {
  return <AppShell api={backend} updateGuard={updateGuard}><Workspaces /></AppShell>;
}
