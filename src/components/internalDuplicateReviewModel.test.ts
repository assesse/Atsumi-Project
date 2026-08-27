import { describe, expect, it } from "vitest";
import type { InternalDuplicateGroup } from "../api/contracts";
import { galleryId } from "../core/types";
import {
  buildInternalRemovalSelections,
  buildInternalReviewBlocks,
  selectionsMatchPlan,
} from "./internalDuplicateReviewModel";

const rows = (missingC = false): InternalDuplicateGroup[] => Array.from({ length: 5 }, (_, row) => ({
  groupId: `row-${row + 1}`,
  blockId: "block-a3",
  sequenceIndex: row,
  revision: 3,
  entryId: "entry-1",
  galleryId: galleryId(1),
  relation: "translation_visual" as const,
  confidence: .92,
  recommendedKeepSourcePage: row + 1,
  pages: [0, 1, 2, 3]
    .filter((track) => !(missingC && track === 2 && row === 2))
    .map((track) => ({
      sourcePage: track * 5 + row + 1,
      exactSha256: track === 0,
      visualSimilarity: track === 0 ? 1 : .92,
      detailHashDistance: track === 0 ? 0 : 12,
      lowInformation: false,
      editionTrackId: `block-a3-t${track}`,
      editionTrackOrdinal: track,
    })),
  resolved: false,
  createdAt: "2026-08-20T00:00:00.000Z",
  updatedAt: "2026-08-20T00:00:00.000Z",
}));

describe("internalDuplicateReviewModel", () => {
  it("builds four deterministic edition tracks and turns set B into row selections", () => {
    const blocks = buildInternalReviewBlocks(rows());
    expect(blocks).toHaveLength(1);
    expect(blocks[0]).toMatchObject({ edition: true, tracks: [
      { label: "세트 A", pages: [1, 2, 3, 4, 5], coveredRows: 5 },
      { label: "세트 B", pages: [6, 7, 8, 9, 10], coveredRows: 5 },
      { label: "세트 C", pages: [11, 12, 13, 14, 15], coveredRows: 5 },
      { label: "세트 D", pages: [16, 17, 18, 19, 20], coveredRows: 5 },
    ] });
    const selections = buildInternalRemovalSelections(blocks, { "block-a3": ["block-a3-t1"] }, {});
    expect(selections).toHaveLength(5);
    expect(selections.map((selection) => selection.keepSourcePage)).toEqual([6, 7, 8, 9, 10]);
    expect(selections[0]?.removeSourcePages).toEqual([1, 11, 16]);
    expect(selections.reduce((sum, selection) => sum + selection.removeSourcePages.length, 0)).toBe(15);
  });

  it("preserves a missing scene instead of auto-selecting another set", () => {
    const blocks = buildInternalReviewBlocks(rows(true));
    expect(blocks[0]?.tracks[2]).toMatchObject({ coveredRows: 4, missingRows: 1 });
    const selections = buildInternalRemovalSelections(blocks, { "block-a3": ["block-a3-t2"] }, {});
    expect(selections).toHaveLength(4);
    expect(selections.map((selection) => selection.groupId)).not.toContain("row-3");
  });

  it("keeps multiple selected edition tracks and removes only unselected counterparts", () => {
    const blocks = buildInternalReviewBlocks(rows());
    const selections = buildInternalRemovalSelections(blocks, {
      "block-a3": ["block-a3-t0", "block-a3-t2"],
    }, {});

    expect(selections).toHaveLength(5);
    expect(selections.map((selection) => selection.keepSourcePage)).toEqual([1, 2, 3, 4, 5]);
    expect(selections[0]?.removeSourcePages).toEqual([6, 16]);
    expect(selections.reduce((sum, selection) => sum + selection.removeSourcePages.length, 0)).toBe(10);
  });

  it("preserves only rows where every selected edition is missing", () => {
    const source = rows(true).map((group, index) => index === 2 ? {
      ...group,
      pages: group.pages.filter((page) => page.editionTrackOrdinal !== 0),
    } : group);
    const blocks = buildInternalReviewBlocks(source);
    const selections = buildInternalRemovalSelections(blocks, {
      "block-a3": ["block-a3-t0", "block-a3-t2"],
    }, {});

    expect(selections.map((selection) => selection.groupId)).not.toContain("row-3");
    expect(selections).toHaveLength(4);
  });

  it("keeps legacy and standalone rows on the existing individual-page path", () => {
    const legacy = rows().slice(0, 1).map((group) => ({
      ...group,
      blockId: "legacy",
      pages: group.pages.map(({ editionTrackId: _id, editionTrackOrdinal: _ordinal, ...page }) => page),
    }));
    const blocks = buildInternalReviewBlocks(legacy);
    expect(blocks[0]?.edition).toBe(false);
    expect(buildInternalRemovalSelections(blocks, {}, { "row-1": 6 })).toEqual([{
      groupId: "row-1", expectedRevision: 3, keepSourcePage: 6, removeSourcePages: [1, 11, 16],
    }]);
  });

  it("invalidates a stale plan when the selected track changes", () => {
    const blocks = buildInternalReviewBlocks(rows());
    const selected = buildInternalRemovalSelections(blocks, { "block-a3": ["block-a3-t0"] }, {});
    const plan = { planId: "plan", entryId: "entry-1", selections: selected, filesToQuarantine: 15, bytesToQuarantine: 1, expiresAt: "later" };
    expect(selectionsMatchPlan(selected, plan)).toBe(true);
    expect(selectionsMatchPlan(buildInternalRemovalSelections(blocks, { "block-a3": ["block-a3-t0", "block-a3-t2"] }, {}), plan)).toBe(false);
  });
});
