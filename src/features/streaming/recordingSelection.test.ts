import { describe, expect, it } from "vitest";
import { emptyRecordingSelection, retainRecordingSelection, selectRecording } from "./recordingSelection";

const ids = ["a", "b", "c", "d", "e"];
const plain = { ctrl: false, shift: false }, ctrl = { ctrl: true, shift: false }, shift = { ctrl: false, shift: true };
describe("recording selection's independent album-style gesture contract", () => {
  it("plain click selects one, a repeated click clears it, and another card replaces it", () => {
    const first = selectRecording(emptyRecordingSelection(), "b", ids, plain);
    expect(first).toEqual({ ids: ["b"], anchorId: "b" });
    expect(selectRecording(first, "b", ids, plain)).toEqual(emptyRecordingSelection());
    expect(selectRecording(first, "d", ids, plain)).toEqual({ ids: ["d"], anchorId: "d" });
  });
  it("Ctrl adds/removes without losing the original plain-clicked card or toggle anchor", () => {
    const first = selectRecording(emptyRecordingSelection(), "a", ids, plain);
    const second = selectRecording(first, "c", ids, ctrl);
    expect(second).toEqual({ ids: ["a", "c"], anchorId: "c" });
    expect(selectRecording(second, "c", ids, ctrl)).toEqual({ ids: ["a"], anchorId: "c" });
    expect(selectRecording(second, "b", ids, plain)).toEqual({ ids: ["b"], anchorId: "b" });
  });
  it("Shift adds the anchored visible range in either direction without clearing other choices", () => {
    const state = { ids: ["a", "e"], anchorId: "e" };
    expect(new Set(selectRecording(state, "c", ids, shift).ids)).toEqual(new Set(["a", "c", "d", "e"]));
    const start = { ids: ["e", "b"], anchorId: "b" };
    expect(new Set(selectRecording(start, "c", ids, { ctrl: true, shift: true }).ids)).toEqual(new Set(["e", "b", "c"]));
    expect(selectRecording(emptyRecordingSelection(), "c", ids, shift)).toEqual({ ids: ["c"], anchorId: "c" });
  });
  it("prunes hidden or busy files and their anchors; never selects an ineligible target", () => {
    const state = { ids: ["a", "b", "e"], anchorId: "e" };
    expect(retainRecordingSelection(state, ["a", "c"])).toEqual({ ids: ["a"], anchorId: null });
    expect(selectRecording(state, "busy", ids, ctrl)).toBe(state);
    expect(retainRecordingSelection(state, ids)).toBe(state);
  });
});
