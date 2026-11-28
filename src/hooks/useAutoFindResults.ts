import { useCallback, useEffect, useRef, useState } from "react";
import type { BackendClient } from "../api/backend";
import type { AutoFindCandidate, AutoFindRun, AutoFindSnapshot } from "../api/contracts";

type Load = "snapshot" | "changes";
const emptySnapshot: AutoFindSnapshot = { candidates: [], cutoffEvidence: [], truncations: [] };

function olderThan(run: AutoFindRun, latest?: AutoFindRun) {
  if (!latest) return false;
  if (latest.runId === run.runId) return run.revision < latest.revision;
  return run.startedAt < latest.startedAt || (run.startedAt === latest.startedAt && run.runId < latest.runId);
}

export function useAutoFindResults(
  backend: BackendClient, visible: boolean, mergeGalleries: (items: AutoFindCandidate[]) => void,
) {
  const [snapshot, setSnapshot] = useState(emptySnapshot);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const projection = useRef(emptySnapshot);
  const latestRun = useRef<AutoFindRun | undefined>(undefined);
  const needsSnapshot = useRef(true);
  const visibleRef = useRef(visible);
  const lifetime = useRef(0);
  const pending = useRef<Load | null>(null);
  const worker = useRef<Promise<void> | null>(null);

  const replaceSnapshot = useCallback((data: AutoFindSnapshot) => {
    const latest = latestRun.current;
    if (latest && data.run?.runId !== latest.runId && (!data.run || olderThan(data.run, latest))) return;
    const run = data.run && olderThan(data.run, latest) ? latest : data.run;
    latestRun.current = run;
    const next = { ...data, run };
    projection.current = next;
    needsSnapshot.current = data.run?.state !== run?.state;
    mergeGalleries(data.candidates);
    setSnapshot(next);
  }, [mergeGalleries]);

  const load = useCallback((mode: Load) => {
    if (mode === "snapshot" || pending.current === null) pending.current = mode;
    if (worker.current) return worker.current;
    const generation = lifetime.current;
    const drain = async () => {
      while (pending.current && lifetime.current === generation) {
        const request = pending.current;
        pending.current = null;
        const base = projection.current;
        try {
          if (request === "snapshot") {
            const result = await backend.autoFindSnapshot();
            if (lifetime.current !== generation) return;
            // An exclusion, restore or another run may have replaced this view.
            if (base !== projection.current) continue;
            if (!result.ok) { setError(result.error.message); continue; }
            replaceSnapshot(result.data);
          } else {
            const run = latestRun.current;
            if (!visibleRef.current || needsSnapshot.current || run?.state !== "running" || base.run?.runId !== run.runId) continue;
            const result = await backend.autoFindCandidateChanges(run.runId, base.matchCursor ?? 0);
            if (lifetime.current !== generation) return;
            if (base !== projection.current || latestRun.current?.runId !== run.runId || !visibleRef.current || pending.current === "snapshot") continue;
            if (!result.ok) { setError(result.error.message); continue; }
            const { matchCursor, changedIds, candidates, more, truncations } = result.data;
            let list = base.candidates;
            if (changedIds.length) {
              const updated = new Set(changedIds);
              list = base.candidates.filter(candidate => !updated.has(candidate.id));
              list.push(...candidates);
              list.sort((a, b) => b.publishedRank - a.publishedRank || b.id - a.id);
              mergeGalleries(candidates);
            }
            if (list !== base.candidates || truncations.length !== base.truncations.length) {
              const next = { ...base, run: latestRun.current, matchCursor, candidates: list, truncations };
              projection.current = next;
              setSnapshot(next);
            }
            if (more && pending.current === null) pending.current = "changes";
          }
          setError(null);
        } catch {
          if (lifetime.current === generation) setError("자동 탐색 결과를 불러오지 못했습니다.");
        }
      }
      if (lifetime.current === generation) {
        worker.current = null;
        setLoading(false);
      }
    };
    // Coalesce notifications already queued in this turn.
    const task = Promise.resolve().then(drain);
    worker.current = task;
    return task;
  }, [backend, mergeGalleries, replaceSnapshot]);

  const acceptRun = useCallback((run: AutoFindRun) => {
    const previous = latestRun.current;
    if (olderThan(run, previous) || (previous?.runId === run.runId && previous.revision === run.revision)) return;
    latestRun.current = run;
    if (previous?.runId !== run.runId) {
      projection.current = { ...emptySnapshot, run };
      if (visibleRef.current) {
        const fresh = projection.current;
        // A run which retained pending candidates already has rows. Keep them visible during its first snapshot.
        setSnapshot(current => run.candidatesFound > 0 && current.candidates.length ? { ...current, run } : fresh);
      }
      needsSnapshot.current = true;
    } else if (visibleRef.current) {
      setSnapshot(current => ({ ...current, run }));
    }
    if (run.state !== "running") needsSnapshot.current = true;
    if (visibleRef.current) {
      const full = previous?.runId !== run.runId || run.state !== "running" || (needsSnapshot.current && !worker.current);
      void load(full ? "snapshot" : "changes");
    }
  }, [load]);

  const loadSnapshot = useCallback((showLoading = false) => {
    if (showLoading) setLoading(true);
    return load("snapshot");
  }, [load]);

  useEffect(() => {
    visibleRef.current = visible;
    const run = latestRun.current;
    if (visible && run) {
      setSnapshot(current => current.run === run ? current : { ...projection.current, run });
      if (needsSnapshot.current) void load("snapshot");
      else if (run.state === "running") void load("changes");
    }
  }, [visible, load]);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void backend.on("auto-find:changed", run => { if (!disposed) acceptRun(run); })
      .then(release => { if (disposed) release(); else unsubscribe = release; })
      .catch(() => { if (!disposed) setError("자동 탐색 진행 상태를 불러오지 못했습니다."); });
    return () => {
      disposed = true;
      unsubscribe?.();
      lifetime.current++;
      pending.current = null;
      worker.current = null;
    };
  }, [backend, acceptRun]);

  return { snapshot, error, setError, loading, loadSnapshot, acceptRun, replaceSnapshot };
}
