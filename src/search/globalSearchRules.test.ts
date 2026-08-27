import { describe, expect, it } from "vitest";
import { applyGlobalSearchRules, parseGlobalSearchTagInput } from "./globalSearchRules";

describe("global Explore search rules", () => {
  it("parses newline and comma separated tags into a stable unique list", () => {
    expect(parseGlobalSearchTagInput(" Female:Glasses\nartist:Sugoi_Hi, female:glasses ")).toEqual([
      "artist:sugoi_hi",
      "female:glasses",
    ]);
  });

  it("applies saved rules to one-off searches and lets global exclusions win", () => {
    expect(applyGlobalSearchRules({
      text: "artist:sugoi_hi",
      includeTags: ["female:glasses", "full_color"],
      excludeTags: ["male:glasses"],
      languages: ["korean"],
      sort: "recent",
      pageSize: 50,
    }, ["female:glasses", "webtoon"], ["full_color"])).toEqual({
      text: "artist:sugoi_hi",
      includeTags: ["female:glasses", "webtoon"],
      excludeTags: ["full_color", "male:glasses"],
      languages: ["korean"],
      sort: "recent",
      pageSize: 50,
    });
  });
});
