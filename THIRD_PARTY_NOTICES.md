# Third-party notices

## Fluent UI System Icons 1.1.328

The card metadata and status icons include vector paths from
[Microsoft Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons/tree/1.1.328).
The vendored paths are the 20px Regular variants of Person, People, Warning,
Arrow Download, and Checkmark. No remote icon script or font is loaded at runtime.

MIT License

Copyright (c) 2020 Microsoft Corporation.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Packaged dependencies

The official CHZZK viewing bridge directly uses webview2-com 0.38.2 (MIT).
Tauri 2.11.5 is vendored under the MIT option with Atsumi-specific remote IPC
boundary changes documented in `src-tauri/vendor/ATSUMI_PATCHES.md`. Original
license texts are retained in the vendor source and `public/licenses/`.
Cheese-PIP source code and CHZZK player assets are not copied into Atsumi;
the official website is loaded normally in an isolated WebView2 profile.

The experimental CHZZK workspace adds hls.js 1.7.2 (Apache-2.0), fs2 0.4.3
(MIT option), tungstenite 0.28.0 (MIT option), and chrono 0.4.45 (MIT option).
Their upstream notices and the
Apache-2.0 license text are preserved in `public/licenses/`, which Vite copies
into the packaged frontend. These dependencies are unmodified; Atsumi's
integration code is separate.

Remaining JavaScript and Rust dependency notices are not copied here by hand. Their
exact reviewed versions and source checksums are locked in `pnpm-lock.yaml` and
`src-tauri/Cargo.lock`; their upstream license files are included by the normal
Tauri/Cargo and pnpm distribution process where required. Any vendored asset
that is not represented by those package manifests must be listed explicitly
in this file before release.

## Experimental AVIF decoder dependencies

## Recording merge tools

Official-player recordings are remuxed locally with unmodified FFmpeg/ffprobe
shared executables from BtbN's LGPL Windows build:
`n9.0.1-29-gad500d59cb-20260911`.
The package's supplied `LICENSE.txt` states GNU LGPL version 3.
The complete package, including its license and documentation, is retained in
the application's `media-tools` resources; it is not installed globally.

- FFmpeg project and source: <https://ffmpeg.org/>
- Binary build provenance: <https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-09-11-13-20>
- Asset: `ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0.zip`
- SHA-256: `40eec25b2f55dcad7e4d4e640919b920d29818b56fdaf9353ce1fd8adefc9d6b`

`tools/prepare_media_tools.ps1` verifies this digest before extraction. Before
publishing a release, retain the upstream notices and provide the applicable
corresponding source/build material for the exact distributed binaries and
their included libraries. A link in this development note is not a completed
redistribution compliance audit.

## Experimental AVIF decoder dependencies

Atsumi currently pins the following pure-Rust crates for bounded, experimental AVIF
decoding. Both are distributed under the MIT License:

- `avif-rust` 0.0.6 — <https://github.com/mith-mmk/avif-rust> — MIT, Copyright (c) 2023 MITH@mmk
- `bin-rs` 0.0.10 — <https://github.com/mith-mmk/bin-rs> — MIT, Copyright (c) 2023 MITH@mmk

The exact versions are intentionally pinned in `src-tauri/Cargo.toml` and
`src-tauri/Cargo.lock` because the decoder API and implementation are still treated as
experimental.

MIT License

Copyright (c) 2023 MITH@mmk

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
