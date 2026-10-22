import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { auditLayouts, layoutBrowser } from "../../test/offlineLayoutBrowser";
import { PixivWorkspace } from "./PixivWorkspace";
import { PixivHomeHeader } from "./PixivHomeHeader";
import css from "./PixivWorkspace.css?raw";
import appCss from "../../styles.css?raw";

describe.skipIf(!layoutBrowser)("Pixiv home recommendation geometry", () => {
  it.each([320, 720, 1280])("keeps tags bounded and search first at %ipx", async width => {
    const base = renderToStaticMarkup(<PixivWorkspace active runtime="browser-mock" privacyMode={false} view="explore" />);
    const tags = ["landscape", "風景", "illustration", "아주긴추천검색어".repeat(6), "sky", "drawing", "watercolor"];
    const home = renderToStaticMarkup(<PixivHomeHeader suggestions={{ favoriteTags: [...tags, "account-tag", "favorite-tag", "saved-search"], recommendedTags: tags, popularTags: [...tags, "photo", "city", "sketch"] }} authenticated retained disabled={false} onSearch={() => {}} onRefresh={() => {}} />);
    const html = base.replace('class="pixiv-workspace"', 'class="workspace pixiv-workspace"')
      .replace(/<div class="pixiv-home-heading">[\s\S]*?<\/button><\/div>/, home);
    const result = await auditLayouts([{ name: "pixiv-recommendations", html }], `${appCss}\n${css}\n.pixiv-workspace{display:block}`, width, 720, "pixiv-home");
    expect(result).toHaveLength(1);
    expect(result.flatMap(scene => scene.issues)).toEqual([]);
  }, 60000);
  it.each([320, 720, 1280])("keeps unavailable favorite-tag guidance compact at %ipx", async width => {
    const base = renderToStaticMarkup(<PixivWorkspace active runtime="browser-mock" privacyMode={false} view="explore" />);
    const home = renderToStaticMarkup(<PixivHomeHeader suggestions={{ recommendedTags: ["landscape"], popularTags: ["sky"] }} authenticated retained disabled={false} onSearch={() => {}} onRefresh={() => {}} />);
    const html = base.replace('class="pixiv-workspace"', 'class="workspace pixiv-workspace"')
      .replace(/<div class="pixiv-home-heading">[\s\S]*?<\/button><\/div>/, home);
    const result = await auditLayouts([{ name: "pixiv-favorite-tags-unavailable", html }], `${appCss}\n${css}\n.pixiv-workspace{display:block}`, width, 720, "pixiv-favorite-tags");
    expect(result.flatMap(scene => scene.issues)).toEqual([]);
  }, 60000);
});
