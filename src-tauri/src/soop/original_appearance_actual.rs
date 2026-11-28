//! Opt-in official-control observation -> owned recording -> actual replay DTO.
//! No injected preferences, account profile, desktop input or cloud transmission.
use super::*;
use crate::interface::{AppQuitRequest, AppState};
use crate::platform_media::PlatformMediaService;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const FIELDS: [&str; 7] = [
    "theme",
    "fontLevel",
    "randomNicknameColor",
    "fanMessageColor",
    "showEmoticons",
    "animateEmoticons",
    "smallOgq",
];
const OBSERVE: &str = r#"(()=>{
 const area=document.getElementById('chatting_area'), viewer=document.getElementById('chat_area');
 const ancestors=[];for(const n of [area,viewer]){let p=n;for(let i=0;p&&i<16;i++,p=p.parentElement)if(!ancestors.includes(p))ancestors.push(p)}
 for(const n of [document.documentElement,document.body])if(n&&!ancestors.includes(n))ancestors.push(n);
 let theme=null;if(area&&viewer){if(ancestors.some(n=>n.getAttribute('dark')==='true'||['thema_dark','smode','chat_thema_dark'].some(c=>n.classList.contains(c))))theme='dark';else if(ancestors.some(n=>n.getAttribute('dark')==='false'))theme='light'}
 const level=area?.getAttribute('font-level');let saved={};try{const s=localStorage.getItem('chatOptionSetting');if(s&&s.length<=4096){const p=JSON.parse(s);if(p&&typeof p==='object'&&!Array.isArray(p))saved=p}}catch{}
 const bool=k=>typeof saved[k]==='boolean'?saved[k]:null;
 // Independently read the public computed values; do not use the observer,
 // native control, or replay DTO as this comparison's expected palette.
 let palette=null;
 if(viewer)try{
   const inherited=getComputedStyle(viewer);
   const canonical=raw=>{if(typeof raw!=='string'||raw.length>64)return null;const s=raw.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g,'');
     if(/^#[a-f\d]{6}$/i.test(s))return s.toLowerCase();
     const m=/^rgba?\(([\d,./\t\n\f\r ]+)\)$/i.exec(s);if(!m)return null;
     const syntax=m[1].includes(',')?/^[\t\n\f\r ]*\d{1,3}[\t\n\f\r ]*,[\t\n\f\r ]*\d{1,3}[\t\n\f\r ]*,[\t\n\f\r ]*\d{1,3}(?:[\t\n\f\r ]*,[\t\n\f\r ]*1(?:\.0+)?)?[\t\n\f\r ]*$/:/^[\t\n\f\r ]*\d{1,3}[\t\n\f\r ]+\d{1,3}[\t\n\f\r ]+\d{1,3}(?:[\t\n\f\r ]*\/[\t\n\f\r ]*1(?:\.0+)?)?[\t\n\f\r ]*$/;if(!syntax.test(m[1]))return null;
     const parts=m[1].split(m[1].includes(',')?',':/\s+|\//).map(v=>v.trim()).filter(Boolean);
     if(parts.length!==3&&parts.length!==4)return null;if(parts.length===4&&!/^1(?:\.0+)?$/.test(parts.pop()))return null;
     if(!parts.every(v=>/^\d{1,3}$/.test(v)&&Number(v)<=255))return null;
     return '#'+parts.map(v=>Number(v).toString(16).padStart(2,'0')).join('');};
   const read=name=>{try{return canonical(inherited.getPropertyValue(name))}catch{return null}};
   palette={version:1,basis:'computed-css-variable',nickname:Array.from({length:20},(_,i)=>read('--nickname-ramdom-color'+(i+1))),roles:Object.fromEntries(['normal','streamer','manager','vip','subscribe','fan','support','nightbot'].map(role=>[role,read('--'+role+'-color')]))};
 }catch{}
 const rows=[...document.querySelectorAll('.chatting-viewer .chatting-list-item,.chat_memoyo .chatting-list-item')].filter(n=>n.querySelector('.message-container .message-text .msg'));
 const row=rows.find(n=>n.querySelector('.username .author'));const msg=row?.querySelector('.message-container .message-text .msg');const style=msg?getComputedStyle(msg):null;
 const box=document.querySelector('#chatting_area .chatbox')||document.querySelector('#chatbox_height');
 const authors=rows.slice(-20).map(n=>{const a=n.querySelector('.username .author');if(!a)return null;const classes=[...a.classList].filter(c=>/^(?:random-color\d+|nickname.*)$/.test(c)).slice(0,8);return {classes,role:n.getAttribute('user-type'),palette:classes.find(c=>/^random-color\d+$/.test(c))??null,color:getComputedStyle(a).color}}).filter(Boolean);
 const v=document.getElementById('livePlayer'),q=v?.getVideoPlaybackQuality?.();
 const shape=n=>{if(!n)return null;const s=getComputedStyle(n),r=n.getBoundingClientRect();return {tag:n.tagName,classes:[...n.classList].filter(c=>/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(c)).slice(0,12),children:n.childElementCount,display:s.display,visibility:s.visibility,opacity:s.opacity,rect:{x:r.x,y:r.y,width:r.width,height:r.height},backgroundColor:s.backgroundColor,hasBackgroundImage:s.backgroundImage!=='none'}};
 const fixedNodes=Object.fromEntries(['webplayer','chatting_area','chatbox','chatbox_height','chat_area'].map(id=>[id,shape(document.getElementById(id))]));
 const counts=Object.fromEntries(['#chat_area > *','#chat_area .chatting-list-item','#chat_area .msg','#chatting_area .chatting-list-item','#chatting_area .message-container .message-text .msg','.chatting-list-item'].map(s=>[s,document.querySelectorAll(s).length]));
 const backgroundAncestors=[];let effectiveBackgroundColor=null,complexBackground=false;
 for(let n=viewer,i=0;n&&i<12;n=n.parentElement,i++){
   const d=shape(n),css=getComputedStyle(n);backgroundAncestors.push(d);
   if(Number(d.opacity)!==1||d.display==='none'||d.visibility!=='visible'||css.filter!=='none'||css.backdropFilter!=='none'||css.mixBlendMode!=='normal')complexBackground=true;
   if(effectiveBackgroundColor!==null)continue;
   if(d.hasBackgroundImage)complexBackground=true;
   const m=/^rgba?\(\s*([\d.]+)[, ]+([\d.]+)[, ]+([\d.]+)(?:\s*[,/]\s*([\d.]+))?\s*\)$/.exec(d.backgroundColor);
   if(!m){complexBackground=true;continue}const alpha=m[4]===undefined?1:Number(m[4]);
   if(alpha>0&&alpha<1)complexBackground=true;
   if(alpha===1&&!complexBackground)effectiveBackgroundColor=d.backgroundColor;
 }
 if(complexBackground)effectiveBackgroundColor=null;
 const hex=raw=>{const m=/^rgb\(\s*(\d+),\s*(\d+),\s*(\d+)\s*\)$/.exec(raw??'');return m&&m.slice(1).every(v=>+v<=255)?'#'+m.slice(1).map(v=>(+v).toString(16).padStart(2,'0')).join(''):null};
 const ordinaryColors=[];
 for(const row of [...(viewer?.children??[])].filter(n=>n.classList.contains('chatting-list-item')).slice(-32).reverse()){
   if(ordinaryColors.length>=3)break;
   if(row.querySelector('.police,.staff,.balloon,.gift_box')||['fan','staff','police'].includes(row.getAttribute('user-type')))continue;
   const n=row.querySelector(':scope > .message-container > .message-text > .msg');if(!n||n.style.color||n.parentElement.style.color)continue;
   const css=getComputedStyle(n),rect=n.getBoundingClientRect();if(css.display==='none'||css.visibility!=='visible'||css.opacity!=='1'||rect.width<=0||rect.height<=0)continue;
   ordinaryColors.push(hex(css.color));
 }
 const foreground=ordinaryColors.length&&ordinaryColors.every(v=>v!==null&&v===ordinaryColors[0])?ordinaryColors[0]:null;
 const surface={version:1,basis:'computed-dom',foreground,background:hex(effectiveBackgroundColor)};
 const lastRows=[...(viewer?.querySelectorAll('.chatting-list-item')??[])].slice(-3);
 const structural=n=>({tag:n.tagName,classes:[...n.classList].filter(c=>/^[A-Za-z][A-Za-z0-9_-]{0,39}$/.test(c)).slice(0,3)});
 let treeNodes=0,treeBytes=0,treeTruncated=false;
 const tree=(n,depth)=>{const v=structural(n),size=JSON.stringify(v).length+16;if(treeNodes>=100||treeBytes+size>6000){treeTruncated=true;return null}treeNodes++;treeBytes+=size;v.children=[];if(depth<4){for(const c of n.children){const child=tree(c,depth+1);if(child)v.children.push(child);else break}}else if(n.children.length)treeTruncated=true;return v};
 const rowTrees=lastRows.map(n=>tree(n,0));
 const messageStyles=lastRows.flatMap(n=>[...n.querySelectorAll('.msg')]).slice(0,6).map(n=>{const s=getComputedStyle(n),parents=[];for(let p=n.parentElement,i=0;p&&p!==viewer&&i<5;p=p.parentElement,i++)parents.push(structural(p));return {node:structural(n),fontSize:s.fontSize,lineHeight:s.lineHeight,color:s.color,ancestors:parents}});
 const diagnostics={documentHidden:document.hidden,visibilityState:document.visibilityState,chatOpen:document.getElementById('webplayer')?.classList.contains('chat_open')??null,fixedNodes,counts,backgroundAncestors,complexBackground,areaChildren:[...(viewer?.children??[])].slice(0,8).map(shape),rowTrees,treeNodes,treeTruncated,messageStyles,authorCount:viewer?.querySelectorAll('.author').length??0,lastRowAuthorCounts:lastRows.map(n=>n.querySelectorAll('.author').length)};
 return {theme,fontLevel:/^[1-8]$/.test(level??'')?Number(level):null,randomNicknameColor:viewer?viewer.classList.contains('random-color-on'):null,fanMessageColor:bool('st1_fanchatcolor'),showEmoticons:bool('st1_emoticon'),animateEmoticons:bool('st1_emoticon_ani'),smallOgq:bool('st1_ogq'),palette,surface,ordinarySurfaceSamples:ordinaryColors.length,backgroundColor:box?getComputedStyle(box).backgroundColor:null,effectiveBackgroundColor,fontSize:style?.fontSize??null,lineHeight:style?.lineHeight??null,authors,ordinaryRows:rows.length,themeTogglePresent:!!document.querySelector('#modecheck,#modecheck2'),diagnostics,video:{time:v?.currentTime,ready:v?.readyState,frames:q?.totalVideoFrames,rate:v?.playbackRate,muted:v?.muted,volume:v?.volume}};
})()"#;

