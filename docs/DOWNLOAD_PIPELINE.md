# Hitomi download processing

## Worker boundaries

- Album coordinators load immutable page plans and commit page checkpoints in completion order. They hand fully received albums to the finalization queue and can start the next album without waiting for overlap comparison.
- A shared FIFO page pool serves every album, including a single large album. Its size follows the configured worker ceiling (1–8); there is no per-album multiplication of HTTP concurrency. The existing shared HTTP gate still applies learned limits, priorities, server waits and retries.
- A page task still performs source validation, optional WebP conversion and durable storage. This is not an unrestricted raw-response RAM queue or a completely separate HTTP/decoder spool.
- Two finalization workers perform integrity checks, artist locking, overlap comparison and manifest completion. Up to eight received albums wait in their queue; a full queue blocks coordinator handoff. Coordinators themselves are also bounded (at most eight).
- The pending page queue holds at most `8 × worker ceiling` small descriptors. Received image buffers remain in active page workers, not in queued descriptors or finalization tasks. Existing page-byte and image-work limits remain in force.

## Integrity and cancellation

- Storage layout, atomic page writes, file synchronization, SHA-256 checkpoints, manifests and overlap decision guards are unchanged.
- Pages can finish out of order; the coordinator serializes successful checkpoint/progress updates. It cancels siblings on the first failure and drains accepted tasks before releasing the job for retry.
- An entry-ownership fence also holds across finalization: a rapid user cancel/retry waits until the older attempt has drained, even though it has a different attempt ID. Waiting coordinators observe shutdown.
- Child cancellation propagates user cancellation/shutdown downward without turning a source error into a user cancellation.
- Queue close drains accepted work. Cancelled tasks return promptly, artist-lock waits poll cancellation, and album verification checks cancellation between pages. Unexpected page/finalization panics become stable worker failures rather than silently losing a worker.
- Received albums use the existing durable `hashing` state while waiting for finalization. The normal startup interruption recovery requeues them and reuses verified checkpoints; no schema migration is required. A partial or changed file is not silently accepted.

## Reused validation

- A freshly decoded WebP carries a process-local digest of the validated bytes. Storage checks that digest before bypassing the second full decode. Changed buffers, other formats and payloads without this proof follow the original decoder path.
- Overlap hashing checks path, size and SHA, then performs its own full decode once; the read helper no longer decodes the same buffer immediately beforehand.
- Incoming album integrity is checked once after acquiring artist locks instead of twice in immediate succession. Existing-candidate checks and latest-state checks before exclusion/merge remain.

## Adaptive concurrency

- Ordinary successful requests supply observations; no benchmark traffic is generated.
- A window needs at least 30 seconds, eight successful requests and 2 MiB. Sparse samples accumulate for up to ten minutes rather than being thrown away every short window.
- Recovery trials add only one request slot. An improved comparable window must confirm the trial before its limit is persisted. Unconfirmed trials revert.
- Local image-work pressure alone does not lower the network limit. Network errors, rate/latency regression and server backpressure still can. Retry-After is never shortened; ordinary unsuccessful trial retesting waits two minutes.
- HTTP/2 and larger concurrency ceilings are not enabled by this change. They require a separate controlled comparison after measuring the pipeline.

## Diagnostics and verification

Album reception/handoff, finalization queue wait and finalization duration are logged at info level. Debug logs add HTTP response time/bytes, source-validation versus storage time, and artist-lock wait. They do not record signed source URLs, credentials or page contents. HTTP timing is aggregate service time, not a separate TTFB measurement.

Regression coverage includes single-album page parallelism, the global eight-worker ceiling, out-of-order checkpointing, failure-driven sibling cancellation, shutdown during artist-lock wait, interrupted-job recovery, digest-proof mutation/cancellation, and low-rate concurrency recovery.

This change does not implement WebView/renderer memory stabilization, alter retained thumbnail caches, or claim a measured speed multiplier. Live throughput depends on the server, image format, disk and overlap workload.

Verified on 2026-09-19 (KST): frontend 115 files / 1,147 tests; Rust library 796 passed / 20 existing ignored, main 2 passed; frontend typecheck/build, all-target Clippy with warnings denied, changed-file formatting and diff checks passed. Windows-native tests used isolated temporary data outside the restricted test sandbox. Frontend full-suite verification passed with two workers after an overloaded parallel run produced timing failures. The development executable was rebuilt; no app launch, user-data edit, commit, push or release was performed.
