# Debug launcher recovery — 2026-09-21

## Failure

The desktop debug shortcut correctly selected the shared `2849` workspace.
The app did not start: Windows PowerShell 5.1 failed before Node/Tauri ran with
`AmbiguousParameter` for parameter name `''`.

The launcher invoked a second PowerShell process with:

```text
-File run_frontend.ps1 tauri dev -- -- /prefetch:2
```

The bare `--` separators were interpreted by PowerShell's script parameter
binder instead of reaching the Tauri/Cargo command. A successful Rust build
does not exercise this launch path. The earlier prefetch test only parsed the
script and matched text; it could not detect the runtime binding error.

## Repair

- The child PowerShell process now runs `run_tauri_dev.ps1` with no script
  arguments crossing the native `-File` boundary.
- That wrapper passes `dev`, both separators, and `/prefetch:2` to the frontend
  runner through an explicit `string[]` argument. The fixed application prefetch
  scenario remains unchanged.
- The launcher checks for the new wrapper before starting.
- `test_debug_launcher.ps1` starts actual Windows PowerShell 5.1 in an isolated
  path containing spaces, reuses the real frontend runner's parameter contract,
  verifies the exact received arguments, and checks child exit-code propagation.
  Its fixture does not start an app, Cargo, Node, or any recording.
- The prefetch regression suite includes the executable argument-binding test.

No release binary, desktop shortcut, user recording, or recording configuration
is changed by this repair. Prior launch logs were preserved under
`.runtime/debug-launch-repair-20260921-010050` before the actual launch test.

## Verification

- Argument-binding test and the full shortcut/prefetch regression script passed.
- Invoked the actual desktop debug shortcut's target, arguments, and working
  directory. Vite started and Tauri successfully ran
  `target\debug\atsumi.exe /prefetch:2` after a 45.22-second incremental build.
- The resulting debug process (PID 4640) reported `interactive_frame` at
  1,090 ms and `recordings_ready` at 1,348 ms after process creation. Windows
  reported a responsive main window titled `Atsumi`.
- The computer-use helper did not expose that window as a target, so no visual
  inspection or click-through result is claimed. The app was left running.