fn eval(app: &AppHandle, script: &str) -> Result<Value, String> {
    let views: Vec<_> = app
        .webviews()
        .into_values()
        .filter(|v| v.label().starts_with("soop-original-"))
        .collect();
    if views.len() != 1 {
        return Err("receiver_count".into());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    views[0]
        .eval_with_callback(script, move |v| {
            let _ = tx.send(v);
        })
        .map_err(|_| "inspect_dispatch")?;
    let text = rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "inspect_timeout")?;
    if text.len() > 24 * 1024 {
        return Err("inspect_bound".into());
    }
    serde_json::from_str(&text).map_err(|_| "inspect_shape".into())
}
fn observe(app: &AppHandle) -> Result<Value, String> {
    eval(app, OBSERVE)
}
fn mute_own_video(app: &AppHandle) -> Result<(), String> {
    let applied = eval(
        app,
        r#"(()=>{
      const v=document.getElementById('livePlayer');if(!(v instanceof HTMLVideoElement))return {applied:false};
      const key='__atsumiAppearanceQaMuteGuard';let guard=v[key];
      if(guard===undefined){
        guard=()=>{if(v.volume!==0)v.volume=0;if(!v.muted)v.muted=true};
        Object.defineProperty(v,key,{value:guard});v.addEventListener('volumechange',guard);
      }else if(typeof guard!=='function')return {applied:false};
      guard();return {applied:true,muted:v.muted,volume:v.volume};
    })()"#,
    )?;
    if applied["applied"] != true || applied["muted"] != true || applied["volume"] != 0.0 {
        return Err("qa_mute_not_applied".into());
    }
    Ok(())
}
fn require_muted(observed: &Value) -> Result<(), String> {
    if observed["video"]["muted"] != true || observed["video"]["volume"] != 0.0 {
        return Err("qa_mute_not_maintained".into());
    }
    Ok(())
}
fn values(observed: &Value) -> Value {
    Value::Object(
        FIELDS
            .into_iter()
            .map(|k| (k.to_string(), observed[k].clone()))
            .collect(),
    )
}

