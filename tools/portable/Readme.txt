Atsumi - Windows portable

Extract the entire Atsumi folder before starting Start-Atsumi.cmd.
Do not copy atsumi.exe alone: media-tools contains recording merge tools,
their shared DLLs, and licenses. Chrome/Edge extensions and the NAVER grid
connector are not required by the CHZZK playback compatibility mode.

Microsoft WebView2 Runtime is still required. Most Windows PCs have it.
An optional official Fixed Version runtime can be packaged in WebView2/;
Start-Atsumi.cmd selects it for this process without installing it globally.
Windows 10 Fixed Version deployments also require Microsoft's documented
AppContainer read/execute permissions. Do not disable browser sandboxing.

This is installation-free, not an all-data-on-USB mode. Settings, login and
community identity stay in the existing AppData location. Downloads and
recordings stay in the folder selected in Settings.

From 2.1.1, the in-app updater downloads a signed portable ZIP. After you
accept the update and finish downloads/scans/recordings, it verifies the ZIP,
closes normally, replaces only application files and restarts. No installer
is run and no user data is deleted. A writable, local, non-linked folder is
required. An optional WebView2/ runtime is preserved as-is.

Older portable versions need one manual migration: extract this ZIP into a
NEW folder, fully quit the old Atsumi and start the new copy. Later updates
can be applied in-app. Do not run both copies at once.

Updates keep rollback files and result.txt under .atsumi-update/<id>/.
If replacement fails the old files are restored. After a power loss during
replacement, close Atsumi and recover application files from that backup
folder, or extract a fresh ZIP into a new folder; AppData is unaffected.

Playback compatibility depends on the upstream site. Available quality,
login and age/entitlement requirements still apply. Only record content
you are entitled to save. No login/session from Chrome or Edge is copied.
