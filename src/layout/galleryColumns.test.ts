import { describe, expect, it } from "vitest";
import { resolveGalleryColumns } from "./galleryColumns";

describe("responsive gallery columns", () => {
  it("treats the setting as a maximum, not a fixed column count", () => {
    expect(resolveGalleryColumns(900, 4, 220)).toBe(1);
    expect(resolveGalleryColumns(1_000, 4, 220)).toBe(2);
    expect(resolveGalleryColumns(1_700, 4, 220)).toBe(3);
    expect(resolveGalleryColumns(2_100, 4, 220)).toBe(4);
  });

  it("never exceeds the configured maximum", () => {
    expect(resolveGalleryColumns(2_100, 1, 220)).toBe(1);
    expect(resolveGalleryColumns(2_100, 2, 220)).toBe(2);
    expect(resolveGalleryColumns(2_100, 3, 220)).toBe(3);
  });

  it("reserves more room when previews are wider", () => {
    expect(resolveGalleryColumns(1_000, 4, 220)).toBe(2);
    expect(resolveGalleryColumns(1_000, 4, 360)).toBe(1);
  });
});
