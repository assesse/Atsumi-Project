import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ActivityDrawer } from "./ActivityDrawer";
import { QueueSummary } from "./WorkQueuePanel";
import { ExcludedAlbumDialog } from "./ExcludedAlbumDialog";
import { galleryId, type Gallery } from "../core/types";
import type { QueueSnapshot } from "../api/workConsole";
import { auditLayouts, layoutBrowser } from "../test/offlineLayoutBrowser";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";
import activityCss from "./ActivityDrawer.css?raw";
import queueCss from "./WorkQueuePanel.css?raw";
import dialogCss from "./ExcludedAlbumDialog.css?raw";
import processingCss from "./GalleryProcessingBadge.css?raw";

const noop = () => {};
const galleries: Gallery[] = (["downloading", "hashing", "verifying", "cancelled", "quarantined"] as const).map((state, index) => ({
  id: galleryId(index + 1), title: "앨범 제목 ".repeat(12), subtitle: "", artist: "작가", pages: 100, score: 0, coverIndex: 0,
  publishedAt: "2026-10-06", language: "korean", tags: [], series: [], characters: [],
  download: { entryId: `entry-${index}`, state, progress: 25 },
}));
const snapshot: QueueSnapshot = { queriedAt: "", counts: { downloading: 1, hashing: 1, verifying: 1, cancelled: 1, quarantined: 1 }, globalActive: 3,
  totalRows: 3, page: 1, pageSize: 100, items: [], batches: [], etaSeconds: null, recentCompleted: 0, lastProgressAt: null };
const activity = renderToStaticMarkup(<ActivityDrawer open galleries={galleries} sessionDownloads={galleries.map((item,index) => ({ galleryId: item.id, occurredAt: index }))}
  queueSummary={<QueueSummary snapshot={snapshot} />} queuePanel={<section>대기 큐</section>}
  onClose={noop} onReview={noop} onRetry={noop} onCancel={noop} />);
const dialog = renderToStaticMarkup(<ExcludedAlbumDialog context={{ galleryId: galleryId(1), reasons: [{ kind: "manual", detail: "사용자 제외 ".repeat(10), excludedAt: "2026-10-06" }],
  quarantined: false, quarantineEntryId: null, reviewId: "review", reviewGalleryId: galleryId(1), legacyCandidateId: null,
  retainedGallery: { galleryId: galleryId(2), title: "보존된 판본 제목 ".repeat(12) } }} title={"제외된 앨범 ".repeat(15)}
  onClose={noop} onRestore={async () => true} onReview={noop} onOpenRetained={noop} />).replace("<dialog", "<dialog open");

describe.skipIf(!layoutBrowser)("activity and exclusion offline layout", () => {
  it.each([[900, 650], [1440, 900]])("keeps long content and all actions inside the viewport at %ix%i", async (width, height) => {
    const results = await auditLayouts([{ name: "activity", html: activity }, { name: "exclusion-evidence", html: dialog }],
      appCss + inkCss + activityCss + queueCss + processingCss + dialogCss, width, height, `activity-exclusion-${width}`);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 60000);
});