fn set_phase(app: &AppHandle, second: bool, toggle_theme: bool) -> Result<Value, String> {
    // Only official event handlers write preferences. Hidden checkbox defaults
    // become usable only after opening the original settings menu.
    let script = format!(
        r#"(()=>{{
      const second={second},toggleTheme={toggle_theme};
      const click=s=>{{const e=document.querySelector(s);if(!e)throw Error('missing_control');e.click();return e}};
      for(const close of document.querySelectorAll('.chat_layer.on > .close'))close.click();
      click('#setbox_set a');
      const desired=second?5:3;
      for(let i=0;i<8;i++){{const current=Number(document.getElementById('chatting_area')?.getAttribute('font-level'));if(current===desired)break;if(!(current>=1&&current<=8))throw Error('font_missing');click('.chat_layer.setting_chat .font-level '+(current<desired?'.plus':'.minus'))}}
      const set=(id,want)=>{{const input=document.getElementById(id);if(!input||input.type!=='checkbox')throw Error('option_missing');const li=input.closest('li');if(!li)throw Error('option_parent');if(input.checked===want){{li.click();li.click()}}else li.click();if(input.checked!==want)throw Error('option_not_applied')}};
      click('.chat_layer.setting_chat a.more_layer[data-title="person"]');set('st1_randomnickname',!second);set('st1_fanchatcolor',!second);
      click('.chat_layer.sub.person .history_back');click('.chat_layer.setting_chat a.more_layer[data-title="mark"]');
      set('st1_emoticon',!second);set('st1_emoticon_ani',!second);set('st1_ogq',second);
      click('.chat_layer.sub.mark > .close');
      let themeClicked=false;if(toggleTheme){{const t=document.querySelector('#modecheck,#modecheck2');if(t&&t instanceof HTMLInputElement&&t.type==='checkbox'){{t.click();themeClicked=true}}}}
      return {{applied:true,themeClicked}};
    }})()"#
    );
    let action = eval(app, &script)?;
    if action["applied"] != true {
        return Err("official_controls_failed".into());
    }
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let mut seen = observe(app)?;
        require_muted(&seen)?;
        if seen["fontLevel"] == if second { 5 } else { 3 }
            && seen["randomNicknameColor"] == !second
            && seen["fanMessageColor"] == !second
            && seen["showEmoticons"] == !second
            && seen["animateEmoticons"] == !second
            && seen["smallOgq"] == second
        {
            seen["themeClicked"] = action["themeClicked"].clone();
            return Ok(seen);
        }
        if Instant::now() >= until {
            return Err("official_controls_unobserved".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn no_links(path: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    if !path.is_absolute() {
        return Err("path_not_absolute".into());
    }
    for p in path.ancestors() {
        if let Ok(m) = fs::symlink_metadata(p) {
            if m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0 {
                return Err("linked_path".into());
            }
        }
    }
    Ok(())
}
fn read(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    no_links(path)?;
    let m = fs::metadata(path).map_err(|_| "evidence_metadata")?;
    if !m.is_file() || m.len() > max {
        return Err("evidence_bound".into());
    }
    let bytes = fs::read(path).map_err(|_| "evidence_read")?;
    if bytes.len() as u64 > max {
        return Err("evidence_grew".into());
    }
    Ok(bytes)
}
fn write(path: &Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    no_links(path)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "evidence_encode")?;
    if bytes.len() > 24 * 1024 * 1024 {
        return Err("export_bound".into());
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "evidence_create")?;
    f.write_all(&bytes)
        .and_then(|_| f.sync_all())
        .map_err(|_| "evidence_write".into())
}
fn controls(directory: &Path) -> Result<Vec<Value>, String> {
    let mut found = Vec::new();
    let mut total = 0usize;
    for i in 0..4 {
        let path = directory.join(if i == 0 {
            "chat.events.jsonl".to_string()
        } else {
            format!("chat.events.{i:06}.jsonl")
        });
        if !path.exists() {
            break;
        }
        let bytes = read(&path, 8 * 1024 * 1024)?;
        total += bytes.len();
        if total > 16 * 1024 * 1024 {
            return Err("events_bound".into());
        }
        for line in bytes.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
            if line.len() > 512 * 1024 {
                return Err("event_line_bound".into());
            }
            let v: Value = serde_json::from_slice(line).map_err(|_| "event_shape")?;
            if v["format"] == "atsumi-soop-display-v1" {
                found.push(v);
                if found.len() > 128 {
                    return Err("control_bound".into());
                }
            }
        }
    }
    Ok(found)
}
fn page(media: &PlatformMediaService, job: &str, position: u64) -> Result<Value, String> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let value = serde_json::to_value(
            media
                .replay_chat(job, position, None, 120)
                .map_err(|_| "replay_read")?,
        )
        .map_err(|_| "replay_shape")?;
        if value["state"] != "preparing" && value["state"] != "indexing" {
            return Ok(value);
        }
        if Instant::now() >= until {
            return Err("replay_preparation_timeout".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}
fn asset_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if (matches!(k.as_str(), "assetId" | "imageAssetId" | "fallbackAssetId")
                    || k.ends_with("ImageAssetId"))
                    && v.as_str()
                        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                {
                    ids.insert(v.as_str().unwrap().to_owned());
                } else {
                    asset_ids(v, ids)
                }
            }
        }
        Value::Array(rows) => {
            for v in rows {
                asset_ids(v, ids)
            }
        }
        _ => {}
    }
}
fn wait_recording(app: &AppHandle, deadline: Instant, seconds: u64) -> Result<Value, String> {
    let begin = observe(app)?;
    require_muted(&begin)?;
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        if Instant::now() >= deadline {
            return Err("source_budget".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let end = observe(app)?;
    require_muted(&end)?;
    if !end["video"]["time"]
        .as_f64()
        .zip(begin["video"]["time"].as_f64())
        .is_some_and(|(a, b)| a > b + 1.)
        || !end["video"]["frames"]
            .as_u64()
            .zip(begin["video"]["frames"].as_u64())
            .is_some_and(|(a, b)| a > b)
        || end["video"]["rate"] != 1.0
    {
        return Err("source_stalled".into());
    }
    Ok(end)
}
fn exercise(app: &AppHandle, root: &Path, report: &mut Value) -> Result<(), String> {
    let startup = app.state::<Arc<crate::startup::Startup>>();
    crate::initialize_backend(app, &startup).map_err(|_| "backend_init")?;
    let state = app.state::<AppState>();
    let media = state.platform_media(app).map_err(|_| "media_init")?;
    if !media.jobs(None).is_empty() {
        return Err("profile_not_empty".into());
    }
    let host = app.state::<SoopOriginalHost>();
    host.set_tray_muted(app, true);
    let channel = std::env::var("ATSUMI_SOOP_APPEARANCE_CHANNEL").unwrap_or_else(|_| "ioioiobb".into());
    if !policy::valid_channel(&channel) { return Err("qa_channel_invalid".into()); }
    report["source"] = json!(channel);
    let opened = host
        .open(
            app,
            media.clone(),
            channel,
            uuid::Uuid::new_v4().to_string(),
            root.join("media"),
        )
        .map_err(|error| match error.as_str() {
            "방송 중인 채널이 아닙니다." => "source_offline",
            "채널을 찾을 수 없습니다." => "source_not_found",
            "SOOP에 연결하지 못했습니다. 잠시 후 다시 시도해 주세요." => "source_connection",
            _ => "source_open",
        })?;
    let token = opened.token;
    let source_deadline = Instant::now() + Duration::from_secs(65);
    host.viewport(app,&token,serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":720},"chat":{"x":970,"y":0,"width":320,"height":720},"visible":true,"muted":true,"occlusions":[]})).map_err(|_|"viewport_shape")?).map_err(|_|"viewport")?;
    let ready = Instant::now() + Duration::from_secs(25);
    loop {
        let s = host.snapshot(&token).map_err(|_| "source_snapshot")?;
        if s.capture_ready
            && s.chat_visible
            && observe(app)?["video"]["ready"].as_u64().unwrap_or(0) >= 3
        {
            break;
        }
        if Instant::now() >= ready {
            return Err("source_not_ready".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
    host.set_tray_muted(app, false);
    host.resume(app, &token).map_err(|_| "source_resume")?;
    mute_own_video(app)?;
    let initial = set_phase(app, false, false)?;
    report["initialObserved"] = initial.clone();
    host.record(app, &token, true).map_err(|_| "record_start")?;
    let started = Instant::now() + Duration::from_secs(8);
    let job = loop {
        if let Some(j) = host
            .snapshot(&token)
            .map_err(|_| "record_snapshot")?
            .recording_job
        {
            break j;
        }
        if Instant::now() >= started || Instant::now() >= source_deadline {
            return Err("record_start_timeout".into());
        }
        thread::sleep(Duration::from_millis(100));
    };
    let a = wait_recording(app, source_deadline, 7)?;
    let b_action = set_phase(app, true, true)?;
    let b = wait_recording(app, source_deadline, 8)?;
    let c_action = set_phase(app, false, b_action["themeClicked"] == true)?;
    let c = wait_recording(app, source_deadline, 8)?;
    host.record(app, &token, false).map_err(|_| "record_stop")?;
    report["observedPhases"] = json!([a.clone(), b.clone(), c.clone()]);
    report["muteSamplesVerified"] = json!(true);
    report["mutePolicy"] = json!("qa-owned-video-muted-and-volume-zero");
    report["themeSwitchObserved"] = json!(
        b_action["themeClicked"] == true
            && c_action["themeClicked"] == true
            && a["theme"] != b["theme"]
            && a["theme"] == c["theme"]
    );
    report["recordingJob"] = json!(job);
    let until = Instant::now() + Duration::from_secs(60);
    let completed = loop {
        let j = media
            .jobs(Some("soop"))
            .into_iter()
            .find(|j| j.job_id == job)
            .ok_or("record_missing")?;
        if matches!(
            j.state.as_str(),
            "completed" | "failed" | "interrupted" | "cancelled"
        ) {
            break j;
        }
        if Instant::now() >= until {
            return Err("record_finish_timeout".into());
        }
        thread::sleep(Duration::from_millis(250));
    };
    report["recordingState"] = json!(completed.state);
    if completed.state != "completed" || media.jobs(None).len() != 1 {
        return Err("record_not_completed".into());
    }
    let item = completed.library.as_ref().ok_or("library_missing")?;
    report["captureOutcome"] =
        serde_json::to_value(&item.capture_outcome).map_err(|_| "capture_shape")?;
    let duration = item.duration;
    let duration_ms = (duration * 1000.).floor() as u64;
    let directory = root.join("media/SOOP").join(&job);
    no_links(&directory)?;
    let timeline: Value =
        serde_json::from_slice(&read(&directory.join("chat-timeline.json"), 64 * 1024)?)
            .map_err(|_| "timeline_shape")?;
    let origin = timeline["firstSegmentCommittedAtMs"]
        .as_u64()
        .zip(timeline["firstSegmentDurationMs"].as_u64())
        .and_then(|(end, d)| {
            if (1..=120000).contains(&d) {
                end.checked_sub(d)
            } else {
                None
            }
        })
        .ok_or("timeline_missing")?;
    let controls = controls(&directory)?;
    let mut chosen = Vec::new();
    let mut previous = 0;
    for (i, observed) in [&a, &b, &c].into_iter().enumerate() {
        let expected = values(observed);
        let expected_palette = &observed["palette"];
        let nickname = expected_palette["nickname"]
            .as_array()
            .ok_or("palette_unobserved")?;
        let roles = expected_palette["roles"]
            .as_object()
            .ok_or("palette_unobserved")?;
        if nickname.len() != 20
            || roles.len() != 8
            || !nickname.iter().chain(roles.values()).all(|v| {
                v.as_str().is_some_and(|s| {
                    s.len() == 7
                        && s.starts_with('#')
                        && s.as_bytes()[1..]
                            .iter()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
                })
            })
        {
            return Err("palette_partly_unobserved".into());
        }
        let control = controls
            .iter()
            .find(|v| {
                v["values"] == expected
                    && v["palette"] == *expected_palette
                    && v["surface"] == observed["surface"]
                    && v["receivedAtMs"]
                        .as_u64()
                        .is_some_and(|t| i == 0 || t > previous)
            })
            .ok_or("observed_control_missing")?;
        previous = control["receivedAtMs"].as_u64().ok_or("control_clock")?;
        // Sample near each observed phase end so ordinary chat, when present,
        // is rendered too. A separate zero query checks the initial snapshot.
        let next_change = controls
            .iter()
            .filter(|v| v["values"] != expected || v["palette"] != *expected_palette || v["surface"] != observed["surface"])
            .filter_map(|v| v["receivedAtMs"].as_u64())
            .filter(|t| *t > previous)
            .min();
        let position = next_change
            .map(|next| next.saturating_sub(origin).saturating_sub(250))
            .unwrap_or_else(|| duration_ms.saturating_sub(50));
        if position > duration_ms {
            return Err("control_outside_video".into());
        }
        let p = page(&media, &job, position)?;
        if p["soopDisplay"]["values"] != expected || p["soopDisplayIncomplete"] == true {
            return Err("replay_settings_mismatch".into());
        }
        if p["soopDisplay"]["palette"] != *expected_palette
            || p["soopDisplay"]["paletteIncomplete"] == true
        {
            return Err("replay_palette_mismatch".into());
        }
        if p["soopDisplay"]["surface"] != observed["surface"] || p["soopDisplay"]["surfaceIncomplete"] == true {
            return Err("replay_surface_mismatch".into());
        }
        chosen.push(json!({"label":(["initial","changed","restored"][i]),"positionMs":position,"observedDom":observed,"page":p}));
    }
    let initial_page = page(&media, &job, 0)?;
    if initial_page["soopDisplay"]["values"] != values(&a)
        || initial_page["soopDisplay"]["palette"] != a["palette"]
        || initial_page["soopDisplay"]["paletteIncomplete"] == true
    {
        return Err("initial_settings_mismatch".into());
    }
    for index in [1usize, 0, 2] {
        let pos = chosen[index]["positionMs"].as_u64().ok_or("position")?;
        let again = page(&media, &job, pos)?;
        if again["soopDisplay"] != chosen[index]["page"]["soopDisplay"] {
            return Err("backward_settings_mismatch".into());
        }
    }
    let opened = media.open(&job).map_err(|_| "replay_open")?;
    media.close(&opened.token);
    if page(&media, &job, chosen[1]["positionMs"].as_u64().unwrap())?["soopDisplay"]
        != chosen[1]["page"]["soopDisplay"]
    {
        return Err("reopen_settings_mismatch".into());
    }
    let mut ids = BTreeSet::new();
    for s in &chosen {
        asset_ids(&s["page"], &mut ids)
    }
    if ids.len() > 128 {
        return Err("asset_count_bound".into());
    }
    let mut assets = serde_json::Map::new();
    let mut asset_bytes = 0;
    for id in ids {
        let read = media
            .replay_chat_asset(&job, &id)
            .map_err(|_| "local_asset_read")?;
        let asset = json!({"state":read.state,"dataUrl":read.data_url});
        asset_bytes += asset["dataUrl"].as_str().map_or(0, str::len);
        if asset_bytes > 8 * 1024 * 1024 {
            return Err("asset_export_bound".into());
        }
        assets.insert(id, asset);
    }
    let mut row_settings = std::collections::BTreeMap::new();
    let mut creation_modes = BTreeSet::new();
    for s in &chosen {
        for row in s["page"]["items"].as_array().ok_or("page_items")? {
            if row["soopDisplay"].is_object() {
                let id = row["id"].as_str().ok_or("row_id")?;
                let creation = row["soopDisplay"].clone();
                if row_settings
                    .insert(id.to_string(), creation.clone())
                    .is_some_and(|old| old != creation)
                {
                    return Err("past_row_settings_rewritten".into());
                }
                if let Some(show) = creation["showEmoticons"].as_bool() {
                    creation_modes.insert(show);
                }
            }
        }
    }
    report["displayControlCount"] = json!(controls.len());
    report["rowCreationSettingsObserved"] = json!(row_settings.len());
    report["bothCreationModesObserved"] = json!(creation_modes.len() == 2);
    report["backwardAndReopenVerified"] = json!(true);
    report["paletteValuesVerifiedPerSnapshot"] = json!(28);
    report["surfaceValuesComparedPerSnapshot"] = json!(2);
    report["surfaceForegroundObserved"] = json!([&a,&b,&c].iter().any(|v| v["surface"]["foreground"].is_string()));
    report["durationSeconds"] = json!(duration);
    write(
        &root.join("appearance-export.json"),
        &json!({"jobId":job,"durationSeconds":duration,"snapshots":chosen,"assets":assets,"qualifications":{"timing":"receive-time-approximate","themeSwitchObserved":report["themeSwitchObserved"],"relayConcurrentCapture":"separate-native-tests","onlyNewOwnedRecording":true}}),
    )?;
    host.close(app, &token).map_err(|_| "source_close")?;
    Ok(())
}

#[test]
#[ignore = "Explicit fresh-profile official SOOP settings recording; no account or cloud changes"]
fn actual_official_settings_recording_and_replay() {
    assert_eq!(
        std::env::var("ATSUMI_SOOP_APPEARANCE_QA").as_deref(),
        Ok("official-settings-fresh-profile"),
        "QA_OPT_IN"
    );
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let root = workspace
        .join(".runtime/soop-appearance-native-qa")
        .join(&run);
    no_links(&root).unwrap();
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("media")).unwrap();
    let identity = format!("local.atsumi.soopappearanceqa.{run}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    no_links(&profile).unwrap();
    assert!(!profile.exists(), "QA_PROFILE_EXISTS");
    let result = Arc::new(Mutex::new(json!({"passed":false})));
    let output = result.clone();
    let worker_root = root.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = identity;
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app=tauri::Builder::default().any_thread().manage(SoopOriginalHost::default()).manage(crate::youtube::original::YouTubeOriginalHost::default()).manage(Arc::new(crate::startup::Startup::default())).manage(Arc::new(crate::ui_download_events::DownloadEvents::default()))
      .setup(move|app|{
        if app.path().app_data_dir()?!=profile{return Err("QA_PROFILE_PATH".into())}no_links(&profile)?;
        tauri::window::WindowBuilder::new(app,"main").title("Atsumi official appearance verification").inner_size(1290.,720.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
        let handle=app.handle().clone();thread::spawn(move||{
          let mut report=json!({"passed":false,"isolated":true,"noCloud":true,"noDesktopInput":true,"source":"ioioiobb","freshDataPreserved":true});
          let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||exercise(&handle,&worker_root,&mut report))).unwrap_or_else(|_|Err("qa_worker_panic".into()));
          report["passed"]=json!(outcome.is_ok());report["error"]=json!(outcome.err());*lock(&output)=report;
          if let Some(state)=handle.try_state::<AppState>(){
            if let Ok(snapshot)=state.active_work_snapshot(){if state.request_graceful_quit(handle.clone(),AppQuitRequest{expected_work_set_fingerprint:snapshot.work_set_fingerprint,confirm_active_work:true,force_when_status_unknown:false}).is_ok_and(|v|v.accepted){return}}
            lock(&output)["passed"]=json!(false);lock(&output)["quitError"]=json!("normal_quit_rejected");let _=write(&worker_root.join("pending-result.json"),&lock(&output));
          }else{handle.exit(2)}
        });Ok(())
      }).build(context).expect("QA_BUILD");
    app.run_return(|_, _| {});
    let mut report = lock(&result).clone();
    report["normalAppExitReturned"] = json!(true);
    write(&root.join("result.json"), &report).unwrap();
    println!(
        "QA_APPEARANCE_RESULT={}",
        root.join("result.json").display()
    );
    assert_eq!(
        report["passed"], true,
        "Actual settings recording remains unverified; inspect result"
    );
}

#[path = "original_mission_actual.rs"]
mod mission_actual;
