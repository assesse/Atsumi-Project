import { describe, expect, it } from "vitest";
import { galleryId } from "../core/types";
import { initialUiState, uiReducer } from "./uiState";

const ids = [galleryId(1), galleryId(2), galleryId(3), galleryId(4)];

describe("uiReducer selection", () => {
  it("clears the sole selection and its anchor when the same card is plain-clicked again", () => {
    const selected = uiReducer(initialUiState, {
      type: "selection.click",
      id: ids[1]!,
      visibleIds: ids,
      ctrl: false,
      shift: false,
    });
    expect([...selected.selection.ids]).toEqual([ids[1]]);
    expect(selected.selection.anchorId).toBe(ids[1]);

    const reselected = uiReducer(selected, {
      type: "selection.click",
      id: ids[1]!,
      visibleIds: ids,
      ctrl: false,
      shift: false,
    });
    expect([...reselected.selection.ids]).toEqual([]);
    expect(reselected.selection.anchorId).toBeNull();
  });

  it("enters multiple selection only after Control adds a second card and keeps the existing toggle anchor contract", () => {
    const selected = uiReducer(initialUiState, {
      type: "selection.click",
      id: ids[0]!,
      visibleIds: ids,
      ctrl: false,
      shift: false,
    });
    const multiple = uiReducer(selected, {
      type: "selection.click",
      id: ids[1]!,
      visibleIds: ids,
      ctrl: true,
      shift: false,
    });
    expect([...multiple.selection.ids]).toEqual([ids[0], ids[1]]);
    expect(multiple.selection.anchorId).toBe(ids[1]);

    const oneRemaining = uiReducer(multiple, {
      type: "selection.click",
      id: ids[1]!,
      visibleIds: ids,
      ctrl: true,
      shift: false,
    });
    expect([...oneRemaining.selection.ids]).toEqual([ids[0]]);
    // Toggling is intentionally anchored to the action target even when that
    // target was removed; projection/retain clears a stale anchor as before.
    expect(oneRemaining.selection.anchorId).toBe(ids[1]);
  });

  it("replaces a multiple selection with the plain-clicked card", () => {
    const first = uiReducer(initialUiState, {
      type: "selection.click",
      id: ids[0]!,
      visibleIds: ids,
      ctrl: false,
      shift: false,
    });
    const multiple = uiReducer(first, {
      type: "selection.click",
      id: ids[2]!,
      visibleIds: ids,
      ctrl: true,
      shift: false,
    });
    const replaced = uiReducer(multiple, {
      type: "selection.click",
      id: ids[1]!,
      visibleIds: ids,
      ctrl: false,
      shift: false,
    });

    expect([...replaced.selection.ids]).toEqual([ids[1]]);
    expect(replaced.selection.anchorId).toBe(ids[1]);
  });

  it("toggles with control and adds an anchored range with shift", () => {
    const anchored = uiReducer(initialUiState, {
      type: "selection.click",
      id: ids[0]!,
      visibleIds: ids,
      ctrl: true,
      shift: false,
    });
    const ranged = uiReducer(anchored, {
      type: "selection.click",
      id: ids[2]!,
      visibleIds: ids,
      ctrl: false,
      shift: true,
    });
    expect([...ranged.selection.ids]).toEqual(ids.slice(0, 3));
  });

  it("creates a keyboard range from the focused card when no click anchor exists", () => {
    const ranged = uiReducer(initialUiState, {
      type: "selection.range",
      anchorId: ids[1]!,
      id: ids[3]!,
      visibleIds: ids,
    });

    expect([...ranged.selection.ids]).toEqual(ids.slice(1));
    expect(ranged.selection.anchorId).toBe(ids[1]);
  });

  it("drops hidden selections and a stale anchor when the visible projection changes", () => {
    const selected = uiReducer(initialUiState, {
      type: "selection.click",
      id: ids[0]!,
      visibleIds: ids,
      ctrl: false,
      shift: false,
    });
    const anchoredElsewhere = uiReducer(selected, {
      type: "selection.click",
      id: ids[1]!,
      visibleIds: ids,
      ctrl: true,
      shift: false,
    });
    const deselectedAnchor = uiReducer(anchoredElsewhere, {
      type: "selection.click",
      id: ids[1]!,
      visibleIds: ids,
      ctrl: true,
      shift: false,
    });

    const retained = uiReducer(deselectedAnchor, { type: "selection.retain", ids: [ids[0]!, ids[2]!] });
    expect([...retained.selection.ids]).toEqual([ids[0]]);
    expect(retained.selection.anchorId).toBeNull();
  });

  it("restores a parked context selection and rejects an anchor outside it", () => {
    const restored = uiReducer(initialUiState, {
      type: "selection.restore",
      ids: [ids[0]!, ids[2]!],
      anchorId: ids[2]!,
    });
    expect([...restored.selection.ids]).toEqual([ids[0], ids[2]]);
    expect(restored.selection.anchorId).toBe(ids[2]);

    const staleAnchor = uiReducer(restored, {
      type: "selection.restore",
      ids: [ids[0]!],
      anchorId: ids[3]!,
    });
    expect([...staleAnchor.selection.ids]).toEqual([ids[0]]);
    expect(staleAnchor.selection.anchorId).toBeNull();
  });
});

describe("uiReducer gallery grouping", () => {
  it("supports flat, daily, and artist projections independently in both library views", () => {
    const autoFindFlat = uiReducer(initialUiState, {
      type: "grouping.set",
      view: "auto-find",
      grouping: "all",
    });
    expect(autoFindFlat.grouping).toEqual({ "auto-find": "all", downloads: "all" });

    const downloadsArtist = uiReducer(autoFindFlat, {
      type: "grouping.set",
      view: "downloads",
      grouping: "artist",
    });
    expect(downloadsArtist.grouping).toEqual({ "auto-find": "all", downloads: "artist" });
  });
});

describe("uiReducer detail tabs", () => {
  it("inserts a child immediately after its parent and deduplicates tabs", () => {
    const first = uiReducer(initialUiState, { type: "detail.open", id: ids[0]! });
    const second = uiReducer(first, { type: "detail.open", id: ids[2]! });
    const child = uiReducer(second, { type: "detail.open", id: ids[1]!, parentId: ids[0]! });
    const duplicate = uiReducer(child, { type: "detail.open", id: ids[0]! });

    expect(duplicate.detail.tabs).toEqual([ids[0], ids[1], ids[2]]);
    expect(duplicate.detail.activeId).toBe(ids[0]);
  });
});
