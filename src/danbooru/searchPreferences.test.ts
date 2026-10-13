import { describe, expect, it } from "vitest";
import {
  buildDanbooruSearchQuery,
  danbooruLimitedTermCount,
  defaultDanbooruSearchFilters,
  sanitizeDanbooruSearchFilters,
  isDanbooruDate,
  validateDanbooruSearchDates,
  withoutRetiredRelationship,
  DANBOORU_SORTS,
  type DanbooruSearchFilters,
} from "./searchPreferences";

describe("Danbooru search preferences", () => {
  it.each([
    ["newest", ""], ["oldest", "order:id_asc"], ["popular", "order:rank"], ["file_size", "order:filesize"],
    ["popular_week", "age:<7days order:score"], ["popular_month", "age:<30days order:score"],
  ] as const)("maps %s to the upstream server order without client-side sorting", (sort, metatag) => {
    expect(buildDanbooruSearchQuery("rating:g", { ...defaultDanbooruSearchFilters(), sort })).toBe(["rating:g", metatag].filter(Boolean).join(" "));
  });

  it("removes orders that the anonymous server times out and migrates their saved preferences", () => {
    expect(DANBOORU_SORTS.map(item => item.value)).toEqual(["newest", "oldest", "popular", "popular_week", "popular_month", "file_size"]);
    for (const sort of ["score", "favorites", "resolution", "tag_count", "portrait", "landscape"]) {
      expect(sanitizeDanbooruSearchFilters({ sort }).sort).toBe("newest");
    }
  });

  it("does not duplicate an explicitly typed order with a settings default", () => {
    expect(buildDanbooruSearchQuery("rating:g order:portrait", { ...defaultDanbooruSearchFilters(), sort: "score" })).toBe("rating:g order:portrait");
  });

  it.each(["popular_week", "popular_month"] as const)("preserves %s preferences and does not impose its age on an explicit order", sort => {
    expect(sanitizeDanbooruSearchFilters({ sort }).sort).toBe(sort);
    expect(buildDanbooruSearchQuery("rating:g order:id_asc", { ...defaultDanbooruSearchFilters(), sort })).toBe("rating:g order:id_asc");
    expect(buildDanbooruSearchQuery("1234567", { ...defaultDanbooruSearchFilters(), sort })).toBe("1234567");
    expect(danbooruLimitedTermCount(buildDanbooruSearchQuery("solo", { ...defaultDanbooruSearchFilters(), sort }))).toBe(2);
  });

  it("intersects a popularity period with explicit date, rating and score filters without silently clearing them", () => {
    const filters = { ...defaultDanbooruSearchFilters(), sort: "popular_week" as const, ratings: ["g" as const], dateFrom: "2026-10-08", dateTo: "2026-10-10", minimumScore: "20" };
    expect(buildDanbooruSearchQuery("solo", filters)).toBe("solo rating:g date:2026-10-08..2026-10-10 score:>=20 age:<7days order:score");
  });

  it("keeps native popularity and the user's explicit dates without silently widening either", () => {
    const filters = { ...defaultDanbooruSearchFilters(), sort: "popular" as const, dateFrom: "2020-01-01", dateTo: "2020-01-31" };
    expect(sanitizeDanbooruSearchFilters(filters).sort).toBe("popular");
    expect(buildDanbooruSearchQuery("rating:g", filters)).toBe("rating:g date:2020-01-01..2020-01-31 order:rank");
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
      sort: "file_size" as const,
    };
    expect(buildDanbooruSearchQuery("sample_artist solo", filters)).toBe(
      "sample_artist solo rating:g,s filetype:jpg,png date:2026-08-01..2026-08-31 score:>=20 favcount:>=5 order:filesize",
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
