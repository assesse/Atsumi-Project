import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../App";
import { backend } from "../api/backend";
import * as workConsole from "../api/workConsole";
import { communityApi } from "../features/community/api";
import { ReviewEditor } from "../features/community/ReviewEditor";
import { mockGalleries } from "../data/mockGalleries";
import { ThumbnailClient, ThumbnailProvider } from "../thumbnail";
import { setTutorialDismissed } from "../tutorial/tutorialPreference";
import { SelectionToolbar } from "../components/SelectionToolbar";
import { DetailWorkspace } from "../components/DetailWorkspace";
import { KeyboardShortcutsDialog } from "../components/KeyboardShortcutsDialog";
import { DownloadOverlapReviewDialog } from "../components/DownloadOverlapReviewDialog";
import { InternalDuplicateDialog } from "../components/InternalDuplicateDialog";
import { RecordingLibrary } from "../features/streaming/RecordingLibrary";
import { AutoRecordingPanel } from "../features/streaming/AutoRecordingPanel";
import type { BrowserRecording } from "../api/officialBrowser";
import type { DownloadOverlapReview, InternalDuplicateReview } from "../api/contracts";
import { galleryId } from "../core/types";
import { auditLayouts, layoutBrowser, type LayoutScene } from "../test/offlineLayoutBrowser";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";

// All component CSS, with the same base/theme order as main.tsx. Fixtures use
// actual React components; no real media, backend IPC or remote API is reached.
const componentCss = Object.values(import.meta.glob("../**/*.css", { query: "?raw", import: "default", eager: true })) as string[];
const styles = [...componentCss, appCss, inkCss].join("\n");
const noop = () => {};
const settle = () => new Promise(resolve => setTimeout(resolve, 30));

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

