# Atsumi desktop boundary patch

`tauri-2.11.5` is copied from the installed crates.io source without changes to its
MIT/Apache-2.0 license files. Atsumi uses its Windows official-site webview only
through a dedicated recording bridge, not Tauri IPC.

Local changes are limited to the IPC boundary:

- Reject all Tauri invocations from remote document origins before dispatch,
  including the internal `__TAURI_CHANNEL__|fetch` exception. Trusted local app
  command/channel behavior remains unchanged.
- Ignore the reserved `ATSUMI_BROWSER_CAPTURE:` WebMessage string prefix in the
  Tauri parser. The independent native recording handler validates the sending
  URL, one-use user-issued start nonce, recording ID and payload limits. This
  avoids treating recording fragments as invalid Tauri command envelopes.
- Treat the `atsumi-player` scheme and its Windows HTTP(S) mapping as untrusted
  for every IPC command, even though its immutable public assets are registered
  as a custom scheme. Its local iframe only receives a narrow parent message
  bridge and a replay media capability.
- Explicitly guard main-frame-only initialization scripts with
  `window.self === window.top` on Windows. Wry 0.55.1 documents that its Windows
  implementation otherwise injects these scripts into every subframe. This
  prevents the opaque original-player iframe from receiving Tauri globals or
  the invocation key; the native origin rejection remains independent protection.

Why strings: Wry's WebMessage handler expects a string. Posting an object causes
`TryGetWebMessageAsString` to fail before subsequent WebView2 listeners run.

Re-audit these restrictions and regression tests whenever upgrading Tauri. This
is an application-specific integration patch, not a claim about an upstream fix.

## Windows input deadlock (Tao 0.35.3)

`tao-0.35.3` preserves the installed crates.io source and license. Its four
Windows input files backport the exact upstream commit `c704261c`:
https://github.com/tauri-apps/tao/pull/1215

The live Atsumi UI stack on 2026-09-12 showed `KeyEventBuilder::process_message`
calling `PeekMessageW`, synchronously reentering the window procedure, then
waiting forever on its already-held `KEY_EVENT_BUILDERS` mutex. The app was
minimized; the main, tray and single-instance windows shared the blocked thread.

The upstream change moves keyboard and IME message peeks before input locks,
passes the precomputed result into the handlers, and shortens layout-cache
guards. It does not replace the runtime with the broader Tao 0.36 rewrite.
Re-audit this backport when upgrading `tauri-runtime-wry` / Tao.
