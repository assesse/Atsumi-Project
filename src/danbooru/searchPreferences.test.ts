import { describe, expect, it } from "vitest";
import {
  buildDanbooruSearchQuery,
  danbooruLimitedTermCount,
  defaultDanbooruSearchFilters,
  sanitizeDanbooruSearchFilters,
  isDanbooruDate,
  validateDanbooruSearchDates,
  withoutRetiredRelationship,
  type DanbooruSearchFilters,
} from "./searchPreferences";

describe("Danbooru search preferences", () => {
  it.each([
    ["newest", ""], ["oldest", "order:id_asc"], ["score", "order:score"],
    ["favorites", "order:favcount"], ["resolution", "order:mpixels"],
    ["file_size", "order:filesize"], ["tag_count", "order:tagcount"],
    ["portrait", "order:portrait"], ["landscape", "order:landscape"],
  ] as const)("maps %s to the upstream server order without client-side sorting", (sort, metatag) => {
    expect(buildDanbooruSearchQuery("rating:g", { ...defaultDanbooruSearchFilters(), sort })).toBe(["rating:g", metatag].filter(Boolean).join(" "));
  });

  it("does not duplicate an explicitly typed order with a settings default", () => {
    expect(buildDanbooruSearchQuery("rating:g order:portrait", { ...defaultDanbooruSearchFilters(), sort: "score" })).toBe("rating:g order:portrait");
  });

  it("validates real dates, exact four-digit years and ordered inclusive ranges", () => {
    for (const invalid of ["201888-01-01", "0000-01-01", "2025-02-29", "2024-04-31", "2024-13-01", "2024--"]) expect(isDanbooruDate(invalid)).toBe(false);
    expect(isDanbooruDate("2024-02-29")).toBe(true);
    expect(validateDanbooruSearchDates({ dateFrom: "2018-10-08", dateTo: "2012-10-08" })).toContain("종료일");
    expect(validateDanbooruSearchDates({ dateFrom: "2018-10-08", dateTo: "2018-10-08" })).toBeNull();
    expect(buildDanbooruSearchQuery("", { ...defaultDanbooruSearchFilters(), dateFrom: "2018-10-08", dateTo: "2018-10-08" })).toBe("date:2018-10-08..2018-10-08");
  });
  it("builds free metatag filters separately from regular tags", () => {
    const filters: DanbooruSearchFilters = {
      ...defaultDanbooruSearchFilters(),
      ratings: ["g", "s"],
      fileTypes: ["jpg", "png"],
      dateFrom: "2026-08-01",
      dateTo: "2026-08-31",
      minimumScore: "20",
      minimumFavorites: "5",
      sort: "score" as const,
    };
    expect(buildDanbooruSearchQuery("sample_artist solo", filters)).toBe(
      "sample_artist solo rating:g,s filetype:jpg,png date:2026-08-01..2026-08-31 score:>=20 favcount:>=5 order:score",
    );
  });

  it("sanitizes unknown persisted values", () => {
    expect(sanitizeDanbooruSearchFilters({ ratings: ["g", "bad"], sort: "bad" }).ratings).toEqual(["g"]);
    expect(sanitizeDanbooruSearchFilters({ sort: "bad" }).sort).toBe("newest");
  });

  it("drops retired relationship preferences and generated restored-query filters without removing typed metatags", () => {
    const stored = { ...defaultDanbooruSearchFilters(), relationship: "has_children" };
    const filters = sanitizeDanbooruSearchFilters(stored);
    expect(filters).not.toHaveProperty("relationship");
    expect(buildDanbooruSearchQuery("solo", filters)).toBe("solo");
    expect(withoutRetiredRelationship("solo child:any", "solo", stored)).toBe("solo");
    expect(withoutRetiredRelationship("solo child:any", "solo child:any", stored)).toBe("solo child:any");
  });

  it("keeps numeric post IDs direct and counts only limited search terms", () => {
    expect(buildDanbooruSearchQuery("1234567", { ...defaultDanbooruSearchFilters(), sort: "score" })).toBe("1234567");
    expect(danbooruLimitedTermCount("artist:name solo rating:g date:2026-08-01 score:>=10")).toBe(2);
    expect(danbooruLimitedTermCount("artist:name solo order:score")).toBe(3);
  });
});
