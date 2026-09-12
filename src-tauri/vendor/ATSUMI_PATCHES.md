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

Why strings: Wry's WebMessage handler expects a string. Posting an object causes
`TryGetWebMessageAsString` to fail before subsequent WebView2 listeners run.

Re-audit these restrictions and regression tests whenever upgrading Tauri. This
is an application-specific integration patch, not a claim about an upstream fix.
