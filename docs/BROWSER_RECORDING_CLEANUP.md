# Verified browser-recording fragment cleanup

New successful browser-recording merges automatically remove only the exact
`segment-000000000000.mp4` / `.webm` files listed in that recording's journal.
Previously completed recordings without a cleanup marker are not batch-cleaned.
No development test reads or deletes existing user recordings.

## Publication and recovery

1. Compare every input's codec parameters, actual duration and source cut points;
   fingerprint each input with streaming SHA-256 reads.
2. Stream-copy the complete set to a unique partial derivative, verify its codec
   parameters and duration, then decode all video and audio with FFmpeg. Decoder
   errors or mismatched decoded duration fail the merge and preserve originals.
3. Recheck the source hashes and journal generation. Durably write a bounded
   proof containing every source hash plus merged-media, timeline and journal
   hashes. Atomically publish the unique media/timeline names and persist
   `merge.status=complete` with `sourceCleanup.status=pending` in recording.json.
4. Outside the capture/store locks, validate that proof and the retained files.
   Pin Windows directory ancestors against replacement, then open each exact
   source with no-follow, deny-write/delete sharing, verify its hash and single
   hard-link count, and delete that verified handle. Never enumerate a directory
   to find candidates or recursively remove anything.
5. Persist cleanup `complete`, or `blocked` with the number already absent and a
   retryable UI explanation. A crash leaves the durable pending marker; missing
   already-cleaned sources are accepted and remaining sources are retried safely.

The merged media is never replaced or removed by cleanup. Archive replay already
opens that merged file, so active replay handles remain valid throughout cleanup.
An external original-file reader that does not share delete access blocks cleanup
until the reader closes. Source buttons are hidden once cleanup is marked;
merged playback and a cleanup-only retry action remain available.

Retained files include the recording metadata, source journal, merge timeline,
proof, chat/viewer logs, assets, init files, unjournaled/partial media and all
unrelated files. Historical segment counts/bytes remain recording history, not a
measurement of currently occupied disk space. Missing or changed outputs,
symlinks/reparse points, hard links, readonly/busy files, malformed manifests,
traversal or cancellation never authorize an unsafe deletion. Non-Windows builds
fail cleanup closed until equivalent handle-relative deletion is implemented.

## Cost and cancellation

Full decoding deliberately checks the middle of the recording too; container
metadata or a head/tail playback sample alone cannot justify irreversible source
deletion. This adds work proportional to recording length. FFmpeg uses one decoder
and filter thread, below-normal process priority and a maximum two-hour decode
deadline. The worker also runs below normal priority. A long/high-resolution
recording can take considerable extra time; a timeout preserves originals.
The worker checks cancellation during streaming 128-KiB hashes and tool polling,
kills/waits its own tool, and does not hold either store mutex for these long
operations. Already-completed cleanup is **not** media-hashed on app startup.

## Verification

Temporary-only Windows storage regressions cover successful cleanup/reopen,
existing replay handles, a crash after one deletion, failed final metadata write,
missing journal, same-length media/timeline/proof/source corruption, a busy
original reader, hard links, readonly files, traversal and cancellation. Existing
legacy-merge and failed-publication tests ensure unproven outputs preserve sources.
Explicit FFmpeg fixtures exercise fragmented AVC/AAC and one/two duration-less
WebM segments through real full decoding and cleanup, plus an intact-container
corrupt-middle decode rejection and malformed/cancelled merge rejection.

Commands (only with verified application-managed local media tools):

```powershell
cargo test --offline --lib browser_store
cargo test --offline --lib browser_merge
$env:ATSUMI_MERGE_TEST_TOOLS_DIR = '<verified-local-tools>\bin'
cargo test --offline --lib synthetic_ffmpeg -- --ignored
```

These synthetic results do not claim successful cleanup of a real user's live
recording; no existing recording is used as a test fixture.

2026-09-12 Windows verification: 32 store/cleanup tests passed; 9 merge unit tests
passed; all 4 explicit synthetic FFmpeg integration tests passed (1.67 seconds);
21 RecordingPlayback UI tests passed. The corrupt-middle fixture retained valid
container metadata but failed full decoding, as required. No live recording or
existing user file was deleted during development or verification.
