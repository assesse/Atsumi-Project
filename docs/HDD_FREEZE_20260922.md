# HDD contention and the frozen Explore screen — 2026-09-22

## Evidence (KST)

- Atsumi debug backend PID 27084 continued completing downloads/final checks while the main UI no longer handled input.
- The main WebView2 profile had no surviving renderer. Its 15:47:07 Crashpad minidump reports exception `0xE0000008` (Chromium out of memory), renderer PID 2740, WebView2 153.0.4234.48. This is a confirmed renderer failure, not evidence of a global backend deadlock.
- The dump parameters describe a failed allocation and Windows commit capacity, **not the renderer's measured resident memory**. No pre-crash heap profile is available; the exact allocation responsible for growth is not established.
- C: is the NVMe SSD and contains SQLite/WAL and `thumbnail-cache`. D: is the ST2000LM015 HDD and contains `D:\Astumi` media. Earlier samples reached 97–98% HDD active time with ~108–113 MiB/s reads; later samples varied. High active time alone is not a transfer-throughput metric.
- Album final verification rereads SHA-256 checkpoints/candidate files. Recording merge separately reads inputs, muxes, decodes the result and verifies cleanup proofs. These jobs previously had no HDD read pacing. Proof checks protect originals from unsafe deletion and remain in place.
- Logs and the crash dump were copied before restarting to the task's `hdd-freeze-20260922` diagnostic directory. No user media or database was deleted for this repair.

## Changes

1. Cover retention accounts for encoded bytes **plus decoded RGBA pixels**. Session pins have a 192 MiB estimated budget; unused display handles have an independent 64 MiB budget as well as their TTL/count limit. These are cache accounting limits, not a cap on total process memory. Visible subscribers are not evicted. Canonical SSD cache files remain usable without redownloading or reading the album original on a cache hit.
2. Cancelled thumbnail requests leave both pending maps immediately. Unowned/late completion byte arrays are discarded; legitimate response-before-token events remain scoped to their pending same-key handshake. The former count-only unsolicited-image buffer is removed.
3. A terminated main renderer is logged and reloaded without restarting Rust workers. Automatic recovery is limited to two attempts per five minutes to avoid a crash loop. GPU/utility failures and temporarily unresponsive renderers are not forcibly reloaded. The tray also provides `화면 새로 고침` independently of React.
4. Windows' storage seek-penalty query detects rotational volumes. SHA checkpoint/recording-proof reads on HDD share a 24 MiB/s cooperative budget per volume **and yield at least their actual read duration**. A byte ceiling alone did not address high active time on small/slow reads in runtime samples. Local FFmpeg remux/decode input is paced to an approximate 16 MiB/s target based on bitrate. Repeated segment metadata probes also leave a cancellable gap (up to 250 ms) on HDD. SSD/unknown devices are unthrottled. No network-download, live-capture, playback-rate or system-wide policy is changed. Cancellation is checked at most 25 ms apart during budget waits.
5. MP4 local exports no longer request `+faststart`, avoiding its extra whole-file relocation pass. Local replay already serves byte-range seeks and can read a trailing `moov` atom. Full decode/hash/proof verification before source deletion remains mandatory.
6. A local thumbnail cache miss validates the SHA and decodes the original once, instead of decoding it once for validation and again for thumbnail generation.

## Trade-offs and limits

- Background checks/merges can finish later on HDD; interactive work and live capture keep headroom. The budget is not a promise that total disk active time will never reach 100%: writes, seeks, other programs, and unpaced I/O still contribute.
- FFmpeg pacing is based on average bitrate, not a hard physical-device byte cap. Caches and variable bitrate affect measured physical traffic.
- The OOM endpoint is confirmed; cache cleanup defects are verified in code/tests. Establishing how much each defect contributed to this particular crash would require a pre-crash memory trace. Recovery protects against renderer loss but does not assert that every possible source of memory growth is removed.
- Other chats' pending source changes remain in place. No commit, push or release is part of this task.

## Verification

- Frontend: 121 files / 1,207 tests passed; typecheck and Vite build passed.
- Backend after the elapsed-time/probe-gap supplement: 852 passed, 23 opt-in tests ignored; the development build passed.
- Six synthetic FFmpeg integration tests passed again after the supplement, including MP4 source-clock concat, random-access/progressive replay, final export/cleanup, and rejection of a corrupted middle sample.
- Read-only native volume checks: D:\Astumi enables HDD pacing, C:\Users\JJH\AppData\Roaming\local.atsumi.next does not.
- Runtime: `artist:atage` returned 48 results with cover graphics after restart. No download was newly requested by this smoke check.
- Recovery fault injection: terminated only main renderer PID 27204 at 16:25:15 KST. `ProcessFailed` logged kind 1 / reason 3 / exit -1 / reload true; renderer PID 30132 replaced it. Backend PID 36260 and its original start time remained unchanged. The new main document's accessibility tree was present. This tests renderer termination, not a deliberate system OOM.
- Initial post-fix HDD samples still included 97–100% active time at 5.8–8.5 MiB/s. They motivated the elapsed-read-time/probe-gap supplement rather than claiming the bandwidth cap had resolved all contention.
- Final development restart: backend PID 25676, 16:41:36 KST. The main document loaded; startup recovered/resumed eight existing download jobs with zero startup recovery issues. The HDD policy was detected for D: at 16:41:38. A resumed job completed its final verification successfully at 16:43:48. No new download was requested by the diagnostic work.
- Final short HDD samples at 16:44:07–16:44:14: active time 22%, 23%, 20%, 14%; reads 9.25, 9.43, 9.44, 1.63 MiB/s; queue depth 0–1. An earlier startup sample still reached 100% with a queue of 11. These are observations under the current workload, not a controlled before/after benchmark or a guarantee against future saturation.
- Computer Use inspected the final main document's accessibility tree, but the final click check was blocked by the automation helper's `coordinate input geometry is unavailable` error. The earlier search/recovery checks above are the completed input/recovery evidence; do not treat the final tree inspection as a complete interactive regression test.

## References

- [Microsoft: WebView2 process failures and recovery](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-related-events)
- [Chromium Breakpad: Windows minidump exception codes](https://chromium.googlesource.com/breakpad/breakpad/+/master/src/google_breakpad/common/minidump_exception_win32.h)
- [Chromium allocator: out-of-memory crash parameters](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/base/allocator/partition_allocator/src/partition_alloc/oom.cc)
- [Microsoft: storage seek-penalty descriptor](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-device_seek_penalty_descriptor)
