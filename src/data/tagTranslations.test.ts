import { describe, expect, it } from "vitest";
import rawTranslations from "./tagTranslations.ko.json";
import { normalizeTagTranslationKey, tagTooltip, tagTranslationEntryCount } from "./tagTranslations";

describe("tag translations", () => {
  it("loads the supplied string-to-string match file without changing its contents", () => {
    const entries = Object.entries(rawTranslations);
    expect(tagTranslationEntryCount).toBe(7280);
    expect(entries.every(([, value]) => typeof value === "string")).toBe(true);
    expect(entries.filter(([, value]) => value === "")).toHaveLength(6429);
    expect(entries.filter(([key, value]) => value !== "" && value.trim().toLowerCase() === key.trim().toLowerCase())).toHaveLength(217);
    expect(entries.filter(([key, value]) => value !== "" && value.trim().toLowerCase() !== key.trim().toLowerCase())).toHaveLength(634);
  });

  it("normalizes namespaces, underscores, whitespace, and case", () => {
    expect(normalizeTagTranslationKey("female:mind_control")).toBe("mind control");
    expect(normalizeTagTranslationKey("tag:full__color")).toBe("full color");
    expect(normalizeTagTranslationKey("  WEBTOON  ")).toBe("webtoon");
  });

  it("uses a real translation only when it differs from the English key", () => {
    expect(tagTooltip("female:mind_control")).toMatchObject({ key: "mind control", text: "정신조종", language: "ko" });
    expect(tagTooltip("tag:western_cg")).toMatchObject({ text: "western cg", language: "en" });
    expect(tagTooltip("tag:comic")).toMatchObject({ text: "comic", language: "en" });
    expect(tagTooltip("male:unknown_rare_tag")).toMatchObject({ text: "unknown rare tag", language: "en" });
    expect(tagTooltip("tag:yaoi")).toMatchObject({ text: "BL", language: "ko" });
  });
});
