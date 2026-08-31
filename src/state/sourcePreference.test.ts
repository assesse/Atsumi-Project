import { beforeEach, describe, expect, it } from "vitest";
import { loadContentSource, saveContentSource } from "./sourcePreference";

describe("source preference", () => {
  beforeEach(() => window.localStorage.clear());

  it("defaults to Hitomi and persists Danbooru", () => {
    expect(loadContentSource()).toBe("hitomi");
    saveContentSource("danbooru");
    expect(loadContentSource()).toBe("danbooru");
  });

  it("normalizes unknown stored values", () => {
    window.localStorage.setItem("atsumi.content-source.v1", "unknown");
    expect(loadContentSource()).toBe("hitomi");
  });
});
