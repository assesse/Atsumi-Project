fn main() {
    tauri_build::build();

    // Cargo examples and lib tests do not inherit the app's Windows manifest.
    // Tauri's test runtime and WebView probes need Common Controls v6 as well.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
        // The app binary already embeds Tauri's complete manifest via resource.lib.
        // Do not generate a second MANIFEST resource with the same id for that target.
        println!("cargo:rustc-link-arg-bin=atsumi=/MANIFEST:NO");
    }
}
