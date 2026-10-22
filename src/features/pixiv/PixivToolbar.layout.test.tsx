import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { auditLayouts, layoutBrowser } from "../../test/offlineLayoutBrowser";
import { PixivWorkspace } from "./PixivWorkspace";
import css from "./PixivWorkspace.css?raw";
import appCss from "../../styles.css?raw";

describe.skipIf(!layoutBrowser)("Pixiv search-first toolbar geometry", () => {
  it.each([320, 720, 1280])("keeps search above or beside account controls at %ipx", async width => {
    // Server-render the real workspace; effects/native API never run. The long
    // unknown-account label is also the most constrained toolbar state.
    const scenes = (["explore", "favorites", "downloads"] as const).map(view => ({
      name: `pixiv-toolbar-${view}`,
      html: renderToStaticMarkup(<PixivWorkspace active runtime="browser-mock" privacyMode={false} view={view} />)
        .replace('class="pixiv-workspace"', 'class="workspace pixiv-workspace"'),
    }));
    const results = await auditLayouts(scenes, `${appCss}\n${css}\n.pixiv-workspace{display:block}`, width, 720, "pixiv-toolbar");
    expect(results).toHaveLength(3);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 60000);
});
