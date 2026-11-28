//! Opt-in, one-source local verification. No cloud service, login or UI input.
//! All newly created artifacts are retained, including an incomplete capture.
use atsumi_lib::{platform_media::{MediaJob, PlatformMediaService, PlatformMediaTools}, soop::original::SoopOriginalHost};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, fs, path::{Path, PathBuf}, sync::{Arc, Mutex}, thread, time::{Duration, Instant}};
use tauri::Manager;

const CHANNEL: &str = "ecvhao";
const BROADCAST: &str = "298637823";
const RECORD_SECONDS: u64 = 120;
const SOFT_BYTES: u64 = 256 * 1024 * 1024;
const FREE_RESERVE: u64 = 2 * 1024 * 1024 * 1024;

fn own_directory(path: &Path) -> Result<(), &'static str> {
    for parent in path.ancestors() {
        let metadata = fs::symlink_metadata(parent).map_err(|_| "path_metadata")?;
        if metadata.file_type().is_symlink() { return Err("path_link"); }
        #[cfg(windows)] {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 { return Err("path_reparse"); }
        }
    }
    Ok(())
}
fn tree_bytes(root: &Path) -> Result<u64, &'static str> {
    let mut pending = vec![(root.to_owned(), 0usize)];
    let mut entries = 0; let mut bytes = 0u64;
    while let Some((path, depth)) = pending.pop() {
        if depth > 6 { return Err("tree_depth"); }
        own_directory(&path)?;
        for entry in fs::read_dir(path).map_err(|_| "tree_read")? {
            let entry = entry.map_err(|_| "tree_entry")?;
            entries += 1; if entries > 4096 { return Err("tree_count"); }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|_| "tree_metadata")?;
            #[cfg(windows)] {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 { return Err("tree_reparse"); }
            }
            if metadata.file_type().is_symlink() { return Err("tree_link"); }
            if metadata.is_dir() { pending.push((entry.path(), depth + 1)); }
            else if metadata.is_file() { bytes = bytes.checked_add(metadata.len()).ok_or("tree_overflow")?; }
            else { return Err("tree_type"); }
        }
    }
    Ok(bytes)
}
fn fact_number(value: &Value, key: &str) -> Option<f64> {
    value[key].as_f64().filter(|value| value.is_finite() && *value >= 0.)
}
fn appearance_facts(app: &tauri::AppHandle) -> Value {
    let Some(view)=app.webviews().into_values().find(|v|v.label().starts_with("soop-original-")) else {return Value::Null;};
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    // Read only public style/asset geometry; no nickname, message body or ticket.
    let script=r#"(()=>{const rows=Array.from(document.querySelectorAll('.chatting-list-item[user-type]')).slice(-80);return{rows:rows.map(r=>{const a=r.querySelector('.message-container .username .author'),i=r.querySelector('.username .thumb img');let src=null;try{const u=new URL(i?.currentSrc||i?.src||'',location.href);if(u.protocol==='https:'&&u.hostname==='static.file.sooplive.com'&&u.pathname.startsWith('/spcon/'))src=u.href;}catch{}return{role:r.getAttribute('user-type'),paletteKey:a?.getAttribute('data-color')??null,color:a?getComputedStyle(a).color:null,image:src,width:i?.naturalWidth??0,height:i?.naturalHeight??0,renderWidth:i?.getBoundingClientRect().width??0,renderHeight:i?.getBoundingClientRect().height??0}}),staff:document.querySelectorAll('.bubble-container.notice').length,police:document.querySelectorAll('.bubble-container.warning').length}})()"#;
    if view.eval_with_callback(script,move|raw|{let _=tx.try_send((raw.len()<=65536).then_some(raw));}).is_err(){return Value::Null;}
    rx.recv_timeout(Duration::from_secs(3)).ok().flatten().and_then(|raw|serde_json::from_str(&raw).ok()).unwrap_or(Value::Null)
}
fn wait_terminal(media: &PlatformMediaService, id: &str, seconds: u64) -> Option<MediaJob> {
    let start = Instant::now();
    loop {
        let row = media.jobs(Some("soop")).into_iter().find(|job| job.job_id == id)?;
        if matches!(row.state.as_str(), "completed" | "failed" | "interrupted" | "cancelled") { return Some(row); }
        if start.elapsed() >= Duration::from_secs(seconds) { return Some(row); }
        thread::sleep(Duration::from_millis(250));
    }
}
fn inspect_saved(media: &PlatformMediaService, job: &MediaJob) -> Result<Value, &'static str> {
    let playback = media.open(&job.job_id).map_err(|_| "local_open")?;
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut cursor = Some("0".to_owned()); let mut rows = 0usize;
        let mut gifts = 0; let mut emotes = 0; let mut colors = 0; let mut personalcons = 0;
        let mut offsets = Vec::new(); let mut ids = BTreeSet::new();
        let mut unsupported = 0; let mut partial = false; let mut quality = String::new();
        loop {
            if Instant::now() >= deadline || rows > 20_000 { return Err("replay_budget"); }
            let page = media.replay_chat(&job.job_id, 0, cursor.as_deref(), 120).map_err(|_| "replay_page")?;
            if page.state == "indexing" { thread::sleep(Duration::from_millis(50)); continue; }
            unsupported = unsupported.max(page.unsupported_items); partial |= page.partial_file;
            quality.clone_from(&page.timeline_quality);
            for row in page.items {
                rows += 1;
                let appearance = serde_json::to_value(&row).map_err(|_| "row_shape")?;
                if let Some(id) = appearance["soop"]["personalcon"]["imageAssetId"].as_str() {
                    if id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
                        personalcons += 1; ids.insert(id.to_owned());
                    }
                }
                colors += usize::from(row.soop.as_ref().is_some_and(|meta| meta.nickname_color_key.is_some()));
                if row.soop_gift.is_some() { gifts += 1; }
                if row.soop_emoticon.is_some() { emotes += 1; }
                if row.soop_gift.is_some() || row.soop_emoticon.is_some() {
                    if offsets.len() < 64 { offsets.push(json!({"kind":if row.soop_gift.is_some(){"gift"}else{"ogq"},"timeMs":row.time_ms})); }
                    let value = serde_json::to_value(&row).map_err(|_| "row_shape")?;
                    for key in ["soopGift", "soopEmoticon"] {
                        if let Some(metadata) = value[key].as_object() {
                            for (name, value) in metadata {
                                if name.ends_with("AssetId") {
                                    if let Some(id) = value.as_str().filter(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())) { ids.insert(id.to_owned()); }
                                }
                            }
                        }
                    }
                }
            }
            cursor = page.cursor;
            if cursor.is_none() { break; }
        }
        if ids.len() > 256 { return Err("asset_read_budget"); }
        let mut assets = 0;
        for id in &ids {
            let asset = media.replay_chat_asset(&job.job_id, id).map_err(|_| "asset_read")?;
            assets += usize::from(asset.state == "ready" && asset.data_url.is_some());
        }
        Ok(json!({"rows":rows,"giftRows":gifts,"ogqRows":emotes,"nicknamePaletteRows":colors,"personalconRows":personalcons,
            "richEventOffsets":offsets,"referencedAssetIds":ids.len(),"readableLocalAssets":assets,
            "unsupportedItems":unsupported,"partialFile":partial,"timelineQuality":quality,
            "richObserved":gifts+emotes>0,"localLibraryReopened":true,"remoteAVVerified":false}))
    })();
    media.close(&playback.token);
    result
}

