import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadContentSource, saveContentSource } from "./sourcePreference";

describe("source preference", () => {
  beforeEach(() => window.localStorage.clear());
  afterEach(() => vi.restoreAllMocks());

  it("defaults to Hitomi and persists Danbooru", () => {
    expect(loadContentSource()).toBe("hitomi");
    saveContentSource("danbooru");
    expect(loadContentSource()).toBe("danbooru");
    expect(window.localStorage.getItem("atsumi.content-source.v1")).toBe("danbooru");
    saveContentSource("hitomi");
    expect(loadContentSource()).toBe("hitomi");
  });

  it.each(["unknown", "toString", "constructor", "__proto__"])("normalizes unknown stored value %s", (source) => {
    window.localStorage.setItem("atsumi.content-source.v1", source);
    expect(loadContentSource()).toBe("hitomi");
  });

  it("falls back to Hitomi when preference storage cannot be read", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("Storage blocked"); });
    expect(loadContentSource()).toBe("hitomi");
  });

  it("does not block source switching when preference storage cannot be written", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("Storage blocked"); });
    expect(() => saveContentSource("danbooru")).not.toThrow();
  });
});
