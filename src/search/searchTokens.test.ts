import { describe, expect, it } from "vitest";
import { activeSearchToken, canonicalSearchToken, metadataSearchToken, normalizeTokenValue, replaceActiveSearchToken } from "./searchTokens";

describe("search tokens", () => {
  it("normalizes whitespace and duplicate underscores", () => {
    expect(normalizeTokenValue(" Story   Arc ")).toBe("story_arc");
    expect(normalizeTokenValue("full__color")).toBe("full_color");
    expect(normalizeTokenValue("sugoi\\_hi")).toBe("sugoi_hi");
  });

  it("separates display tag tokens from backend include values", () => {
    expect(metadataSearchToken("webtoon")).toEqual({ displayToken: "tag:webtoon", includeTag: "webtoon" });
    expect(metadataSearchToken("story arc")).toEqual({ displayToken: "tag:story_arc", includeTag: "story arc" });
    expect(metadataSearchToken("female:glasses")).toEqual({ displayToken: "female:glasses", includeTag: "female:glasses" });
    expect(metadataSearchToken("male:business suit")).toEqual({ displayToken: "male:business_suit", includeTag: "male:business suit" });
    expect(canonicalSearchToken(" Series:Rain Archives ")).toBe("series:rain_archives");
    expect(canonicalSearchToken(" Artist:Mizuno Tooru ")).toBe("artist:mizuno_tooru");
    expect(canonicalSearchToken(" Group:Circle  Energy ")).toBe("group:circle_energy");
  });

  it("replaces only the active token and preserves a negative prefix", () => {
    expect(replaceActiveSearchToken("artist:mizuno tag:full", 23, "tag:full_color")).toBe("artist:mizuno tag:full_color");
    expect(replaceActiveSearchToken("artist:a  -tag:web  group:b", 16, "tag:webtoon")).toBe("artist:a  -tag:webtoon  group:b");
    expect(activeSearchToken("a  tag:web  b", 7)).toEqual({ start: 3, end: 10, value: "tag:web" });
  });
});
