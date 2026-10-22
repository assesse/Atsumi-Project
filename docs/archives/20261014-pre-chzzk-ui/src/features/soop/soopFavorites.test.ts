import { beforeEach, describe, expect, it } from "vitest";
import { readSoopFavorites, SOOP_FAVORITES_KEY, writeSoopFavorites } from "./soopFavorites";

beforeEach(() => localStorage.clear());
describe("SOOP local favorites", () => {
  it("returns an empty list for missing, malformed or foreign data", () => {
    expect(readSoopFavorites()).toEqual([]);
    localStorage.setItem(SOOP_FAVORITES_KEY, "{"); expect(readSoopFavorites()).toEqual([]);
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify({ name: "wrong" })); expect(readSoopFavorites()).toEqual([]);
  });
  it("deduplicates and normalizes valid channel identities", () => {
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify([{ channelId: "AFLOL", name: "LCK" }, { channelId: "aflol", name: "again" }, { channelId: "https://evil.test", name: "bad" }, null]));
    expect(readSoopFavorites()).toEqual([{ channelId: "aflol", name: "LCK" }]);
  });
  it("stores identity only and excludes image/session fields", () => {
    const channel = { channelId: "aflol", name: "LCK", image: "data:image/png;base64,AAAA", token: "not-saved" };
    expect(writeSoopFavorites([channel])).toBe(true);
    expect(JSON.parse(localStorage.getItem(SOOP_FAVORITES_KEY)!)).toEqual([{ channelId: "aflol", name: "LCK" }]);
  });
  it("caps persisted favorite count", () => {
    expect(writeSoopFavorites(Array.from({ length: 120 }, (_, index) => ({ channelId: `channel${index}`, name: "Channel" })))).toBe(true);
    expect(readSoopFavorites()).toHaveLength(100);
  });
});
