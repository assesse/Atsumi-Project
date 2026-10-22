//! Full desktop app with a NEW, isolated identity and media directory.
//! It neither copies nor touches the ordinary app's database or account session.
//! Start the isolated Vite server on port 5179, or pass its explicit port.
use std::{path::PathBuf, sync::Arc};
use atsumi_lib::{application::ApplicationService, domain::SettingsPatch, infrastructure::SqliteRepository};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port = 5179u16;
    let mut port_seen = false;
    let mut click_probe = false;
    for arg in std::env::args().skip(1) {
        if arg == "--soop-vod-click-probe" && !click_probe { click_probe = true; }
        else if !port_seen && !arg.starts_with('-') { port = arg.parse()?; port_seen = true; }
        else { return Err("expected optional port and --soop-vod-click-probe only".into()); }
    }
    if click_probe && !cfg!(debug_assertions) { return Err("click probe is debug-only".into()); }
    // Never inherit opt-in accidentally from a parent shell.
    std::env::remove_var("ATSUMI_SOOP_VOD_CLICK_PROBE");
    if click_probe { std::env::set_var("ATSUMI_SOOP_VOD_CLICK_PROBE", "1"); }
    if port < 1024 { return Err("non-system loopback port required".into()); }
    let id = format!("local.atsumi.platformqa.{}", uuid::Uuid::new_v4().simple());
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().ok_or("workspace missing")?.to_owned();
    let root = workspace.join(".runtime/platform-app-qa").join(&id);
    std::fs::create_dir_all(root.join("media"))?;
    // Only the isolated process's diagnostic recorder is redirected.
    std::env::set_var("LOCALAPPDATA", root.join("local"));
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("Windows application data unavailable")?).join(&id);
    if data.exists() { return Err("test identity already exists; nothing overwritten".into()); }
    std::fs::create_dir_all(&data)?;
    let repository = Arc::new(SqliteRepository::open(data.join("atsumi-next.sqlite3"))?);
    let service = ApplicationService::new(repository);
    let revision = service.settings_get()?.revision;
    service.settings_update(SettingsPatch {
        download_root: Some(root.join("media").to_string_lossy().into_owned()),
        privacy_on_startup: Some(false),
        ..Default::default()
    }, revision)?;
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = id.clone();
    context.config_mut().build.dev_url = Some(format!("http://127.0.0.1:{port}").parse()?);
    for window in &mut context.config_mut().app.windows { window.title = "Atsumi · 플랫폼 검증 (격리)".into(); }
    println!("QA_IDENTITY={id}");
    println!("QA_MEDIA_ROOT={}", root.join("media").display());
    atsumi_lib::run_with_context(context)?;
    Ok(())
}
