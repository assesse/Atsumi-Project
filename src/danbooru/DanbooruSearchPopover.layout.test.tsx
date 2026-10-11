import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { DanbooruSearchFilterPanel } from "../components/DanbooruWorkspace";
import { auditLayouts, layoutBrowser } from "../test/offlineLayoutBrowser";
import { defaultDanbooruSearchFilters } from "./searchPreferences";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";
import popoverCss from "./DanbooruSearchPopover.css?raw";
import dateCss from "./DanbooruDateRange.css?raw";

const scenes = [false, true].map(open => ({ name: open ? "filters-open" : "filters-closed", html: renderToStaticMarkup(
  <div style={{ height: "100vh", paddingLeft: "min(22vw, 220px)" }}>
    <main className="danbooru-workspace" style={{ height: "100%" }}>
      <header className="danbooru-header"><span>검색</span></header>
      <div className="danbooru-overview"><section className="danbooru-search-tools">
        <button className="danbooru-filter-button">상세 조건</button>
        {open ? <DanbooruSearchFilterPanel filters={defaultDanbooruSearchFilters()} onChange={() => {}} onApply={() => {}} onReset={() => {}} onClose={() => {}} /> : null}
      </section><section className="danbooru-heading"><div><h1>Danbooru post 탐색</h1><p>검색 결과</p></div></section></div>
      <section className="danbooru-content"><div className="gallery-viewport" style={{ height: "100%" }}>앨범 목록</div></section>
    </main>
  </div>,
) }));

describe.skipIf(!layoutBrowser)("Danbooru compact filter overlay", () => {
  it.each([[700, 600], [1050, 750], [1440, 900]])("keeps calendar controls usable and content available at %ix%i", async (width, height) => {
    const results = await auditLayouts(scenes, appCss + inkCss + popoverCss + dateCss, width, height, `danbooru-popover-${width}`);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
    expect(popoverCss).toContain("position: absolute");
    expect(popoverCss).toContain("overflow: visible");
  }, 60_000);
});
