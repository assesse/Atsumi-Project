import { describe, expect, it } from "vitest";
import type { DownloadChangedEvent } from "../api/contracts";
import { galleryId } from "../core/types";
import { galleryMap, mockGalleries } from "../data/mockGalleries";
import { applyDownloadChanged } from "./downloadProjection";

const event = (revision: number, progress: number): DownloadChangedEvent => ({
  entryId: "entry-4051027",
  galleryId: 4051027,
  revision,
  state: "downloading",
  progress,
  attempt: 2,
  errorCode: "SOURCE_TIMEOUT",
  errorMessage: "The source timed out",
});

describe("download event projection", () => {
  it("patches only the target gallery and records its revision", () => {
    const before = galleryMap();
    const targetId = galleryId(4051027);
    const untouchedId = galleryId(4051038);
    const result = applyDownloadChanged(before, event(2, 63));

    expect(result.applied).toBe(true);
    expect(result.galleries.get(targetId)?.download?.progress).toBe(63);
    expect(result.galleries.get(targetId)?.download).toMatchObject({
      revision: 2,
      attempt: 2,
      errorCode: "SOURCE_TIMEOUT",
      errorMessage: "The source timed out",
    });
    expect(result.galleries.get(untouchedId)).toBe(before.get(untouchedId));
    expect(result.galleries.get(targetId)?.download?.revision).toBe(2);
  });

  it("ignores an older event so it cannot overwrite the latest state", () => {
    const latest = applyDownloadChanged(galleryMap(mockGalleries), event(4, 88));
    const stale = applyDownloadChanged(latest.galleries, event(3, 12));

    expect(stale.applied).toBe(false);
    expect(stale.galleries).toBe(latest.galleries);
    expect(stale.galleries.get(galleryId(4051027))?.download?.progress).toBe(88);
  });

  it("projects the typed overlap review target from a review-required event", () => {
    const result = applyDownloadChanged(galleryMap(), {
      ...event(8, 100),
      state: "review_required",
      reviewKind: "gallery_duplicate",
      reviewId: "overlap-review-1",
    });
    expect(result.galleries.get(galleryId(4051027))?.download).toMatchObject({
      state: "review_required",
      reviewKind: "gallery_duplicate",
      reviewId: "overlap-review-1",
    });
  });
});
