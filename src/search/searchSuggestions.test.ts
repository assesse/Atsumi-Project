import { describe, expect, it } from "vitest";
import { buildSearchSuggestionCatalog, catalogSuggestion, historyDisplayToken } from "./searchSuggestions";

describe("search suggestion catalog", () => {
  it("keeps structured history only when the field is empty", () => {
    const catalog = buildSearchSuggestionCatalog([{ historyId: 1, text: "", includeTags: ["story arc"], excludeTags: [], languages: [], sort: "recent", pageSize: 50, useCount: 3, lastUsedAt: "2026-08-21" }]);
    expect(historyDisplayToken({ historyId: 2, text: "", includeTags: [], excludeTags: ["webtoon"], languages: [], sort: "recent", pageSize: 50, useCount: 1, lastUsedAt: "" })).toBe("-tag:webtoon");
    expect(catalog).toHaveLength(1);
    expect(catalog[0]?.token).toBe("tag:story_arc");
  });

  it("does not duplicate the same search when only its saved page size changed", () => {
    const catalog = buildSearchSuggestionCatalog([
      { historyId: 2, text: "archive", includeTags: [], excludeTags: [], languages: ["korean"], sort: "recent", pageSize: 80, useCount: 2, lastUsedAt: "2026-08-31" },
      { historyId: 1, text: "archive", includeTags: [], excludeTags: [], languages: ["korean"], sort: "recent", pageSize: 50, useCount: 1, lastUsedAt: "2026-08-30" },
    ]);

    expect(catalog).toHaveLength(1);
    expect(catalog[0]?.request?.pageSize).toBe(80);
  });
  it("adapts only SQLite tag suggestions and never creates synthetic candidates", () => {
    expect(catalogSuggestion({ namespace: "female", name: "big balls", token: "female:big_balls", galleryCount: 4822, favorite: true })).toMatchObject({ type: "FEMALE", label: "big balls", favorite: true, galleryCount: 4822 });
  });

  it("keeps artist and group namespaces distinct in the suggestion UI", () => {
    expect(catalogSuggestion({ namespace: "artist", name: "mizuno tooru", token: "artist:mizuno_tooru", galleryCount: 142, favorite: false }))
      .toMatchObject({ type: "ARTIST", token: "artist:mizuno_tooru", label: "mizuno tooru" });
    expect(catalogSuggestion({ namespace: "group", name: "circle energy", token: "group:circle_energy", galleryCount: 76, favorite: true }))
      .toMatchObject({ type: "GROUP", token: "group:circle_energy", label: "circle energy", favorite: true });
  });
});
