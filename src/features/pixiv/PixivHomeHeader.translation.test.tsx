import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { auditLayouts, layoutBrowser } from "../../test/offlineLayoutBrowser";
import { PixivHomeHeader } from "./PixivHomeHeader";
import type { PixivHomeSuggestions } from "./model";
import appCss from "../../styles.css?raw";
import workspaceCss from "./PixivWorkspace.css?raw";
import translationCss from "./PixivHomeHeader.css?raw";

const suggestions: PixivHomeSuggestions = {
  favoriteTags: ["イラスト", "OC", "創作", "オリジナル", "untranslated"],
  recommendedTags: ["イラスト"], popularTags: ["landscape"],
  favoriteTagTranslations: [
    { tag: "イラスト", translation: "일러스트", language: "ko" },
    { tag: "OC", translation: "original character", language: "en" },
    { tag: "創作", translation: "창작", language: "ko" },
    { tag: "オリジナル", translation: "창작", language: "ko" },
  ],
};
let host: HTMLDivElement, root: Root;
const search = vi.fn();
async function render(value: PixivHomeSuggestions | undefined = suggestions, authenticated = true) {
  await act(async () => root.render(<PixivHomeHeader suggestions={value} authenticated={authenticated} disabled={false} retained onSearch={search} onRefresh={() => {}} />));
}
function favoriteButtons() { return [...host.querySelectorAll<HTMLButtonElement>(".pixiv-discovery-tags>div:first-child button")]; }

describe("Pixiv official favorite-tag translations", () => {
  beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement("div"); document.body.append(host); root = createRoot(host); search.mockClear(); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
  it("shows subordinate Korean/English while keeping original order and original click queries", async () => {
    await render();
    const buttons = favoriteButtons();
    expect(buttons.map(button => button.querySelector(".pixiv-tag-original")?.textContent)).toEqual(suggestions.favoriteTags);
    expect(buttons[0]!.querySelector(".pixiv-tag-translation")?.textContent).toBe("일러스트");
    expect(buttons[0]!.querySelector(".pixiv-tag-translation")?.getAttribute("lang")).toBe("ko");
    expect(buttons[1]!.querySelector(".pixiv-tag-translation")?.getAttribute("lang")).toBe("en");
    expect(buttons[0]!.title).toBe(""); // One shared hover tooltip, not two competing popups.
    expect(buttons[0]!.dataset.pixivMetadata).toBe("tag");
    expect(buttons[4]!.textContent).toBe("untranslated");
    await act(async () => buttons[0]!.click());
    expect(search).toHaveBeenCalledExactlyOnceWith("イラスト");
  });
  it("keeps two distinct original tags that share one translation and does not translate other groups", async () => {
    await render();
    const translated = [...host.querySelectorAll(".pixiv-tag-translation")];
    expect(translated.filter(node => node.textContent === "창작")).toHaveLength(2);
    expect(host.querySelectorAll(".pixiv-discovery-tags>div:nth-child(2) .pixiv-tag-translation")).toHaveLength(0);
    expect(favoriteButtons()).toHaveLength(5);
  });
  it.each([undefined, null, {}, "bad", Array.from({ length: 11 }, () => suggestions.favoriteTagTranslations![0])])("keeps originals with unavailable or excessive translation data (%j)", async translations => {
    await render({ ...suggestions, favoriteTagTranslations: translations } as PixivHomeSuggestions);
    expect(favoriteButtons().map(button => button.textContent)).toEqual(suggestions.favoriteTags);
    expect(host.querySelector(".pixiv-tag-translation")).toBeNull();
    expect(host.querySelector(".pixiv-favorite-tags-error")).toBeNull();
  });
  it.each(["", "   ", "イラスト", "line\nbreak", "tab\t", "x".repeat(121), null, 4, {}])("ignores an invalid translation without losing valid originals (%j)", async translation => {
    await render({ ...suggestions, favoriteTagTranslations: [{ tag: "イラスト", translation, language: "ko" }] } as PixivHomeSuggestions);
    expect(favoriteButtons()).toHaveLength(5);
    expect(host.querySelector(".pixiv-tag-translation")).toBeNull();
  });
  it("requires exact canonical keys and ko/en, while retaining bounded Unicode strings as plain text", async () => {
    await render({ ...suggestions, favoriteTagTranslations: [
      { tag: " イラスト ", translation: "bad key", language: "ko" },
      { tag: "unrelated", translation: "unrelated", language: "ko" },
      { tag: "OC", translation: "not supported", language: "romaji" },
      { tag: "創作", translation: "가".repeat(120), language: "ko" },
      { tag: "オリジナル", translation: "<img src=x onerror=alert(1)>", language: "en" },
    ] } as PixivHomeSuggestions);
    expect(host.querySelectorAll(".pixiv-tag-translation")).toHaveLength(2);
    expect(host.querySelector("img")).toBeNull();
    expect(host.textContent).toContain("<img src=x onerror=alert(1)>");
  });
  it("hides account-only translations together with originals after auth loss or cleared suggestions", async () => {
    await render();
    expect(host.textContent).toContain("일러스트");
    await render(suggestions, false);
    expect(host.textContent).not.toContain("즐겨찾기 태그");
    expect(host.textContent).not.toContain("일러스트");
    expect(host.textContent).toContain("landscape");
    await act(async () => root.render(<PixivHomeHeader authenticated disabled retained onSearch={search} onRefresh={() => {}} />));
    expect(host.querySelector(".pixiv-discovery-tags")).toBeNull();
  });
  it("does not let invalid favorite data expose otherwise valid translations", async () => {
    await render({ ...suggestions, favoriteTags: undefined });
    expect(host.querySelector(".pixiv-tag-translation")).toBeNull();
    expect(host.querySelector(".pixiv-favorite-tags-error")).not.toBeNull();
  });
});

describe.skipIf(!layoutBrowser)("Pixiv translated home-tag geometry", () => {
  it.each([320, 720, 1280])("keeps original and translation controls inside %ipx", async width => {
    const longTags = Array.from({ length: 10 }, (_, index) => `원문${index}${"長".repeat(40)}`);
    const sample: PixivHomeSuggestions = { ...suggestions, favoriteTags: longTags,
      favoriteTagTranslations: longTags.map(tag => ({ tag, translation: "공식 번역 ".repeat(15), language: "ko" })) };
    const html = renderToStaticMarkup(<div className="workspace pixiv-workspace"><PixivHomeHeader suggestions={sample} authenticated disabled={false} retained onSearch={() => {}} onRefresh={() => {}} /></div>);
    const result = await auditLayouts([{ name: "official-favorite-translations", html }], `${appCss}\n${workspaceCss}\n${translationCss}\n.pixiv-workspace{display:block}`, width, 900, "pixiv-favorite-translations");
    expect(result.flatMap(scene => scene.issues)).toEqual([]);
  }, 60000);
});
