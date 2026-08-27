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

JavaScript and Rust dependencies are not copied into this notice by hand. Their
exact reviewed versions and source checksums are locked in `pnpm-lock.yaml` and
`src-tauri/Cargo.lock`; their upstream license files are included by the normal
Tauri/Cargo and pnpm distribution process where required. Any vendored asset
that is not represented by those package manifests must be listed explicitly
in this file before release.

## Experimental AVIF decoder dependencies

Atsumi Next currently pins the following pure-Rust crates for bounded, experimental AVIF
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
