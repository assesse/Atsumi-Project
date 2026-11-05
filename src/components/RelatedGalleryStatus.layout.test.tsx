import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { mockGalleries } from "../data/mockGalleries";
import type { Gallery } from "../core/types";
import { GalleryProcessingBadge, GalleryProcessingSurface } from "./GalleryProcessingBadge";
import { auditLayouts, layoutBrowser } from "../test/offlineLayoutBrowser";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";
import processingCss from "./GalleryProcessingBadge.css?raw";

const samples = [
  { download: { entryId: "1", state: "completed" }, exclusionLabel: "중복 판정으로 제외" },
  { download: undefined, exclusionLabel: "탐색에서 제외" },
  { download: { entryId: "3", state: "quarantined" } },
  { download: undefined, duplicateCount: 2 },
  { download: { entryId: "5", state: "completed" } },
  { download: { entryId: "6", state: "hashing" } },
] satisfies ({ download: Gallery["download"]; exclusionLabel?: string; duplicateCount?: number })[];
const html = renderToStaticMarkup(<main style={{ padding: 24 }}>
  <section className="related-list">{samples.map((sample, index) => {
    const { download, ...options } = sample;
    const gallery: Gallery = { ...mockGalleries[index]!, title: "긴 관련 앨범 제목과 이미 처리된 판본 표시", download };
    return <GalleryProcessingSurface className="related-card" gallery={gallery} {...options} key={gallery.id} tabIndex={0}>
      <div className="related-cover" style={{ position: "relative", height: 180, overflow: "hidden" }}>
        <svg className="thumbnail-image" width="120" height="180" viewBox="0 0 120 180" role="img" aria-label="검사용 표지">
          <rect width="120" height="180" fill="#555168"/><circle cx="60" cy="60" r="28" fill="#c5bacd"/>
          <path d="M10 175 Q10 95 60 95 Q110 95 110 175" fill="#9981a9"/>
        </svg>
        <span className="processing-preview-wash" aria-hidden="true"/>
        <GalleryProcessingBadge gallery={gallery} {...options} overlay/>
      </div>
      <div className="related-copy card-content"><strong>{gallery.title}</strong><span>작가 · {index + 1}</span><div className="meta-bottom">24p · #{gallery.id}</div></div>
    </GalleryProcessingSurface>;
  })}</section></main>);
describe.skipIf(!layoutBrowser)("related gallery statuses offline layout", () => {
  it.each([[1280,720],[1920,1080]])("keeps badges and titles inside related cards at %ix%i", async (width,height) => {
    const css = appCss + inkCss + processingCss + ".related-list{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}.related-card{--related-preview-width:120px}.related-cover{width:120px}.related-copy{padding:14px;min-width:0}.related-copy strong{overflow-wrap:anywhere}";
    const result = await auditLayouts([{name:"related-statuses", html}], css, width, height, "related-status-" + width);
    expect(result.flatMap(r=>r.issues)).toEqual([]);
  }, 60000);
});
