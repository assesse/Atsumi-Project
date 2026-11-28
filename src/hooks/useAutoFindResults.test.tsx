import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../api/backend";
import type { AutoFindCandidate, AutoFindRun } from "../api/contracts";
import { galleryId } from "../core/types";
import { useAutoFindResults } from "./useAutoFindResults";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("reads only candidate changes while Auto Find runs and catches up after returning", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const run: AutoFindRun = {
    runId: "test-run", revision: 1, state: "running", totalFavorites: 8, completedFavorites: 0,
    candidatesFound: 0, startedAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z",
    historyMode: "include_all_history",
  };
  const candidate: AutoFindCandidate = { id: galleryId(10), title: "a", artist: "artist", pages: 1,
    language: "korean", tags: [], series: [], characters: [], publishedRank: 1, popularity: 0,
    thumbnailWidth: 320, thumbnailHeight: 480, runId: run.runId,
    matchedFavorite: { namespace: "artist", value: "artist" }, discoveredAt: run.startedAt };
  const full = vi.spyOn(backend, "autoFindSnapshot").mockResolvedValue({ ok: true,
    data: { run, candidates: [], matchCursor: 7, cutoffEvidence: [], truncations: [] } });
  const delta = vi.spyOn(backend, "autoFindCandidateChanges").mockResolvedValue({ ok: true,
    data: { matchCursor: 7, candidates: [], changedIds: [], more: false, truncations: [] } });
  const merge = vi.fn();
  let state!: ReturnType<typeof useAutoFindResults>;
  let renders = 0;
  function View({ visible }: { visible: boolean }) {
    renders++;
    state = useAutoFindResults(backend, visible, merge);
    return null;
  }
  const root = createRoot(document.createElement("div"));
  const show = (visible: boolean) => act(async () => { root.render(<View visible={visible} />); });
  const advance = (revision: number) => act(async () => {
    state.acceptRun({ ...run, revision, completedFavorites: revision - 1 });
  });
  try {
    await show(true);
    await act(async () => { await state.loadSnapshot(); });
    const unchanged = state.snapshot.candidates;
    await advance(2);
    await advance(3);
    expect(state.snapshot.candidates).toBe(unchanged);
    expect(state.snapshot.run?.completedFavorites).toBe(2);
    expect(delta).toHaveBeenCalledTimes(2);
    expect(full).toHaveBeenCalledTimes(1);

    await show(false);
    const hiddenRenders = renders;
    await advance(4);
    await advance(5);
    expect(renders).toBe(hiddenRenders);
    expect(delta).toHaveBeenCalledTimes(2);
    expect(full).toHaveBeenCalledTimes(1);
    delta.mockResolvedValueOnce({ ok: true, data: {
      matchCursor: 8, candidates: [candidate], changedIds: [candidate.id], more: true, truncations: [],
    } }).mockResolvedValueOnce({ ok: true, data: {
      matchCursor: 9, candidates: [], changedIds: [candidate.id], more: false, truncations: [],
    } });
    await show(true);
    expect(delta.mock.calls.slice(-2)).toEqual([[run.runId, 7], [run.runId, 8]]);
    expect(state.snapshot.candidates).toEqual([]);
    expect(state.snapshot.matchCursor).toBe(9);
    expect(full).toHaveBeenCalledTimes(1);

    const terminal = { ...run, state: "completed" as const, revision: 10, completedFavorites: 8 };
    full.mockResolvedValueOnce({ ok: true, data: {
      run: terminal, matchCursor: 9, candidates: [], cutoffEvidence: [], truncations: [],
    } });
    await act(async () => { state.acceptRun(terminal); });
    await advance(6);
    expect(state.snapshot.run).toEqual(terminal);
    expect(full).toHaveBeenCalledTimes(2);
  } finally {
    await act(async () => root.unmount());
  }
});
