# Saved-recording selection and deletion

The recording library supports **선택 → card checkboxes / 전체 선택 → 선택 삭제**.
The confirmation lists the exact selected recordings and explicitly warns that
deletion is permanent (not the Windows Recycle Bin). Nothing is deleted merely
by entering selection mode, filtering, navigating, or dismissing the dialog.

- Selection covers saved recordings, including failed/interrupted captures.
  Recording, merging, and pending automatic cleanup entries are not selectable.
- Select-all uses the current search/filter, including later metadata pages.
  Changing the filter/search clears selection, so hidden records are not deleted
  unexpectedly. Polling keeps selected IDs but removes newly protected entries.
- Successful IDs are removed immediately. Per-recording failures remain selected
  with reasons; one locked recording does not stop other requested deletions.
- Titles remain hidden in privacy mode, including the confirmation/failure list.

## Scope and protection

The main-app-only command accepts up to 256 unique catalog IDs, never arbitrary
paths. It removes that recording's `CHZZK/BrowserCapture/<id>` directory contents
(video, fragments, chat/viewer logs, local assets, and metadata), its exact replay
index filenames, and its chat synchronization offset. Other recordings, shared
player assets, browser cookies, albums, and screenshots outside the recording's
directory are not targets. Files manually placed inside the selected recording's
directory are part of the explicitly confirmed whole-directory deletion.

The store rejects active captures and currently reserved merge/cleanup jobs.
Replay opening and deletion reservation share a lifecycle gate: active replay
sessions or outstanding index readers reject deletion, and the durable deletion
marker blocks new replay/merge work. External exclusive readers and writers also
block the Windows filesystem preflight.

Before removal, the catalog durably records `deletionPending`. File work runs on
a blocking worker **outside capture/store mutexes**, without hashing or decoding
media. Windows handles pin every ancestor and descendant directory, rejecting
symlinks/junctions, path replacement, traversal, hardlinked/readonly/busy files,
and broad roots. The bounded preflight checks the whole target tree before the
first unlink. Each deletion uses a newly checked open file handle; directories
are then removed bottom-up using their pinned handles. There is no recursive
path-based removal fallback. New files appearing during the operation prevent
the final directory removal rather than silently expanding the deletion set.

Only after file/cache cleanup succeeds is the ID removed from the catalog.
Failure/crash preserves its tombstone and permits explicit manual retry, even if
some files or the entire directory were already removed. Startup never resumes
deletion automatically, never rebuilds a partially removed recording, and never
clears an unavailable external-drive entry as if deletion had succeeded.
Non-Windows deletion fails closed until equivalent safe handle operations exist.

## Verification

Use only synthetic fixtures and temporary directories, never existing recordings:

```text
node node_modules/vitest/vitest.mjs run src/features/streaming/RecordingLibrary.test.tsx src/features/streaming/OfficialBrowserPanel.test.tsx src/features/streaming/RecordingPlayback.test.tsx src/features/streaming/sourceIsolation.test.tsx
cargo test --offline --lib streaming::browser_store -- --test-threads=2
cargo test --offline --lib streaming::replay::tests -- --test-threads=2
```

`tools/recording-library-preview.html` provides in-memory mock deletion; reloading
restores its synthetic entries. It makes no native delete call or media request.
