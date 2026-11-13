import { afterEach, describe, expect, it } from "vitest";
import type { BrowserRecording } from "../../api/officialBrowser";
import { canRetryStoppedRecording, readRetryBatch, recoveryRevision, recoveryState, saveRetryBatch, summarizeRecovery, submitRetryBatch, subscribeRetryBatch, retryBatchSubmitting, type RetryEntry } from "./recordingRecovery";
const item = (patch: Partial<BrowserRecording> = {}): BrowserRecording => ({ id: "a", channelId: "a", title: "fixture", startedAt: 1, updatedAt: 2, status: "interrupted", mimeType: "video/webm", outputDir: "fixture", segmentCount: 2, bytesWritten: 100, durationSeconds: 10, lastError: "old capture gap", segments: [], merge: { status: "failed", segmentCount: 2, updatedAt: 2, lastError: "no space" }, ...patch });
const complete = (): BrowserRecording => item({ merge: { status: "complete", segmentCount: 2, updatedAt: 3, file: "merged-" + "a".repeat(32) + ".webm", timelineFile: "merged-" + "a".repeat(32) + ".timeline.jsonl", bytes: 100, durationSeconds: 10 } });
const entry = (r: BrowserRecording): RetryEntry => ({ id: r.id, before: recoveryRevision(r), request: "accepted" });
afterEach(() => localStorage.clear());
describe("recording retry progress", () => {
  it("does not retry historical interruption once the saved media is complete", () => { expect(canRetryStoppedRecording(complete())).toBe(false); expect(canRetryStoppedRecording(item())).toBe(true); });
  it.each([{ status: "recording" }, { ending: { reason: "checking", trigger: "fixture", stoppedAt: 2, confirmedAt: null } }, { summaryPending: true }, { storageCheckPending: true }, { deletionPending: true }, { mediaRemovedAt: 3 }, { merge: { status: "queued", segmentCount: 2, updatedAt: 3 } }, { workProgress: { phase: "merging", completed: 1, total: 2, unit: "seconds", updatedAt: 3 } }] satisfies Partial<BrowserRecording>[])('excludes protected or already-queued work: %j', patch => expect(canRetryStoppedRecording(item(patch))).toBe(false));
  it("never calls accepted work completed or failed from the unchanged pre-request snapshot", () => { const r = item(); expect(recoveryState(entry(r), r)).toBe("checking"); expect(recoveryState(entry(r), complete())).toBe("complete"); expect(recoveryState(entry(r), item({ merge: { ...r.merge!, updatedAt: 4 } }))).toBe("failed"); });
  it("waits for archive and cleanup after final media, while trusting measured progress", () => { const r = item(), e = entry(r); expect(recoveryState(e, complete())).toBe("complete"); expect(recoveryState(e, { ...complete(), merge: { ...complete().merge!, sourceCleanup: { status: "pending", deletedSegments: 0, proofFile: "fixture", proofSha256: "fixture" } } })).toBe("waiting"); expect(recoveryState(e, item({ workProgress: { phase: "archiving", completed: 25, total: 100, unit: "bytes", updatedAt: 5 } }))).toBe("running"); });
  it("keeps missing metadata unknown rather than success", () => { const e = entry(item()); expect(recoveryState(e)).toBe("checking"); expect(recoveryState(e, item({ summaryPending: true }))).toBe("checking"); });
  it("persists tracking by platform without replaying requests or treating interrupted submission as accepted", () => { saveRetryBatch({ version: 1, entries: [{ ...entry(item()), request: "requesting" }] }); expect(readRetryBatch("soop")).toBeNull(); const batch = readRetryBatch()!; expect(batch.entries[0]!.request).toBe("unknown"); expect(summarizeRecovery(batch, []).counts.checking).toBe(1); });
});

it('keeps submitting after the library observer detaches, rejects duplicates, and persists each result', async () => {
  let resolve!: (value: boolean) => void;
  const calls: string[] = [], scope = 'navigation-fixture';
  const detach = subscribeRetryBatch(scope, () => {});
  const first = submitRetryBatch(scope, [entry(item()), { ...entry(item()), id: 'b' }], async id => {
    calls.push(id); if (id === 'a') return new Promise<boolean>(r => { resolve = r; }); return true;
  });
  await Promise.resolve(); detach();
  expect(retryBatchSubmitting(scope)).toBe(true);
  await submitRetryBatch(scope, [entry(item())], async () => { throw Error('duplicate'); });
  resolve(true); await first;
  expect(calls).toEqual(['a','b']); expect(retryBatchSubmitting(scope)).toBe(false);
  expect(readRetryBatch(scope)?.entries.map(e => e.request)).toEqual(['accepted','accepted']);
});
