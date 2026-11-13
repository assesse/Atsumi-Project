import type { BrowserRecording } from "../../api/officialBrowser";
import { canRetryRecording, hasCompletedMerge } from "./RecordingPlayback";

export type RetryEntry = { id: string; before: string; request: "pending" | "requesting" | "accepted" | "rejected" | "skipped" | "unknown" };
export type RecordingRetryBatch = { version: 1; entries: RetryEntry[] };
export type RecoveryState = "requesting" | "waiting" | "running" | "complete" | "failed" | "skipped" | "checking";
const storageKey = "atsumi.recording-library.retry-batch.v1";

export function canRetryStoppedRecording(recording: BrowserRecording): boolean {
  return recording.status !== "recording" && recording.ending?.reason !== "checking" && canRetryRecording(recording);
}
export function recoveryRevision(recording: BrowserRecording): string {
  return JSON.stringify([recording.merge, recording.archive, recording.progressive, recording.mediaRemovedAt, recording.deletionPending]);
}
export function recoveryState(entry: RetryEntry, recording?: BrowserRecording): RecoveryState {
  if (entry.request === "rejected") return "failed";
  if (entry.request === "skipped") return "skipped";
  if (entry.request === "pending" || entry.request === "requesting") return "requesting";
  if (!recording || recording.summaryPending || recording.storageCheckPending || recording.mediaRemovedAt != null || recording.deletionPending) return "checking";
  if (recording.workProgress || recording.merge?.status === "merging" || recording.archive?.status === "copying") return "running";
  // A successful IPC response means queued, not completed. An unchanged error
  // from the pre-request snapshot cannot prove that this attempt failed.
  if (entry.request === "accepted" && recoveryRevision(recording) === entry.before) return "checking";
  if (recording.merge?.status === "failed" || recording.merge?.status === "blocked" || recording.archive?.status === "blocked"
    || recording.archive?.lastError || recording.progressive?.lastError || recording.merge?.sourceCleanup?.status === "blocked") return "failed";
  if (hasCompletedMerge(recording) && (!recording.archive || recording.archive.status === "complete") && recording.merge?.sourceCleanup?.status !== "pending") return "complete";
  if (recording.status === "recording" || entry.request === "unknown") return "checking";
  return "waiting";
}
export function summarizeRecovery(batch: RecordingRetryBatch, recordings: BrowserRecording[]) {
  const byId = new Map(recordings.map(recording => [recording.id, recording]));
  const counts: Record<RecoveryState, number> = { requesting: 0, waiting: 0, running: 0, complete: 0, failed: 0, skipped: 0, checking: 0 };
  let running: BrowserRecording | undefined;
  for (const entry of batch.entries) {
    const recording = byId.get(entry.id), state = recoveryState(entry, recording);
    counts[state]++;
    if (state === "running" && recording?.workProgress) running ??= recording;
  }
  return { counts, running, total: batch.entries.length, settled: counts.complete + counts.failed + counts.skipped };
}
export function readRetryBatch(scope = "chzzk"): RecordingRetryBatch | null {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(storageKey + "." + scope) ?? "null");
    if (!saved || typeof saved !== "object" || !("version" in saved) || saved.version !== 1 || !("entries" in saved) || !Array.isArray(saved.entries) || saved.entries.length > 4096) return null;
    if (!saved.entries.every(entry => entry && typeof entry.id === "string" && entry.id.length <= 128 && typeof entry.before === "string" && entry.before.length <= 8192 && ["pending", "requesting", "accepted", "rejected", "skipped", "unknown"].includes(entry.request))) return null;
    return { version: 1, entries: saved.entries.map(entry => ({ ...entry, request: !retryBatchSubmitting(scope) && ["pending", "requesting"].includes(entry.request) ? "unknown" : entry.request })) };
  } catch { return null; }
}
export function saveRetryBatch(batch: RecordingRetryBatch | null, scope = "chzzk") {
  try { if (batch) localStorage.setItem(storageKey + "." + scope, JSON.stringify(batch)); else localStorage.removeItem(storageKey + "." + scope); } catch { /* Display persistence must never block recovery. */ }
}

// Submission belongs to the app session, not a mounted library. Navigation
// detaches only the observer; it must not silently drop the rest of a batch.
const submissions = new Map<string, Promise<void>>();
const observers = new Map<string, Set<() => void>>();
export const retryBatchSubmitting = (scope: string) => submissions.has(scope);
export function subscribeRetryBatch(scope: string, listener: () => void) {
  const group = observers.get(scope) ?? new Set<() => void>();
  observers.set(scope, group); group.add(listener);
  return () => { group.delete(listener); if (!group.size) observers.delete(scope); };
}
export async function submitRetryBatch(scope: string, entries: RetryEntry[], submit: (id: string) => void | Promise<boolean | void | "skipped">) {
  if (submissions.has(scope) || !entries.length) return;
  let batch: RecordingRetryBatch = { version: 1, entries };
  const publish = () => { saveRetryBatch(batch, scope); observers.get(scope)?.forEach(listener => listener()); };
  const update = (id: string, request: RetryEntry['request']) => {
    batch = { ...batch, entries: batch.entries.map(entry => entry.id === id ? { ...entry, request } : entry) }; publish();
  };
  // Start in a microtask so the in-flight guard exists before observers run.
  const task = Promise.resolve().then(async () => {
    publish();
    for (const entry of entries) {
      update(entry.id, 'requesting');
      try { const result = await submit(entry.id); update(entry.id, result === 'skipped' ? 'skipped' : result === false ? 'rejected' : 'accepted'); }
      catch { update(entry.id, 'rejected'); }
    }
  }).finally(() => { submissions.delete(scope); publish(); });
  submissions.set(scope, task);
  await task;
}
