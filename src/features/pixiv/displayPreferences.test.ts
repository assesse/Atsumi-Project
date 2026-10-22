import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { loadPixivSafeOnly, savePixivSafeOnly, PIXIV_SAFE_ONLY_KEY } from "./displayPreferences";

describe("Pixiv local display preference", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.restoreAllMocks());
  it("defaults to account scope, persists both choices and ignores malformed storage", () => {
    expect(loadPixivSafeOnly()).toBe(false);
    expect(savePixivSafeOnly(true)).toBe(true);
    expect(loadPixivSafeOnly()).toBe(true);
    expect(savePixivSafeOnly(false)).toBe(true);
    expect(loadPixivSafeOnly()).toBe(false);
    localStorage.setItem(PIXIV_SAFE_ONLY_KEY, "invalid");
    expect(loadPixivSafeOnly()).toBe(false);
  });
  it("retains the last committed value when persistence fails", () => {
    savePixivSafeOnly(true);
    const dispatch = vi.spyOn(window, "dispatchEvent");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
    expect(savePixivSafeOnly(false)).toBe(false);
    expect(loadPixivSafeOnly()).toBe(true);
    expect(dispatch).not.toHaveBeenCalled();
  });
  it("does not touch another platform's settings or account data", () => {
    localStorage.setItem("atsumi.pinterest.preview-width.v1", "280");
    savePixivSafeOnly(true);
    expect(Object.keys(localStorage).sort()).toEqual(["atsumi.pinterest.preview-width.v1", PIXIV_SAFE_ONLY_KEY]);
    expect(localStorage.getItem("atsumi.pinterest.preview-width.v1")).toBe("280");
  });
});
