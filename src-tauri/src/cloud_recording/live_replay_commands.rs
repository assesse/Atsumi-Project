//! Main-app-only commands. No raw document token, path, offset or frame input.
use super::{run,service};
use crate::cloud_recording::{CloudError,live_replay_session::{Opened,Snapshot},live_replay_index::Page};
#[tauri::command]
pub async fn cloud_live_replay_open(window:tauri::Webview,selection_id:String,expected_revision:u64,request_id:String)->Result<Opened,CloudError>{
    run(window,true,move|state,app|{let service=service(state,app)?;service.live_replays()?.open(app,&service,&selection_id,expected_revision,&request_id)}).await
}
#[tauri::command]
pub async fn cloud_live_replay_cancel_open(window:tauri::Webview,selection_id:String,expected_revision:u64,request_id:String)->Result<(),CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.cancel_open(&selection_id,expected_revision,&request_id)).await
}
#[tauri::command]
pub async fn cloud_live_replay_snapshot(window:tauri::Webview,token:String)->Result<Snapshot,CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.with(&token,|s|s.snapshot())).await
}
#[tauri::command]
pub async fn cloud_live_replay_clock(window:tauri::Webview,token:String)->Result<Option<crate::youtube::original::VodClock>,CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.with(&token,|s|s.clock())).await
}
#[tauri::command]
pub async fn cloud_live_replay_viewport(window:tauri::Webview,token:String,viewport:crate::platform_native_geometry::NativeViewport)->Result<(),CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.with(&token,|s|s.viewport(viewport))).await
}
#[tauri::command]
pub async fn cloud_live_replay_control(window:tauri::Webview,token:String,request:crate::youtube::original::VodControlRequest)->Result<(),CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.with(&token,|s|s.control(request))).await
}
#[tauri::command]
pub async fn cloud_live_replay_fullscreen(window:tauri::Webview,token:String,enabled:bool)->Result<(),CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.with(&token,|s|s.fullscreen(enabled))).await
}
#[tauri::command]
pub async fn cloud_live_replay_controls(window:tauri::Webview,token:String,can_close:bool)->Result<(),CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.with(&token,|s|s.controls(can_close))).await
}
#[tauri::command]
pub async fn cloud_live_replay_measure(window:tauri::Webview,token:String)->Result<Snapshot,CloudError>{
    run(window,true,move|state,app|service(state,app)?.live_replays()?.measure(&token)).await
}
#[tauri::command]
pub async fn cloud_live_replay_page(window:tauri::Webview,token:String,mode:String,cursor:Option<String>,limit:Option<usize>)->Result<Page,CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.with(&token,|s|s.page(&mode,cursor.as_deref(),limit.unwrap_or(80)))).await
}
#[tauri::command]
pub async fn cloud_live_replay_asset(window:tauri::Webview,token:String,asset_id:String)->Result<Option<String>,CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.with(&token,|s|s.asset(&asset_id))).await
}
#[tauri::command]
pub async fn cloud_live_replay_close(window:tauri::Webview,token:String)->Result<(),CloudError>{
    run(window,false,move|state,app|service(state,app)?.live_replays()?.close(&token)).await
}
