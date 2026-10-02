import { describe, expect, it } from "vitest";
import { applyGlobalSearchRules, matchesGlobalSearchRules, parseGlobalSearchTagInput } from "./globalSearchRules";

describe("global Explore search rules", () => {
  const gallery = { artist: "one", artists: ["one", "Second Artist"], language: "korean" as const,
    tags: ["female:glasses", "full color"], group: "A Group", series: ["A Series"], characters: ["A Hero"] };

  it("requires every configured tag and rejects any excluded tag with exact namespaced matches", () => {
    expect(matchesGlobalSearchRules(gallery, ["Female:Glasses", "tag:full_color"], [])).toBe(true);
    expect(matchesGlobalSearchRules(gallery, ["female:glasses", "webtoon"], [])).toBe(false);
    expect(matchesGlobalSearchRules(gallery, [], ["full_color"])).toBe(false);
    expect(matchesGlobalSearchRules(gallery, [], ["male:glasses", "glasses", "color"])).toBe(true);
    expect(matchesGlobalSearchRules(gallery, ["artist:second_artist", "group:a_group", "series:a_series", "character:a_hero"], [])).toBe(true);
  });

  it("is reversible without changing stored discovery metadata", () => {
    const original = JSON.stringify(gallery);
    expect(matchesGlobalSearchRules(gallery, [], ["female:glasses"])).toBe(false);
    expect(matchesGlobalSearchRules(gallery, [], [])).toBe(true);
    expect(JSON.stringify(gallery)).toBe(original);
  });
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
