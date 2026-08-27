import { describe, expect, it } from "vitest";
import {
  DEFAULT_GALLERY_PREVIEW_WIDTH,
  GALLERY_PREVIEW_PRESETS,
  galleryPreviewPreset,
  normalizeGalleryPreviewWidth,
} from "./galleryPreviewPresets";

describe("gallery preview presets", () => {
  it("keeps the shared seven-width contract and 220px default", () => {
    expect(GALLERY_PREVIEW_PRESETS.map(({ width }) => width))
      .toEqual([160, 190, 220, 250, 280, 320, 360]);
    expect(DEFAULT_GALLERY_PREVIEW_WIDTH).toBe(220);
    expect(GALLERY_PREVIEW_PRESETS.map(({ maxTagRows }) => maxTagRows))
      .toEqual([2, 2, 3, 4, 5, 6, 7]);
  });

  it("normalizes legacy values with the frozen compatibility cutovers", () => {
    expect(normalizeGalleryPreviewWidth(Number.NaN)).toBe(220);
    expect(normalizeGalleryPreviewWidth(221)).toBe(220);
    expect(normalizeGalleryPreviewWidth(235)).toBe(220);
    expect(normalizeGalleryPreviewWidth(236)).toBe(250);
    expect(normalizeGalleryPreviewWidth(305)).toBe(280);
    expect(normalizeGalleryPreviewWidth(306)).toBe(320);
  });

  it("grows the type ramp and tag capacity monotonically", () => {
    for (let index = 1; index < GALLERY_PREVIEW_PRESETS.length; index += 1) {
      const previous = GALLERY_PREVIEW_PRESETS[index - 1]!;
      const current = GALLERY_PREVIEW_PRESETS[index]!;
      expect(current.titlePx).toBeGreaterThanOrEqual(previous.titlePx);
      expect(current.bodyPx).toBeGreaterThanOrEqual(previous.bodyPx);
      expect(current.tagPx).toBeGreaterThanOrEqual(previous.tagPx);
      expect(current.maxTagRows).toBeGreaterThanOrEqual(previous.maxTagRows);
    }
    expect(galleryPreviewPreset(250).key).toBe("comfortable");
  });
});
