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

To update, extract the new portable ZIP into a NEW folder. Finish current
work, fully quit Atsumi, then run the new copy. Do not run two versions at
once or extract over a running executable. The installer updater is blocked
for this package to keep it portable. No user data is deleted on update.

Playback compatibility depends on the upstream site. Available quality,
login and age/entitlement requirements still apply. Only record content
you are entitled to save. No login/session from Chrome or Edge is copied.
