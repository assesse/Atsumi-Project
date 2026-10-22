import { describe, expect, it } from "vitest";
import { auditLayouts, layoutBrowser, type LayoutScene } from "../../test/offlineLayoutBrowser";
import { IMAGE_PREVIEW_WIDTHS } from "./imagePreviewPreferences";
import appCss from "../../styles.css?raw";
import pinterestCss from "../pinterest/PinterestWorkspace.css?raw";
import pixivCss from "../pixiv/PixivWorkspace.css?raw";

const bookmark = '<svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 3h12v18l-6-4-6 4z" fill="none" stroke="currentColor" stroke-width="2" /></svg>';
const title = "긴 작품 제목을 포함한 미리보기 카드의 크기와 중첩 컨트롤 확인";
const artist = "A-very-long-artist-name-that-must-not-expand-the-card";

function cards(source: "pinterest" | "pixiv") {
  return Array.from({ length: 12 }, (_, index) => source === "pinterest"
    ? `<article class="pinterest-card is-selected" tabindex="0" aria-label="${title} ${index}">
        <div class="pinterest-image"></div>
        <span class="pinterest-check is-checked" role="checkbox" aria-checked="true">✓</span>
        <button type="button" class="pinterest-bookmark is-saved" aria-label="즐겨찾기 해제">${bookmark}</button>
        <div class="pinterest-card-copy"><strong>${title}</strong><span>${artist}</span><small>저장됨</small></div>
      </article>`
    : `<article class="pixiv-card is-focused" tabindex="0" aria-label="${title} ${index}">
        <div class="pixiv-image"></div>
        <span class="pixiv-card-check" role="checkbox" aria-checked="true">✓</span>
        <button type="button" class="pixiv-bookmark is-saved" aria-label="즐겨찾기 해제">${bookmark}</button>
        <span class="pixiv-saved-marker" aria-label="저장 완료">✓</span>
        <div class="pixiv-card-info"><strong>${title}</strong><button type="button" class="pixiv-artist">${artist}</button><small>12쪽 · 128 MB</small></div>
      </article>`).join("");
}

describe.skipIf(!layoutBrowser)("image preview grids: offline CSS overflow audit", () => {
  it.each([320, 720, 1280])("keeps all five presets and nested controls inside a %ipx viewport", async width => {
    const scenes: LayoutScene[] = (["pinterest", "pixiv"] as const).flatMap(source => IMAGE_PREVIEW_WIDTHS.map(previewWidth => ({
      name: `${source}-${previewWidth}`,
      // The workspace alias only enables the existing overflow auditor. Its
      // layout display is neutralized; provider padding, grid and cards use
      // their production CSS. No account, media fetch or app instance is used.
      html: `<section class="workspace ${source}-workspace" style="display:block;--${source}-preview-width:${previewWidth}px">
        <div class="${source}-grid">${cards(source)}</div>
      </section>`,
    })));
    const results = await auditLayouts(scenes, `${appCss}\n${pinterestCss}\n${pixivCss}`, width, 720, "image-preview");
    expect(results).toHaveLength(10);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 60000);
});
