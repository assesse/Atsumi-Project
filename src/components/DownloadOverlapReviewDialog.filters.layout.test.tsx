import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { DownloadOverlapReview } from "../api/contracts";
import { galleryId } from "../core/types";
import { ThumbnailClient } from "../thumbnail";
import { auditLayouts, layoutBrowser, type LayoutScene } from "../test/offlineLayoutBrowser";
import { DownloadOverlapReviewDialog } from "./DownloadOverlapReviewDialog";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";
import dialogCss from "./DownloadOverlapReviewDialog.css?raw";
import mergeCss from "./OverlapMergeControls.css?raw";

const review: DownloadOverlapReview = {
  reviewId: "filter-layout-review", entryId: "incoming", revision: 1, state: "pending",
  profileVersion: 1, policyVersion: 1, incomingFingerprint: "b",
  incoming: { entryId: "incoming", galleryId: galleryId(2), title: "신규 판본 B", artists: ["Layout fixture"], pageCount: 6 },
  createdAt: "2026-10-17T00:00:00Z", updatedAt: "2026-10-17T00:00:00Z",
  candidates: [{
    candidateId: "layout-pair", existingFingerprint: "a", relation: "partial_overlap", confidence: 0.9,
    existing: { entryId: "existing", galleryId: galleryId(1), title: "기존 판본 A", artists: ["Layout fixture"], pageCount: 5 },
    matchedPages: 4, exactPages: 4, visualPages: 0, existingCoverage: 4 / 5, incomingCoverage: 4 / 6,
    existingUniquePages: 1, incomingUniquePages: 2, longestAlignedRun: 2, rank: 1,
    pagePairs: [[1, 1], [2, 2], [3, 4], [5, 6]].map(([a, b]) => ({
      existingSourcePage: a!, incomingSourcePage: b!, exactSha256: true,
      dHashDistance: 0, pHashDistance: 0, detailHashDistance: 0, edgeSimilarity: 1, visualSimilarity: 1, lowInformation: false,
    })),
  }],
};

async function filterScenes(): Promise<LayoutScene[]> {
  const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "layout fixture" }) });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<DownloadOverlapReviewDialog open={false} review={review} previewWidth={220}
      thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} onMergePages={vi.fn()} />));
    container.querySelector("dialog")!.setAttribute("open", "");
    const scene = (name: string): LayoutScene => ({ name, html: container.innerHTML });
    const toggle = async (label: string) => act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>(".download-overlap-page-filter")].find(b => b.textContent === label)!.click());
    const scenes = [scene("all-on")];
    const firstPage = container.querySelector<HTMLElement>('.download-overlap-page-cell[aria-label^="기존 A 1페이지"]')!;
    await act(async () => firstPage.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    await toggle("일치");
    expect(container.querySelector(".download-overlap-hidden-selection")).toHaveTextContent("선택 1장 숨김");
    scenes.push(scene("differences-hidden-selection"));
    await toggle("대응 없음");
    scenes.push(scene("differences-no-gap-decoration"));
    await toggle("한쪽에만 있음");
    scenes.push(scene("all-off-hidden-selection"));
    await toggle("대응 없음");
    scenes.push(scene("gaps-only"));
    // Include a focused view so the entire page axis is visible in captures,
    // even when album metadata above it requires scrolling in the full dialog.
    return scenes.flatMap(item => {
      const fragment = document.createElement("div");
      fragment.innerHTML = item.html;
      const map = fragment.querySelector(".download-overlap-page-map")!;
      return [item, { name: `${item.name}-page-map`, html: `<div style="width:min(1180px,calc(100vw - 32px));margin:32px auto">${map.outerHTML}</div>` }];
    });
  } finally {
    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  }
}

describe.skipIf(!layoutBrowser)("duplicate review visibility filter layout", () => {
  it.each([[1920, 1080], [1280, 720]])("keeps controls and hidden-selection feedback reachable at %ix%i", async (width, height) => {
    const results = await auditLayouts(await filterScenes(), appCss + inkCss + dialogCss + mergeCss,
      width, height, "overlap-page-filters");
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 60000);
});