// Only a newly created isolated capture is exported. These exact production
// replay rows stay in ignored local evidence; message text never enters logs.
fn export_appearance(media: &PlatformMediaService, job: &MediaJob, root: &Path) -> Result<Value, &'static str> {
    use std::io::Write;
    fn asset_ids(value:&Value,ids:&mut BTreeSet<String>,depth:usize)->Result<(), &'static str>{
        if depth>16{return Err("appearance_export_depth");}
        match value {
            Value::Object(object)=>for(key,value)in object{
                if key=="assetId"||key.ends_with("AssetId"){
                    if let Some(id)=value.as_str().filter(|id|id.len()==64&&id.bytes().all(|b|b.is_ascii_hexdigit()&&!b.is_ascii_uppercase())){ids.insert(id.to_owned());}
                }
                asset_ids(value,ids,depth+1)?;
            },
            Value::Array(array)=>for value in array{asset_ids(value,ids,depth+1)?;},
            _=>{}
        }
        if ids.len()>64{return Err("appearance_export_asset_count");}Ok(())
    }
    let mut items = Vec::new();
    let mut envelope = Value::Null; let mut ids = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if Instant::now() >= deadline { return Err("appearance_export_deadline"); }
        let position_ms=(job.library.as_ref().map_or(0.,|v|v.duration)*1000.).floor() as u64;
        let page = media.replay_chat(&job.job_id,position_ms,None,120).map_err(|_| "appearance_export_page")?;
        if page.state == "indexing" { thread::sleep(Duration::from_millis(50)); continue; }
        if page.state != "ready" { return Err("appearance_export_state"); }
        let value = serde_json::to_value(&page).map_err(|_| "appearance_export_shape")?;
        if envelope.is_null() { envelope = value; }
        for row in page.items {
            let value = serde_json::to_value(row).map_err(|_| "appearance_export_row")?;
            asset_ids(&value,&mut ids,0)?;
            items.push(value);
            if items.len()>600 || ids.len()>64 { return Err("appearance_export_bound"); }
        }
        break;
    }
    let row_count=items.len(); envelope["items"]=json!(items); envelope["cursor"]=Value::Null;
    let mut assets=serde_json::Map::new(); let mut asset_bytes=0usize; let mut ready=0;
    for id in ids {
        let asset=media.replay_chat_asset(&job.job_id,&id).map_err(|_|"appearance_export_asset")?;
        asset_bytes=asset_bytes.saturating_add(asset.data_url.as_ref().map_or(0,String::len));
        if asset_bytes>4*1024*1024 { return Err("appearance_export_asset_bound"); }
        ready+=usize::from(asset.state=="ready"&&asset.data_url.is_some());
        assets.insert(id,serde_json::to_value(asset).map_err(|_|"appearance_export_asset_shape")?);
    }
    let data=serde_json::to_vec(&json!({"jobId":job.job_id,"durationSeconds":job.library.as_ref().map(|v|v.duration),"scope":"actual final native120-row page, not whole archive","page":envelope,"assets":assets})).map_err(|_|"appearance_export_json")?;
    if data.len()>6*1024*1024 { return Err("appearance_export_total_bound"); }
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(root.join("appearance-replay.json")).map_err(|_|"appearance_export_create")?;
    file.write_all(&data).map_err(|_|"appearance_export_write")?;
    Ok(json!({"rows":row_count,"readyLocalAssets":ready,"bytes":data.len(),"sha256":format!("{:x}",Sha256::digest(&data)),"source":"nativeReplayApi","remoteVerified":false}))
}
fn source_counts(directory: &Path) -> Result<Value, &'static str> {
    own_directory(directory)?;
    let mut services = BTreeMap::<u64, u64>::new(); let mut rows = 0u64; let mut bytes = 0usize; let mut hashes = Vec::new();
    for part in 0..512 {
        let name = if part == 0 { "chat.source.jsonl".into() } else { format!("chat.source.{part:06}.jsonl") };
        let path = directory.join(&name);
        if !path.exists() { break; }
        let metadata = fs::symlink_metadata(&path).map_err(|_| "source_metadata")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 8 * 1024 * 1024 { return Err("source_bound"); }
        #[cfg(windows)] { use std::os::windows::fs::MetadataExt; if metadata.file_attributes() & 0x400 != 0 { return Err("source_reparse"); } }
        bytes += metadata.len() as usize; if bytes > 16 * 1024 * 1024 { return Err("source_budget"); }
        let data = fs::read(&path).map_err(|_| "source_read")?;
        for line in data.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
            if line.len() > 512 * 1024 { return Err("source_line"); }
            let value: Value = serde_json::from_slice(line).map_err(|_| "source_json")?;
            rows += 1;
            if let Some(service) = value["extra"]["service"].as_u64().filter(|service| *service <= u16::MAX as u64) { *services.entry(service).or_default() += 1; }
        }
        hashes.push(json!({"file":name,"bytes":data.len(),"sha256":format!("{:x}",Sha256::digest(&data))}));
    }
    Ok(json!({"rows":rows,"bytesRead":bytes,"services":services,"sourceHashes":hashes}))
}
fn run(app: &tauri::AppHandle, root: &Path, channel: &str, broadcast: &str, record_seconds: u64) -> Value {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let ff = workspace.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let mut report = json!({"version":1,"source":channel,"expectedBroadcast":broadcast,"localOnly":true,
        "recordingLimitSeconds":record_seconds,"uploads":0,"automaticRulesChanged":false,"allArtifactsRetained":true,
        "desktopInput":false,"cloudServiceStarted":false,"status":"failed","remoteAVVerified":false});
    let media = match PlatformMediaService::new(root.join("data"), PlatformMediaTools {
        yt_dlp:workspace.join(".runtime/platform-tools/bin/yt-dlp.exe"),deno:workspace.join(".runtime/platform-tools/bin/deno.exe"),
        ffmpeg:ff.join("ffmpeg.exe"),ffprobe:ff.join("ffprobe.exe") }, None) {
        Ok(media) => media, Err(_) => { report["failureStage"]=json!("isolated_media_init"); return report; }
    };
    let host = app.state::<SoopOriginalHost>();
    host.set_tray_muted(app, true);
    let mut token = None; let mut recording = None;
    let attempt = (|| -> Result<(), &'static str> {
        if !media.jobs(None).is_empty() { return Err("catalog_not_empty"); }
        if fs2::available_space(root).map_err(|_| "free_space")? < FREE_RESERVE { return Err("disk_reserve"); }
        let first = host.open(app, media.clone(), channel.into(), uuid::Uuid::new_v4().to_string(), root.join("media")).map_err(|_| "source_open")?;
        token = Some(first.token.clone());
        if first.channel_id != channel || first.broadcast_id != broadcast { return Err("source_identity"); }
        report["sourceIdentityMatched"]=json!(true);
        host.viewport(app,&first.token,serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":720},"chat":{"x":970,"y":0,"width":320,"height":720},"visible":true,"muted":true,"occlusions":[]})).map_err(|_|"viewport_shape")?).map_err(|_|"viewport")?;
        let start = Instant::now(); let mut initial = None; let mut ready = false;
        while start.elapsed() < Duration::from_secs(60) {
            let snapshot = host.snapshot(&first.token).map_err(|_| "source_snapshot")?;
            if snapshot.channel_id != channel || snapshot.broadcast_id != broadcast { return Err("source_identity"); }
            if let Some(view) = app.webviews().values().find(|view| view.label().starts_with("soop-original-")) {
                let facts = super::inspect(view);
                if let (Some(time),Some(decoded)) = (fact_number(&facts,"currentTime"),fact_number(&facts,"decodedFrames")) {
                    let baseline = *initial.get_or_insert(time);
                    if time > baseline + 2. && decoded > 0. && snapshot.capture_ready && snapshot.chat_visible {
                        report["videoClockProgressed"]=json!(true);report["decodedFrames"]=json!(decoded);report["officialChatVisible"]=json!(true);ready=true;break;
                    }
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
        if !ready { return Err("source_not_ready"); }
        let started = Instant::now();
        host.record(app,&first.token,true).map_err(|_|"record_start")?;
        while started.elapsed() < Duration::from_secs(20) {
            let snapshot = host.snapshot(&first.token).map_err(|_| "record_ack")?;
            if let Some(job) = snapshot.recording_job { recording=Some(job);break; }
            thread::sleep(Duration::from_millis(250));
        }
        if recording.is_none() { return Err("record_ack_timeout"); }
        let mut stop_reason = "duration_limit";
        // Leave one second for the native stop request. This is a bounded
        // test deadline, not a new production recording-duration preference.
        let stop_at = Duration::from_secs(record_seconds - 1);
        while started.elapsed() < stop_at {
            let snapshot=host.snapshot(&first.token).map_err(|_|"record_snapshot")?;
            if snapshot.channel_id!=channel||snapshot.broadcast_id!=broadcast { return Err("source_changed"); }
            if snapshot.recording_job.is_none() { stop_reason="source_recording_ended";break; }
            if tree_bytes(&root.join("media"))? >= SOFT_BYTES { stop_reason="soft_disk_threshold";break; }
            if fs2::available_space(root).map_err(|_|"free_space")? < FREE_RESERVE { stop_reason="low_free_space";break; }
            thread::sleep(Duration::from_millis(250).min(stop_at.saturating_sub(started.elapsed())));
        }
        report["stopReason"]=json!(stop_reason);report["recordingWallSeconds"]=json!(started.elapsed().as_secs_f64());
        host.record(app,&first.token,false).map_err(|_|"normal_stop")?;
        if record_seconds == 30 { report["officialAppearance"]=appearance_facts(app); }
        let job=wait_terminal(&media,recording.as_deref().unwrap(),90).ok_or("record_job_missing")?;
        report["jobId"]=json!(job.job_id);report["jobState"]=json!(job.state);
        if job.state!="completed" { return Err("finalize_not_completed"); }
        let item=job.library.as_ref().ok_or("library_missing")?;
        report["media"]=json!({"bytes":item.bytes,"durationSeconds":item.duration,"sha256":item.sha256});
        report["chatArchive"]=serde_json::to_value(&item.chat_archive).map_err(|_|"chat_summary")?;
        let directory=root.join("media/SOOP").join(&job.job_id);
        report["sourceEvents"]=source_counts(&directory)?;
        report["replay"]=inspect_saved(&media,&job)?;
        if record_seconds == 30 {
            report["appearanceExport"]=export_appearance(&media,&job,root)?;
            let observed=report["replay"]["personalconRows"].as_u64().is_some_and(|n|n>0)
                &&report["replay"]["readableLocalAssets"]==report["replay"]["referencedAssetIds"]
                &&report["appearanceExport"]["readyLocalAssets"].as_u64().is_some_and(|n|n>0);
            report["capturedPersonalconObserved"]=json!(observed);
            report["status"]=json!(if observed{"local_personalcon_candidate"}else{"personalcon_unobserved"});
        } else {report["status"]=json!(if report["replay"]["richObserved"]==true {"local_rich_candidate"}else{"rich_unobserved"});}
        Ok(())
    })();
    if let Err(stage)=attempt {report["failureStage"]=json!(stage);}
    // Normal lifecycle owns every failure as well. Never terminate the process
    // before committed chunks and chat have had their bounded drain opportunity.
    if let Some(token)=token.as_ref() {let _=host.record(app,token,false);}
    host.shutdown(app);
    if let Some(id)=recording.as_ref() {
        if let Some(job)=wait_terminal(&media,id,15) { report["finalState"]=json!(job.state);report["jobId"]=json!(job.job_id); }
    }
    if let Some(token)=token.as_ref() {let _=host.close(app,token);}
    media.shutdown();
    let until=Instant::now()+Duration::from_secs(10);
    while !media.active_ids().is_empty()&&Instant::now()<until {thread::sleep(Duration::from_millis(100));}
    if !media.active_ids().is_empty() {
        report["remainingOwnedWork"]=json!(media.active_ids().len());
        report["normalDrainPending"]=json!(true);
        if let Ok(bytes)=serde_json::to_vec_pretty(&report){let _=fs::write(root.join("result.json"),bytes);}
        println!("SOOP_RICH_CAPTURE_QA normalDrainPending=true processRetained=true");
        // Retain the owner and process until its normal shutdown actually drains.
        // A timeout is evidence of unfinished work, never permission to kill it.
        while !media.active_ids().is_empty(){thread::sleep(Duration::from_secs(1));}
        report["normalDrainPending"]=json!(false);
    }
    report["remainingOwnedWork"]=json!(media.active_ids().len());
    report["finalMediaTreeBytes"]=tree_bytes(&root.join("media")).map(Value::from).unwrap_or(Value::Null);
    report
}
pub fn run_main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().count()!=2 {
        return Err("SOOP_RICH_QA_OPT_IN_REQUIRED".into());
    }
    let (channel,broadcast,record_seconds)=match std::env::var("ATSUMI_SOOP_RICH_CAPTURE_QA").as_deref() {
        Ok("ecvhao-298637823-local-only")=>(CHANNEL,BROADCAST,RECORD_SECONDS),
        Ok("suhee0051-298640273-local-only")=>("suhee0051","298640273",RECORD_SECONDS),
        Ok("hinqocorv-298642901-local-only")=>("hinqocorv","298642901",RECORD_SECONDS),
        Ok("cnsgkcnehd74-298652953-appearance-only")=>("cnsgkcnehd74","298652953",30),
        _=>return Err("SOOP_RICH_QA_OPT_IN_REQUIRED".into()),
    };
    let uuid=uuid::Uuid::new_v4().simple().to_string();
    let workspace=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let parent=workspace.join(".runtime/soop-original-qa");fs::create_dir_all(&parent)?;own_directory(&parent)?;
    let root=parent.join(&uuid);fs::create_dir(&root)?;fs::create_dir(root.join("media"))?;
    // Production host selects app_data_dir/soop-original-profile. A UUID app id
    // gives it a fresh real OS profile without changing process-wide APPDATA.
    let identifier=format!("local.atsumi.sooprichqa.{uuid}");
    let appdata=PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA_MISSING")?);
    own_directory(&appdata)?;let profile=appdata.join(&identifier);if profile.exists(){return Err("QA_PROFILE_EXISTS".into());}
    let mut context=tauri::generate_context!();context.config_mut().identifier=identifier;
    context.config_mut().app.windows.clear();context.config_mut().app.tray_icon=None;
    let outcome=Arc::new(Mutex::new(None));let shared=outcome.clone();let run_root=root.clone();let expected_profile=profile.clone();
    let app=tauri::Builder::default().manage(SoopOriginalHost::default()).setup(move|app|{
        if app.path().app_data_dir()?!=expected_profile {return Err("QA_PROFILE_PATH_MISMATCH".into());}
        tauri::window::WindowBuilder::new(app,"main").title("Atsumi · SOOP local capture verification").inner_size(1290.,720.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
        let handle=app.handle().clone();let root=run_root.clone();let outcome=shared.clone();
        thread::spawn(move||{
            let value=run(&handle,&root,channel,broadcast,record_seconds);
            let written=serde_json::to_vec_pretty(&value).ok().is_some_and(|bytes|fs::write(root.join("result.json"),bytes).is_ok());
            let success=value["status"]!="failed"&&written&&value["remainingOwnedWork"]==0;
            println!("SOOP_RICH_CAPTURE_QA status={} reportWritten={} artifactsRetained=true",if success{"finished"}else{"failed"},written);
            *outcome.lock().unwrap()=Some(success);handle.exit(if success{0}else{2});
        });Ok(())
    }).build(context)?;
    app.run_return(|_,_|{});
    fs::write(root.join("retained-locations.json"),serde_json::to_vec_pretty(&json!({"evidenceRoot":root,"freshBrowserProfile":profile,"allArtifactsRetained":true}))?)?;
    let finished=outcome.lock().unwrap().unwrap_or(false);
    if finished{Ok(())}else{Err("SOOP_RICH_QA_INCOMPLETE_ARTIFACTS_RETAINED".into())}
}

/// Exact preserved local fixture only. No Tauri app, source receiver or network.
#[cfg(windows)]
pub fn recover_main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{io::{Read, Seek, SeekFrom, Write}, os::windows::fs::OpenOptionsExt};
    const JOB: &str = "f2925714-6dea-4adf-9100-f940ab0da03b";
    if std::env::args().count() != 2 || std::env::var("ATSUMI_SOOP_RECOVERY_QA").as_deref() != Ok(JOB) {
        return Err("SOOP_RECOVERY_OPT_IN_REQUIRED".into());
    }
    fn digest(file: &fs::File) -> Result<String, &'static str> {
        let mut reader = file.try_clone().map_err(|_| "original_clone")?;
        reader.seek(SeekFrom::Start(0)).map_err(|_| "original_seek")?;
        let mut bytes = Vec::new();
        reader.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| "original_read")?;
        if bytes.len() > 32 * 1024 * 1024 { return Err("original_size"); }
        Ok(format!("{:x}", Sha256::digest(&bytes)))
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let root = workspace.join(".runtime/soop-original-qa/e22f683f88094914b3ddbf04223c52e2");
    let directory = root.join("media/SOOP").join(JOB);
    own_directory(&directory)?;
    for (name, expected) in [
        ("result.json", "252e6303da38a833b9d9c55a1290818ccd552327bc6817f34d545f144069984a"),
        ("data/platform-media/jobs/f2925714-6dea-4adf-9100-f940ab0da03b.json", "2b6b2b95be7da94e15c7133b78109f53163bd5e443cb450d59b046fb339e9d36"),
        ("media/SOOP/f2925714-6dea-4adf-9100-f940ab0da03b/encoded-owner.json", "2827ed82c064ccea9a354655ea61ac4541e1f10eec96ff3ebb4eb1cc84b9fca1"),
        ("media/SOOP/f2925714-6dea-4adf-9100-f940ab0da03b/encoded-segments.jsonl", "7ddde9fb6396797bbbef5178131135367466e7e531e57c05259bad879584aeff"),
        ("media/SOOP/f2925714-6dea-4adf-9100-f940ab0da03b/chat-status.final.json", "a8216c9d76724550ae65a16f51c8d4d900d66a18591a0ba73ee780a2db434481"),
    ] {
        let path = root.join(name); own_directory(&path)?;
        if expected.len() != 64 || digest(&fs::File::open(path)?)? != expected { return Err("SOOP_RECOVERY_SOURCE_CHANGED".into()); }
    }
    let mut pending = vec![directory.clone()]; let mut directories = Vec::new();
    let mut originals = Vec::new(); let mut total = 0u64;
    while let Some(path) = pending.pop() {
        own_directory(&path)?;
        directories.push(fs::OpenOptions::new().access_mode(0x80).share_mode(3)
            .custom_flags(0x02200000).open(&path)?);
        if directories.len() > 8 { return Err("SOOP_RECOVERY_DIRECTORY_BOUND".into()); }
        for entry in fs::read_dir(path)? {
            let path = entry?.path(); own_directory(&path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() { pending.push(path); continue; }
            if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 { return Err("SOOP_RECOVERY_FILE_BOUND".into()); }
            total = total.checked_add(metadata.len()).ok_or("original_overflow")?;
            if originals.len() >= 256 || total > 64 * 1024 * 1024 { return Err("SOOP_RECOVERY_TOTAL_BOUND".into()); }
            let file = fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000).open(&path)?;
            let sha = digest(&file)?;
            originals.push((path, file, metadata.len(), sha));
        }
    }
    let result_file = fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000).open(root.join("result.json"))?;
    originals.push((root.join("result.json"), result_file, 719, "252e6303da38a833b9d9c55a1290818ccd552327bc6817f34d545f144069984a".to_owned()));
    if originals.iter().filter(|(path, _, _, _)| path.extension().is_some_and(|extension| extension == "mp4")).count() != 10
        || directory.join("media.mp4").exists() { return Err("SOOP_RECOVERY_NOT_INITIAL_FAILED_FIXTURE".into()); }
    let ff = workspace.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let media = PlatformMediaService::new(root.join("data"), PlatformMediaTools {
        yt_dlp: root.join("never-extract.exe"), deno: root.join("never-runtime.exe"),
        ffmpeg: ff.join("ffmpeg.exe"), ffprobe: ff.join("ffprobe.exe"),
    }, None)?;
    let mut report = json!({"version":1,"jobId":JOB,"status":"failed","localOnly":true,"newCapture":false,
        "uploads":0,"desktopInput":false,"cloudServiceStarted":false,"automaticRulesChanged":false,
        "originalFailedResultPreserved":true,"remoteAVVerified":false,"finalizerIncludesProbeAndFullDecode":true});
    let started = Instant::now();
    let attempt = (|| -> Result<(), &'static str> {
        let jobs = media.jobs(None);
        if jobs.len() != 1 { return Err("catalog_scope"); }
        let job = &jobs[0];
        if job.job_id != JOB || job.state != "failed" || job.library.is_some() || job.revision != 14
            || job.reference.provider != "soop" || job.reference.kind != "live" || job.reference.external_id != CHANNEL
            || job.broadcast_id.as_deref() != Some(BROADCAST) || job.download_root != root.join("media") {
            return Err("job_identity");
        }
        media.retry(JOB, job.revision).map_err(|_| "retry_admission")?;
        let completed = wait_terminal(&media, JOB, 180).ok_or("job_missing")?;
        report["jobState"] = json!(completed.state);
        if completed.state != "completed" { return Err("finalization_not_completed"); }
        let item = completed.library.as_ref().ok_or("library_missing")?;
        report["productionProbeAndFullDecodePassed"] = json!(true);
        report["media"] = json!({"bytes":item.bytes,"durationSeconds":item.duration,"sha256":item.sha256});
        report["sourceEvents"] = source_counts(&directory)?;
        report["replay"] = inspect_saved(&media, &completed)?;
        report["status"] = json!("local_recovery_verified");
        Ok(())
    })();
    if let Err(stage) = attempt { report["failureStage"] = json!(stage); }
    media.shutdown();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !media.active_ids().is_empty() && Instant::now() < deadline { thread::sleep(Duration::from_millis(100)); }
    report["remainingOwnedWork"] = json!(media.active_ids().len());
    report["elapsedSeconds"] = json!(started.elapsed().as_secs_f64());
    let proofs: Vec<_> = originals.iter().map(|(path, file, bytes, sha)| {
        let unchanged = fs::metadata(path).is_ok_and(|metadata| metadata.len() == *bytes)
            && digest(file).is_ok_and(|actual| &actual == sha)
            && fs::File::open(path).ok().and_then(|fresh| digest(&fresh).ok()).as_ref() == Some(sha);
        json!({"file":path.strip_prefix(&root).unwrap_or(path).to_string_lossy(),"bytes":bytes,"sha256":sha,"unchanged":unchanged})
    }).collect();
    let unchanged = proofs.iter().all(|proof| proof["unchanged"] == true);
    report["originals"] = json!(proofs); report["originalHashesUnchanged"] = json!(unchanged);
    let success = report["status"] == "local_recovery_verified" && report["remainingOwnedWork"] == 0 && unchanged;
    report["passed"] = json!(success);
    let path = root.join(format!("recovery-result-{}.json", uuid::Uuid::new_v4()));
    let mut output = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    output.write_all(&serde_json::to_vec_pretty(&report)?)?; output.sync_all()?;
    println!("SOOP_RECOVERY_QA passed={success} originalHashesUnchanged={unchanged} uploads=0");
    if success { Ok(()) } else { Err("SOOP_RECOVERY_NOT_VERIFIED_ORIGINALS_RETAINED".into()) }
}

#[cfg(not(windows))]
pub fn recover_main() -> Result<(), Box<dyn std::error::Error>> {
    Err("SOOP_RECOVERY_WINDOWS_GUARDS_REQUIRED".into())
}
