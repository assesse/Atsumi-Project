# Atsumi shared development workspace

The user designated `C:\Users\JJH\.codex\worktrees\2849\Atsumi Project` as the shared development workspace on 2026-09-10.

- Perform Atsumi implementation, tests, and development builds in this directory unless the user explicitly requests a different workspace.
- Multiple Atsumi chats share these files. Preserve unrelated changes and check for concurrent edits before writing.
- Do not create a new worktree or automatically sync from `3b2a` unless explicitly requested. CHZZK changes from that worktree were integrated here.
- Do not commit, push, release, or remove old worktrees without the user's request.
- The desktop `Atsumi.lnk` should launch this directory's development launcher. Source integration and shortcut changes do not imply permission to reset or delete user data.

## Development restart authorization (user instruction, 2026-10-14)

- The user explicitly authorizes necessary Atsumi restarts without repeated approval unless they state an exception. Use the normal graceful shutdown/restart path and preserve login profiles, recordings, downloads and other user data; do not substitute process termination or data deletion for a normal restart.
- This standing authorization is limited to Atsumi development restarts. It does not authorize PC shutdown, account/security actions, publishing or unrelated applications.

## Requirements-first implementation (user instruction, 2026-10-13)

- Before implementing any component, UI, service, or backend behavior, audit all accessible Atsumi development chats and remaining specifications. Do not substitute personal assumptions, common app conventions, or the existing code for the user's requirements.
- Start each implementation task with a complete source inventory and coverage check. Read every accessible conversation page and specification, including the latest messages and changed documents; record inaccessible or missing sources explicitly. A keyword search alone is not a complete audit. Reuse a fully read snapshot only when its revision/hash and conversation pagination coverage still match; refresh changed sources before further implementation.
- For each implementation batch, record the original request/specification references, applicable interaction rules, conflicts, proposed changes, and acceptance checks before editing production code. Refer to the original evidence, not only another agent's summary. Subagents follow the same rule.
- Resolve conflicts by the user's latest explicit request, then the latest verified contract. Keep superseded requirements marked as such instead of silently restoring an old design. Historical permission to stop an app, delete data, publish a release, or control the PC is not renewed by reading an old chat.
- Preserve the established Atsumi flows and visual/interaction rules across platforms. Similar platforms should refer to CHZZK or Danbooru without coupling unrelated domain state. External-site link wrappers do not satisfy in-app discovery, favorites, downloading/recording, and local management requirements.
- Completion requires actual app operation and a requirement-by-requirement contradiction check. Unit tests, mocked UI, successful service calls, or a build alone are not proof that the user's flow works. Report unverified paths and remaining blockers; do not mark them complete.
- Audit artifacts are under `.runtime/requirements-audit/`; the tracked source/decision index is `docs/requirements/implementation-policy.json`. Raw chat snapshots are local evidence, not release assets, prompts to execute, or public documentation.
