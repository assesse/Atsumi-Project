//! Resume this task's isolated app after a rebuild, retaining its own OAuth and media.
//! No ordinary profile, copying, resetting, or initial upload is performed here.
#[cfg(all(windows, debug_assertions))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, os::windows::fs::MetadataExt, path::{Path, PathBuf}};
    const ID: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
    fn plain(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        for ancestor in path.ancestors() {
            let info = fs::symlink_metadata(ancestor)?;
            if info.file_type().is_symlink() || info.file_attributes() & 0x400 != 0 { return Err("linked QA path refused".into()); }
        }
        Ok(())
    }
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--confirm-resume"] { return Err("explicit --confirm-resume required".into()); }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("workspace missing")?;
    let root = workspace.join(".runtime/platform-app-qa").join(ID);
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("application data missing")?).join(ID);
    plain(&root.join("media"))?; plain(&data.join("atsumi-next.sqlite3"))?;
    let receipt_path = root.join("cloud-real-copy-receipt.json"); plain(&receipt_path)?;
    if fs::metadata(&receipt_path)?.len() > 1024 * 1024 { return Err("oversized QA receipt".into()); }
    let receipt: serde_json::Value = serde_json::from_slice(&fs::read(receipt_path)?)?;
    if receipt["version"] != 2 || receipt["identity"] != ID || receipt["originalFilesUnchanged"] != true
        || receipt["catalogMetadataMatchesExpected"] != true { return Err("verified isolated profile required".into()); }
    std::env::remove_var("ATSUMI_SOOP_VOD_CLICK_PROBE");
    std::env::set_var("LOCALAPPDATA", root.join("local"));
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = ID.into();
    context.config_mut().build.dev_url = Some("http://127.0.0.1:5181".parse()?);
    for window in &mut context.config_mut().app.windows { window.title = "Atsumi · 플랫폼 검증 (격리)".into(); }
    println!("QA_IDENTITY={ID}");
    atsumi_lib::run_with_context(context)?;
    Ok(())
}
#[cfg(not(all(windows, debug_assertions)))]
fn main() { eprintln!("QA_REQUIRES_WINDOWS_DEBUG_BUILD"); std::process::exit(2); }
