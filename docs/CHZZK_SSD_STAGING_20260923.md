# CHZZK SSD recording workspace

## User-facing contract

- Settings → General → **CHZZK 녹화에 SSD 사용**, off by default. Persisted with settings revision/CAS (schema 49).
- Applies to new recordings only; existing recordings and active paths are not migrated by toggling.
- Confirm local fixed SSD volumes with Windows seek-penalty queries; unknown, removable and network drives are excluded. Choose the confirmed SSD with most free space, requiring 20 GiB at start. If the configured destination is already SSD, record there directly without redundant transfer.
- Work inside a per-user Atsumi directory on that SSD. Destination remains the configured download folder, captured when the recording starts. Turning the option off does not abandon pending transfers.
- Reserve 10 GiB plus the estimated final merge space of all unmerged staged recordings. If insufficient, fail visibly and preserve finalized media; never silently switch back to HDD or delete older recordings.

## Storage lifecycle

1. Capture original encoded media, chat, viewer metrics and profile/assets into the SSD recording directory. Network reception and playback are unchanged.
2. Existing merge pipeline validates a final derivative and safely removes its original segments on SSD.
3. A fair slot in the background merge worker reserves a completed recording for archival. Bulk copy/hash work runs outside all capture/catalog mutexes and uses the existing HDD pacing budget.
4. Copy to an ID-specific destination, protected by an ownership marker. Flush each copy and read it back to verify its SHA-256. Existing identical files are reusable; mismatching files are never overwritten. A bounded inventory includes chat, metrics, profiles, assets, timelines and diagnostics.
5. Persist destination metadata/transfer receipt before atomically publishing the new catalog path in app data. A replay opened against the old path defers publication; playback is not interrupted.
6. Pin and reverify matching destination/source files before deleting SSD working files through checked Windows handles. Keep the tiny SSD metadata/receipt as recovery evidence. Explicit library deletion handles both owned locations.

## Failure and recovery

- Before publication, SSD remains authoritative. Copy failure, cancellation, an absent/full HDD or a destination collision never removes SSD media.
- After publication, the destination is authoritative and cleanup can resume from its durable receipt. A crash between copy and publication reuses matching copies rather than creating a new recording/library entry.
- Failed transfers retry after five minutes, on app restart, or via **보관 이동 다시 시도**. Current status/error is visible in the recording library.
- Symlink/reparse points, path traversal, unexpected ownership and source mutation fail closed. Automatic cleanup never recursively deletes a folder.
- This does not guarantee unlimited recording duration on a finite SSD. Very long concurrent broadcasts may still reach the reserved-space boundary; chunked archival of ongoing broadcasts is outside this change.

## Verification scope

Synthetic local storage tests cover successful transfer, preservation of auxiliary files, restart boundaries, cancellation, mismatch/corruption, unavailable destination, replay leases, explicit deletion and path validation. Native SSD discovery is tested read-only against actual volumes. No real recording is moved or deleted during tests.

Verified on 2026-09-23: 879 backend tests and 192 targeted frontend tests passed; native discovery identified C: as SSD and D: as HDD. A separate tiny synthetic recording bundle was transferred from C: to D: and verified, including auxiliary assets and safe source cleanup. Type checking, production frontend build and the development executable build passed. Real broadcast endurance is not covered by these fixture tests.

CHZZK exposes global settings through the bottom-left gear, so the option can be enabled without switching content sources. Tests and builds do not turn the option on or migrate existing recordings.
