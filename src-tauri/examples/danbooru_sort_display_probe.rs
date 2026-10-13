//! Displays the opt-in live-test artifact, without AppState or user data.
fn main() {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.runtime/danbooru-sort-verification.html")
        .canonicalize()
        .expect("run the live sort test with ATSUMI_DANBOORU_TEST_SCREENSHOT=1 first");
    let url = tauri::Url::from_file_path(path).unwrap();
    let allowed = url.clone();
    let profile = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(
                app,
                "danbooru-verification",
                WebviewUrl::External(url.clone()),
            )
            .title("Atsumi · 단부루 실제 응답 검증")
            .inner_size(1200.0, 800.0)
            .data_directory(profile.path().join("webview"))
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_navigation(move |next| next == &allowed)
            .build()?;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let _profile = profile;
                std::thread::sleep(std::time::Duration::from_secs(120));
                app.exit(0);
            });
            Ok(())
        })
        .build(context)
        .unwrap()
        .run_return(|_, _| {});
}