async function snapshots(): Promise<LayoutScene[]> {
  localStorage.clear(); sessionStorage.clear();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", () => 0);
  vi.stubGlobal("cancelAnimationFrame", noop);
  vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("Network is forbidden in layout fixtures"))));
  vi.spyOn(workConsole, "getQueueSnapshot").mockResolvedValue({ queriedAt: "2026-10-06T00:00:00Z", counts: {}, globalActive: 0, totalRows: 0, page: 1, pageSize: 100, items: [], batches: [], etaSeconds: null, recentCompleted: 0, lastProgressAt: null });
  vi.spyOn(communityApi, "feed").mockImplementation(async (source, _cursor, order) => ({ items: Array.from({ length: 8 }, (_, index) => ({
    id: `${order}-${index}`, source: source ?? "hitomi", workId: String(9900000 + index), nickname: "레이아웃 검사", rating: 4,
    recommended: true, comment: "긴 후기 내용 ".repeat(10), createdAt: "2026-10-06T00:00:00Z", updatedAt: "2026-10-06T00:00:00Z",
  })), nextCursor: null }));
  vi.spyOn(communityApi, "myReviews").mockResolvedValue({ profile: null, identityIssued: false, items: [], nextCursor: null });
  vi.spyOn(communityApi, "work").mockResolvedValue({ items: [], nextCursor: null });
  const duplicate = await backend.duplicateSnapshot();
  const internalScan = await backend.internalDuplicateSnapshot();
  if (!duplicate.ok || !internalScan.ok) throw new Error("Missing offline scan fixture");
  const scanTime = "2026-10-06T00:00:00Z";
  vi.spyOn(backend, "duplicateSnapshot").mockResolvedValue({ ok: true, data: { ...duplicate.data, run: {
    runId: "layout-duplicate", revision: 1, state: "cancelled", totalArtifacts: 354, hashedArtifacts: 354,
    totalPairs: 62435, comparedPairs: 0, candidatesFound: 0, startedAt: scanTime, updatedAt: scanTime,
  } } });
  vi.spyOn(backend, "internalDuplicateSnapshot").mockResolvedValue({ ok: true, data: { ...internalScan.data, run: {
    runId: "layout-internal", revision: 1, state: "completed", totalArtifacts: 5, scannedArtifacts: 5,
    totalPages: 800, comparedPairs: 20000, groupsFound: 46, algorithmVersion: 1, skippedArtifacts: 0,
    skippedPages: 0, startedAt: scanTime, updatedAt: scanTime,
  } } });
  vi.spyOn(backend, "autoFindSnapshot").mockResolvedValue({ ok: true, data: {
    run: { runId: "layout-auto", revision: 1, state: "completed", totalFavorites: 403, completedFavorites: 403,
      candidatesFound: 4867, startedAt: scanTime, updatedAt: scanTime, historyMode: "newer_than_latest_owned" },
    candidates: [], truncations: [], cutoffEvidence: Array.from({ length: 403 }, (_, index) => ({
      artist: `layout_artist_${index}_${"long_name_".repeat(5)}`, namespace: "artist", qualifiedOwnedCount: 20,
      latestOwnedGalleryId: galleryId(9900000 + index), source: "verified_owned_artifact", policyVersion: 2,
    })),
  } });
  for (const source of ["hitomi", "danbooru", "chzzk"] as const) setTutorialDismissed(true, source);
  const dialogDescriptors = ["showModal", "close"].map(name => [name, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, name)] as const);
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value() { this.setAttribute("open", ""); } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value() { this.removeAttribute("open"); } });
  const host = document.createElement("div"); host.id = "root"; document.body.append(host);
  const root = createRoot(host);
  const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "offline layout fixture" }) });
  const scenes: LayoutScene[] = [];
  const render = async (node: ReactNode) => act(async () => { root.render(<ThumbnailProvider client={client}>{node}</ThumbnailProvider>); await settle(); });
  const capture = (name: string) => scenes.push({ name, html: document.body.innerHTML });
  const click = async (selector: string) => {
    const element = document.querySelector<HTMLElement>(selector);
    if (!element) throw new Error(`Layout fixture missing ${selector}`);
    await act(async () => { element.click(); await settle(); });
  };
  try {
    await render(<App />); capture("hitomi-explore");
    await click('[aria-label="언어 필터"]'); capture("hitomi-language");
    await click('[aria-label="언어 필터"]');
    await click('[data-tour="hitomi-search"]'); capture("hitomi-search-results");
    await click('[data-tour="hitomi-nav-downloads"]'); capture("hitomi-downloads");
    await click('.result-details > summary'); capture("hitomi-downloads-status");
    await click('.result-details > summary');
    // Exercise the widest selection state, including an in-flight cancellation.
    const downloads = scenes.find(scene => scene.name === "hitomi-downloads")!.html;
    await render(<SelectionToolbar active count={1234} downloadsView cancelCount={1234} onCompare={noop} onCancelDownloads={noop} onAll={noop} onClear={noop} onPrimary={noop} onDelete={noop} />);
    const selected = host.innerHTML;
    const template = document.createElement("div"); template.innerHTML = downloads;
    template.querySelector(".selection-slot")?.replaceWith(Object.assign(document.createElement("div"), { innerHTML: selected }));
    scenes.push({ name: "hitomi-downloads-selected", html: template.innerHTML });
    await render(<App />);
    await click('[data-tour="hitomi-nav-auto-find"]'); capture("hitomi-auto-find");
    await click('.result-details > summary'); capture("hitomi-auto-find-status");
    await click('.result-details-popover details > summary'); capture("hitomi-auto-find-evidence");
    await click('.result-details > summary');
    await click('[data-tour="hitomi-nav-explore"]');
    await click('[aria-label="활동 기록"]'); capture("activity-queue");
    for (const [section, name] of [["session", "activity-session"], ["automation", "activity-review"]]) {
      const tab = document.querySelector<HTMLButtonElement>(`[aria-controls="activity-${section}-panel"]`);
      if (tab) { await act(async () => { tab.click(); await settle(); }); capture(name!); }
    }
    await click('[aria-label="활동 기록"]');
    await click('[data-tour="hitomi-settings"]');
    const settingsTabs = [...document.querySelectorAll<HTMLButtonElement>(".settings-nav button")];
    for (let i = 0; i < settingsTabs.length; i++) {
      await act(async () => { settingsTabs[i]!.click(); await settle(); }); capture(`settings-${i}`);
    }
    await click('[data-tour="settings-close"]');
    await click('[data-tour="hitomi-favorites"]'); capture("personal-library");
    await click('[data-tour="community-nav"]'); capture("community-feed");
    await click('[aria-label="인기 후기 더 보기"]'); capture("community-popular-dedicated");
    await click('[aria-label="돌아가기"]');
    await click('[aria-label="최신 후기 더 보기"]'); capture("community-latest-dedicated");
    await click('[aria-label="돌아가기"]');
    await click('[data-tour="source-menu"]'); await click('[data-tour="source-danbooru"]'); capture("danbooru-explore");
    await click('[data-tour="danbooru-nav-downloads"]'); capture("danbooru-downloads");
    await click('[data-tour="source-menu"]'); await click('[data-tour="source-chzzk"]'); capture("chzzk-live-setup");
    await click('[data-tour="chzzk-nav-recordings"]'); capture("chzzk-recordings");
    await click('[data-tour="chzzk-nav-auto-record"]'); capture("chzzk-auto-record");
    await render(<div className="community-workspace"><div className="community-content"><ReviewEditor work={{ source: "hitomi", workId: "9900000" }} api={communityApi}
      initialWriter={{ profile: { id: "layout-author", nickname: "레이아웃 검사" }, mine: { id: "layout-own", rating: 4, recommended: true, comment: "후기 수정 화면 검사", hidden: false } }} onClose={noop} onSaved={noop} /></div></div>);
    capture("community-review-editor");
    const galleries = mockGalleries.slice(0, 8).map(gallery => ({ ...gallery, download: undefined, title: `화면 검사 ${"긴 제목 ".repeat(10)}`, pages: 30, pageDimensions: Array.from({ length: 30 }, (_, i) => ({ sourcePage: i + 1, width: 800, height: 1200 })) }));
    await render(<DetailWorkspace tabs={galleries.map(g => g.id)} activeId={galleries[0]!.id} minimized={false} galleries={new Map(galleries.map(g => [g.id, g]))} favoriteMetadata={new Set()} thumbnailClient={client} onActivate={noop} onClose={noop} onCloseAll={noop} onMinimize={noop} onRestore={noop} onOpenRelated={noop} onQueue={noop} onMetadataSearch={noop} onMetadataFavorite={noop} />);
    capture("hitomi-details-many-tabs");
    await click('[aria-label="상세 전체 닫기"]'); capture("hitomi-close-all-confirm");
    const cancel = [...document.querySelectorAll<HTMLButtonElement>(".detail-close-dialog button")].find(button => button.textContent === "취소")!;
    await act(async () => cancel.click());
    await click('.preview-thumb[title="5페이지 확대"]'); capture("hitomi-page-preview");
    await render(<KeyboardShortcutsDialog open onClose={noop} />); capture("keyboard-help");
    const overlap: DownloadOverlapReview = {
      reviewId: "layout-review", entryId: "layout-incoming", revision: 1, state: "pending", profileVersion: 1, policyVersion: 1,
      incomingFingerprint: "a".repeat(64), createdAt: "2026-10-06T00:00:00Z", updatedAt: "2026-10-06T00:00:00Z",
      incoming: { entryId: "layout-incoming", galleryId: galleryId(200), title: "긴 앨범 제목 ".repeat(15), artists: ["작가 이름"], pageCount: 30 },
      candidates: ["near_equivalent", "incoming_contains_existing", "existing_contains_incoming", "partial_overlap"].map((relation, index) => ({
        candidateId: `candidate-${index}`, existing: { entryId: `layout-existing-${index}`, galleryId: galleryId(100 + index), title: "비교할 앨범 제목 ".repeat(10), artists: ["작가 이름"], pageCount: 30 },
        existingFingerprint: "b".repeat(64), relation: relation as DownloadOverlapReview["candidates"][number]["relation"], confidence: 0.94, matchedPages: 20, exactPages: 10, visualPages: 10, existingCoverage: 2 / 3, incomingCoverage: 2 / 3, existingUniquePages: 10, incomingUniquePages: 10, longestAlignedRun: 20, rank: index + 1,
        pagePairs: Array.from({ length: 20 }, (_, i) => ({ incomingSourcePage: i + 1, existingSourcePage: i + 1, exactSha256: i < 10, dHashDistance: 2, pHashDistance: 3, detailHashDistance: 19, edgeSimilarity: 0.91, visualSimilarity: 0.93, lowInformation: false })),
      })),
    };
    await render(<DownloadOverlapReviewDialog open review={overlap} previewWidth={320} thumbnailClient={client} onClose={noop} onRetry={noop} onRescan={noop} onDecision={noop} onMergePages={noop} />); capture("overlap-review");
    await render(<DownloadOverlapReviewDialog open review={overlap} completedPair previewWidth={600} thumbnailClient={client} onClose={noop} onRetry={noop} onRescan={noop} onDecision={noop} onMergePages={noop} />); capture("completed-pair-review");
    const internal: InternalDuplicateReview = { entryId: "layout-entry", galleryId: galleryId(101), title: "내부 중복 검사 제목 ".repeat(15), quarantineRecords: [], groups: Array.from({ length: 4 }, (_, i) => ({
      groupId: `group-${i}`, blockId: `block-${i}`, sequenceIndex: i, revision: 1, entryId: "layout-entry", galleryId: galleryId(101), relation: "exact", confidence: 1, recommendedKeepSourcePage: i + 1,
      pages: [i + 1, i + 10].map(sourcePage => ({ sourcePage, exactSha256: true, visualSimilarity: 1, detailHashDistance: 0, lowInformation: false })), resolved: false, createdAt: "2026-10-06T00:00:00Z", updatedAt: "2026-10-06T00:00:00Z",
    })) };
    await render(<InternalDuplicateDialog open review={internal} thumbnailClient={client} onClose={noop} onRetry={noop} onRescan={noop} onPlan={noop} onApply={noop} onUndo={noop} />); capture("internal-duplicate-review");
    const recordings: BrowserRecording[] = ["recording", "stopped", "interrupted", "failed"].map((status, i) => ({ id: `layout-recording-${i}`, channelId: `channel-${i}`, title: "방송 녹화 긴 제목 ".repeat(15), startedAt: 1800000000000, updatedAt: 1800000001000, status: status as BrowserRecording["status"], mimeType: "video/webm", outputDir: "C:\\SyntheticOnly", segmentCount: 2, bytesWritten: 2000, durationSeconds: 30, lastError: i > 1 ? "재현용 긴 오류 설명 ".repeat(10) : null, segments: [{ index: 0, file: "segment-000000000000.webm", bytes: 1000, durationSeconds: 15 }] }));
    const streamingShell = (children: ReactNode) => <div className="app-shell streaming-shell"><aside className="sidebar" /><main className="streaming-workspace is-official-view">{children}</main></div>;
    await render(streamingShell(<RecordingLibrary recordings={recordings} selectedId={recordings[3]!.id} disabled={false} privacy={false} retrying={false} openingFolder={false} onSelect={noop} onFolder={noop} onReplay={noop} onRetryMerge={noop} onDelete={async () => ({ deletedIds: [], failures: [] })} />)); capture("chzzk-recordings-populated");
    const autoData = { ok: true as const, data: { captureChat: true, error: null, channels: ["recording", "waiting", "attention"].map((status, i) => ({ channelId: String(i).repeat(32), channelName: "자동 녹화 채널 이름 ".repeat(10), enabled: true, checkedAt: 1000, status, recordingId: status === "recording" ? "layout-recording-0" : null, message: "상태 설명 ".repeat(10) })) } };
    await render(streamingShell(<AutoRecordingPanel runtime="browser-mock" api={{ snapshot: async () => autoData, add: async () => autoData, update: async () => autoData }} />)); capture("chzzk-auto-record-populated");
    return scenes;
  } finally {
    await act(async () => root.unmount()); host.remove(); client.dispose();
    for (const [name, descriptor] of dialogDescriptors) { if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, name, descriptor); else Reflect.deleteProperty(HTMLDialogElement.prototype, name); }
  }
}

describe.skipIf(!layoutBrowser)("global Full HD work-area layout", () => {
  // Full HD and conservative usable areas with titlebar/taskbar at 100/125/150%.
  it.each([[1920, 1080], [1920, 1000], [1536, 780], [1280, 640], [1024, 680]])("keeps controls reachable at %ix%i", async (width, height) => {
    const scenes = await snapshots();
    const results = await auditLayouts(scenes, styles, width, height);
    expect(results.length).toBe(scenes.length);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 180000);
});
