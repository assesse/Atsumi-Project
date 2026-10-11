import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { galleryGroupStorageKey, groupGalleries } from "../state/galleryGrouping";
import { auditLayouts, layoutBrowser } from "../test/offlineLayoutBrowser";
import { DownloadArtistFolderGrid } from "./DownloadArtistFolderGrid";
import { ThumbnailClient } from "../thumbnail";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";
import windowCss from "./DownloadArtistFolderWindow.css?raw";

const tags = ["female:big breasts", "female:ball sucking", "female:deepthroat", "female:fingering", "female:huge breasts", "female:very long favorite tag name"];
const items: Gallery[] = [1, 2].map(id => ({ id: galleryId(id), title: "작품", subtitle: "", artist: "Long artist name " + id,
  pages: 20, score: 0, publishedAt: "2026-10-07", coverIndex: 0, language: "korean", tags, series: [], characters: [],
  thumbnailWidth: 400, thumbnailHeight: 600, download: { entryId: `entry-${id}`, state: "completed", progress: 100, createdAt: "2026-10-07" } }));
const groups = groupGalleries(items, "artist", gallery => gallery.download?.createdAt);
const client = new ThumbnailClient({ resolve: () => ({ kind: "image", url: "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==", width: 400, height: 600 }) });
const scenes = [160, 220, 360].map(previewWidth => ({ name: `folder-${previewWidth}`, html: renderToStaticMarkup(
  <div className="gallery-viewport" style={{ padding: 20, height: "100vh" }}><DownloadArtistFolderGrid
    groups={groups} columns={2} previewWidth={previewWidth} favoriteMetadata={new Set(tags)} thumbnailClient={client}
    collapsedGroupKeys={new Set(groups.map(group => galleryGroupStorageKey("downloads", group)))}
    onToggle={() => {}} renderGrid={() => null} /></div>,
) }));

describe.skipIf(!layoutBrowser)("artist folder card layout", () => {
  it.each([[1050, 750], [1440, 900]])("does not inset covers or crop wrapped tag chips at %ix%i", async (width, height) => {
    const results = await auditLayouts(scenes, appCss + inkCss + windowCss, width, height, `artist-folders-${width}`);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 60000);
});
