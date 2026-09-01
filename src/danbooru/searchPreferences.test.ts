import { describe, expect, it } from "vitest";
import {
  buildDanbooruSearchQuery,
  danbooruLimitedTermCount,
  defaultDanbooruSearchFilters,
  sanitizeDanbooruSearchFilters,
  type DanbooruSearchFilters,
} from "./searchPreferences";

describe("Danbooru search preferences", () => {
  it("builds free metatag filters separately from regular tags", () => {
    const filters: DanbooruSearchFilters = {
      ...defaultDanbooruSearchFilters(),
      ratings: ["g", "s"],
      fileTypes: ["jpg", "png"],
      dateFrom: "2026-08-01",
      dateTo: "2026-08-31",
      minimumScore: "20",
      minimumFavorites: "5",
      relationship: "has_children" as const,
      sort: "score" as const,
    };
    expect(buildDanbooruSearchQuery("sample_artist solo", filters)).toBe(
      "sample_artist solo rating:g,s filetype:jpg,png date:2026-08-01..2026-08-31 score:>=20 favcount:>=5 child:any order:score",
    );
  });

  it("sanitizes unknown persisted values", () => {
    expect(sanitizeDanbooruSearchFilters({ ratings: ["g", "bad"], sort: "bad" }).ratings).toEqual(["g"]);
    expect(sanitizeDanbooruSearchFilters({ sort: "bad" }).sort).toBe("newest");
  });

  it("keeps numeric post IDs direct and counts only limited search terms", () => {
    expect(buildDanbooruSearchQuery("1234567", { ...defaultDanbooruSearchFilters(), sort: "score" })).toBe("1234567");
    expect(danbooruLimitedTermCount("artist:name solo rating:g date:2026-08-01 score:>=10")).toBe(2);
    expect(danbooruLimitedTermCount("artist:name solo order:score")).toBe(3);
  });
});
