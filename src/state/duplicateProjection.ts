import type { DuplicateScanRun, DuplicateSnapshot } from "../api/contracts";

export const validDuplicateRun = (run: DuplicateScanRun): boolean =>
  typeof run.runId === "string"
  && run.runId.length > 0
  && Number.isInteger(run.revision)
  && run.revision >= 0
  && [
    run.totalArtifacts,
    run.hashedArtifacts,
    run.totalPairs,
    run.comparedPairs,
    run.candidatesFound,
  ].every((value) => Number.isInteger(value) && value >= 0)
  && run.hashedArtifacts <= run.totalArtifacts
  && run.comparedPairs <= run.totalPairs
  && typeof run.startedAt === "string"
  && run.startedAt.length > 0
  && typeof run.updatedAt === "string"
  && run.updatedAt.length > 0
  && ["running", "completed", "failed", "cancelled"].includes(run.state);

export const duplicateRunIsNewer = (
  current: DuplicateScanRun | undefined,
  incoming: DuplicateScanRun,
): boolean => !current
  || (current.runId === incoming.runId
    ? incoming.revision > current.revision
    : incoming.startedAt > current.startedAt);

export const duplicateEventNeedsSnapshot = (
  previous: DuplicateScanRun | undefined,
  incoming: DuplicateScanRun,
): boolean => incoming.state !== "running"
  || (previous?.candidatesFound ?? 0) < incoming.candidatesFound;

export const mergeHydratedDuplicateSnapshot = (
  current: DuplicateSnapshot | null,
  incoming: DuplicateSnapshot,
  latestRun: DuplicateScanRun | undefined,
): DuplicateSnapshot => {
  const incomingRun = incoming.run;
  const stale = latestRun && (
    !incomingRun
    || (incomingRun.runId === latestRun.runId && incomingRun.revision < latestRun.revision)
    || (incomingRun.runId !== latestRun.runId && incomingRun.startedAt <= latestRun.startedAt)
  );
  if (!stale) return incoming;
  return current ?? { ...incoming, run: latestRun };
};
