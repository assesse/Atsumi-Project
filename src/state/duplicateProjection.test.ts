import { describe, expect, it } from "vitest";
import type { DuplicateScanRun, DuplicateSnapshot } from "../api/contracts";
import {
  duplicateEventNeedsSnapshot,
  duplicateRunIsNewer,
  mergeHydratedDuplicateSnapshot,
  validDuplicateRun,
} from "./duplicateProjection";

const run = (revision: number, patch: Partial<DuplicateScanRun> = {}): DuplicateScanRun => ({
  runId: "run-race",
  revision,
  state: "running",
  totalArtifacts: 4,
  hashedArtifacts: revision,
  totalPairs: 6,
  comparedPairs: revision,
  candidatesFound: 0,
  startedAt: "2026-08-15T00:00:00.000Z",
  updatedAt: "2026-08-15T00:00:00.000Z",
  ...patch,
});

const snapshot = (scanRun: DuplicateScanRun): DuplicateSnapshot => ({
  profile: {
    profileVersion: 1,
    algorithmVersion: 1,
    dHashBits: 1024,
    pHashBits: 64,
    visualMatchThreshold: 0.9,
    lowInformationStdDevThreshold: 6,
  },
  run: scanRun,
  candidates: [],
});

describe("duplicate snapshot revision projection", () => {
  it("rejects a deferred old snapshot after a newer progress event", () => {
    const oldResponse = snapshot(run(1));
    const currentAfterEvent = snapshot(run(3));

    const merged = mergeHydratedDuplicateSnapshot(currentAfterEvent, oldResponse, currentAfterEvent.run);

    expect(merged).toBe(currentAfterEvent);
    expect(merged.run?.revision).toBe(3);
    expect(duplicateRunIsNewer(run(3), run(1))).toBe(false);
  });

  it("does not let a delayed pre-scan snapshot with no run clear a newly observed run", () => {
    const currentAfterStart = snapshot(run(0));
    const preScanResponse: DuplicateSnapshot = { ...currentAfterStart, run: undefined };

    const merged = mergeHydratedDuplicateSnapshot(currentAfterStart, preScanResponse, currentAfterStart.run);

    expect(merged).toBe(currentAfterStart);
    expect(merged.run).toMatchObject({ runId: "run-race", revision: 0, state: "running" });
  });

  it("rejects a delayed snapshot from the previous run after a new run starts", () => {
    const previous = snapshot(run(9, {
      runId: "previous-run",
      startedAt: "2026-08-14T23:00:00.000Z",
    }));
    const current = snapshot(run(0, {
      runId: "new-run",
      startedAt: "2026-08-15T00:00:00.000Z",
    }));

    const merged = mergeHydratedDuplicateSnapshot(current, previous, current.run);

    expect(merged).toBe(current);
    expect(merged.run?.runId).toBe("new-run");
  });

  it("hydrates candidates only for terminal or candidate-count events and ignores invalid payloads", () => {
    expect(duplicateEventNeedsSnapshot(run(1), run(2))).toBe(false);
    expect(duplicateEventNeedsSnapshot(run(1), run(2, { candidatesFound: 1 }))).toBe(true);
    expect(duplicateEventNeedsSnapshot(run(1), run(2, { state: "completed" }))).toBe(true);
    expect(validDuplicateRun(run(2))).toBe(true);
    expect(validDuplicateRun({ ...run(2), runId: "" })).toBe(false);
    expect(validDuplicateRun({ ...run(2), comparedPairs: -1 })).toBe(false);
    expect(duplicateRunIsNewer(
      run(0, { runId: "new", startedAt: "2026-08-15T00:00:00.000Z" }),
      run(9, { runId: "old", startedAt: "2026-08-14T00:00:00.000Z" }),
    )).toBe(false);
  });
});
