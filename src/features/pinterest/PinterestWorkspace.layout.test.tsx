import { describe, expect, it } from "vitest";
import { auditLayouts, layoutBrowser, type LayoutScene } from "../../test/offlineLayoutBrowser";
import css from "./PinterestWorkspace.css?raw";

// The image uses a tab role only as a measurement probe recognized by the shared
// offline layout harness. Production images remain decorative <img alt="">.
const image = (width: number, height: number) => `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><rect width="100%" height="100%" fill="#657"/></svg>`)}`;
function scene(width: number, height: number, preview: boolean): LayoutScene {
  return { name: `${preview ? "preview" : "detail"}-${width}x${height}`, html: `<div class="pinterest-detail-backdrop"><section class="pinterest-detail${preview ? " is-preview show-controls" : ""}"><header><h2>공개 풍경</h2><button type="button">닫기</button></header><div class="pinterest-detail-body"><div class="pinterest-detail-media"><div class="pinterest-image"><img role="tab" aria-label="original-image-bounds" alt="" src="${image(width, height)}"></div></div>${preview ? "" : "<aside>공개 이미지 원본</aside>"}</div><footer><button type="button">즐겨찾기</button><button type="button">원본 저장</button></footer></section></div>` };
}
describe.skipIf(!layoutBrowser)("Pinterest original geometry", () => {
  it.each([[960, 640], [1280, 800]])("keeps original image boxes within their media pane at %ix%i", async (width, height) => {
    const scenes = [scene(1440, 2560, false), scene(2560, 1440, false), scene(1440, 2560, true)];
    const results = await auditLayouts(scenes, `body{margin:0}*{box-sizing:border-box}${css}`, width, height, `pinterest-original-${width}`);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 60000);
});
