//! Opt-in public provider verification. Never accepts an arbitrary input path.
//! Only the explicitly approved agent-created neutral-shapes.jpg fixture is sent.
//! The normal provider profile namespace is deliberately reused without copying
//! cookies; this harness registers no X/data/plugin/IPC services.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tauri::Manager;

#[derive(Serialize)]
struct SearchLink {
    label: String,
    url: String,
}
#[derive(Serialize)]
struct Candidate {
    title: String,
    creator: Option<String>,
    similarity: Option<f64>,
    links: Vec<SearchLink>,
}
#[allow(dead_code)]
enum ProviderInput {
    ImageUrl(String),
    Jpeg(Vec<u8>),
}
struct ProviderRequest {
    job_id: String,
    input: ProviderInput,
}
#[path = "../src/x_media_search_provider.rs"]
mod provider;

const CONTROL: &str = "x-search-probe-control";
const CONTROL_HTML: &str = r#"<head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'"><style>body{background:#101014;color:#eee;font:16px system-ui;padding:28px;line-height:1.6}button{padding:12px 18px;margin-right:12px;font:inherit}p{max-width:650px}</style></head><body><h2>검색 서비스 직접 확인</h2><p id="message"></p><button id="continue" disabled>확인 완료 · 계속</button><button id="cancel">취소</button></body>"#;
const CONTROL_SCRIPT: &str = r#"(()=>{
  if(location.href!=='about:blank')return {ready:false};
  const config=__CONFIG__;
  if(!document.documentElement)document.appendChild(document.createElement('html'));
  if(document.documentElement.dataset.atsumiProbeControl!=='1'){
    document.documentElement.innerHTML=config.html;
    document.documentElement.dataset.atsumiProbeControl='1';
  }
  const next=document.getElementById('continue'),cancel=document.getElementById('cancel'),message=document.getElementById('message');
  if(!next||!cancel||!message)return {ready:false};
  message.textContent=config.message;next.disabled=!config.waiting;
  next.onclick=event=>{if(!event.isTrusted||next.disabled)return;event.preventDefault();next.disabled=true;message.textContent='현재 결과를 확인하는 중입니다.';location.href=config.continueUrl;};
  cancel.onclick=event=>{if(!event.isTrusted)return;event.preventDefault();cancel.disabled=true;next.disabled=true;message.textContent='취소하는 중입니다.';location.href=config.cancelUrl;};
  return {ready:true,version:1,waiting:config.waiting};
})()"#;
fn control_script(continue_url: &str, cancel_url: &str, waiting: bool, message: &str) -> String {
    let config = serde_json::json!({"html":CONTROL_HTML,"continueUrl":continue_url,"cancelUrl":cancel_url,"waiting":waiting,"message":message});
    CONTROL_SCRIPT.replace("__CONFIG__", &config.to_string())
}
fn phase(stage: &'static str) {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    if COUNT
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 32).then_some(n + 1)
        })
        .is_ok()
    {
        eprintln!("{}", serde_json::json!({"searchProbeControl":stage}));
    }
}
fn redact_evidence(mut value: serde_json::Value) -> serde_json::Value {
    if matches!(value.get("state").and_then(serde_json::Value::as_str),Some("challenge"|"consent")) {
        // Replace the whole document so new diagnostics cannot accidentally
        // retain challenge IPs, query tokens or account-specific text.
        value["document"] = serde_json::json!({"classification":"Human verification or consent required; document omitted."});
    }
    value
}
fn prepare_control(
    app: &tauri::AppHandle,
    script: String,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let owned = app.clone();
    let stopped = cancel.clone();
    let deadline = Instant::now() + Duration::from_secs(3);
    app.run_on_main_thread(move || {
        let callback = tx.clone();
        let admitted = (|| -> Result<(), String> {
            if stopped.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err("Control cancelled before render".into());
            }
            owned
                .get_webview_window(CONTROL)
                .ok_or("Control window is absent")?
                .eval_with_callback(script, move |raw| {
                    let _ = callback.send(Ok(raw));
                })
                .map_err(|_| "Control render dispatch failed".to_string())
        })();
        if let Err(error) = admitted {
            let _ = tx.send(Err(error));
        }
    })
    .map_err(|_| "Control render scheduling failed")?;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Control cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err("Control readiness was not acknowledged; no provider request".into());
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(raw) => {
                let value: serde_json::Value = serde_json::from_str(&raw?)
                    .map_err(|_| "Invalid control readiness acknowledgement")?;
                if value.get("ready") == Some(&serde_json::json!(true))
                    && value.get("version") == Some(&serde_json::json!(1))
                {
                    phase("ready_acknowledged");
                    return Ok(());
                }
                return Err("Control is not ready; no provider request".into());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Control renderer disconnected".into())
            }
        }
    }
}

fn main() {
    if std::env::var("ATSUMI_EMIT_SEARCH_CONTROL_SCRIPT").as_deref() == Ok("1") {
        println!(
            "{}",
            control_script(
                "https://atsumi-probe.invalid/continue/fixture",
                "https://atsumi-probe.invalid/cancel/fixture",
                false,
                "합성 이미지 검사를 준비 중입니다."
            )
        );
        return;
    }
    if std::env::var("ATSUMI_RUN_SYNTHETIC_LENS_PROBE").as_deref() != Ok("1") {
        eprintln!("Opt-in required. Only the approved neutral fixture may be sent.");
        return;
    }
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    tauri::Builder::default().setup(|app| {
        let service=match std::env::var("ATSUMI_SYNTHETIC_SEARCH_PROVIDER").as_deref() {
            Ok("saucenao")=>"saucenao",Ok("lens")|Err(_)=>"lens",_=>return Err("Only fixed lens or saucenao fixture probes are permitted".into()),
        };
        let cancel=Arc::new(AtomicBool::new(false));let resume=Arc::new(AtomicBool::new(false));let waiting=Arc::new(AtomicBool::new(false));
        let token=uuid::Uuid::new_v4().to_string();
        let continue_url=format!("https://atsumi-probe.invalid/continue/{token}");let cancel_url=format!("https://atsumi-probe.invalid/cancel/{token}");
        let nav_resume=resume.clone();let nav_cancel=cancel.clone();let nav_waiting=waiting.clone();
        let script=control_script(&continue_url,&cancel_url,false,"합성 이미지 검사를 준비 중입니다.");
        let handoff_continue=continue_url.clone();let handoff_cancel=cancel_url.clone();
        let control=tauri::WebviewWindowBuilder::new(app,"x-search-probe-control",tauri::WebviewUrl::External("about:blank".parse().unwrap()))
            .title("Atsumi · 합성 자료 검색 검사").visible(false).focused(false).inner_size(760.,350.)
            .incognito(true).data_directory(app.path().app_cache_dir()?.join("x-search-fixture-controls-v1"))
            .on_navigation(move |url| {
                if url.as_str()=="about:blank" {return true;}
                if url.as_str()==continue_url&&nav_waiting.swap(false,Ordering::AcqRel) {phase("continue_clicked");nav_resume.store(true,Ordering::Release);}
                if url.as_str()==cancel_url {phase("cancel_clicked");nav_cancel.store(true,Ordering::Release);}
                false
            })
            .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny).on_download(|_,_|false).build()?;
        let close_cancel=cancel.clone();control.on_window_event(move |event|match event {
            tauri::WindowEvent::CloseRequested{api,..}=>{api.prevent_close();phase("close_requested");close_cancel.store(true,Ordering::Release);},
            tauri::WindowEvent::Destroyed=>{phase("control_destroyed");close_cancel.store(true,Ordering::Release);},_=>{},
        });
        let handoff_app=app.handle().clone();let handoff_waiting=waiting.clone();
        let stopped=cancel.clone();
        let handoff:Arc<dyn Fn(String)+Send+Sync>=Arc::new(move |message| {
            phase("handoff_received");
            let display=control_script(&handoff_continue,&handoff_cancel,true,&message);
            handoff_waiting.store(true,Ordering::Release);
            if prepare_control(&handoff_app,display,&stopped).is_err(){phase("handoff_control_failed");handoff_waiting.store(false,Ordering::Release);stopped.store(true,Ordering::Release);return;}
            let owned=handoff_app.clone();let show_cancel=stopped.clone();let show_waiting=handoff_waiting.clone();
            if handoff_app.run_on_main_thread(move ||if !show_cancel.load(Ordering::Acquire) {
                if owned.get_webview_window(CONTROL).is_none_or(|control|control.show().is_err()) {phase("handoff_show_failed");show_waiting.store(false,Ordering::Release);show_cancel.store(true,Ordering::Release);}
            }).is_err(){phase("handoff_show_dispatch_failed");handoff_waiting.store(false,Ordering::Release);stopped.store(true,Ordering::Release);}
        });
        let handle=app.handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".runtime/x-source-search-20261021");
            let result=(|| -> Result<serde_json::Value,String> {
                prepare_control(&handle,script,&cancel)?;
                let input=std::fs::read(root.join("neutral-shapes.jpg")).map_err(|_|"Approved fixture is absent")?;
                // The recorded approved fixture is small and has no user data.
                if input.len()!=11793 || format!("{:X}",Sha256::digest(&input))!="4D2FEB4CF3F6A87DBF830A583B43108A21A25E246AC14DCDA8C68A2C110390FF" {return Err("Approved fixture identity changed; no upload".into());}
                let evidence=Arc::new(Mutex::new(None));
                let outcome=provider::probe(handle.clone(),ProviderRequest{job_id:uuid::Uuid::new_v4().to_string(),input:ProviderInput::Jpeg(input)},cancel,resume,handoff,service,evidence.clone());
                let document=evidence.lock().unwrap_or_else(|p|p.into_inner()).take().map(redact_evidence);
                let report=serde_json::json!({"outcome":outcome,"evidence":document});
                let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|"Evidence clock unavailable")?.as_millis();
                std::fs::write(root.join(format!("{service}-persistent-probe-{stamp}.json")),serde_json::to_vec_pretty(&report).map_err(|_|"Evidence serialization failed")?).map_err(|_|"Evidence write failed")?;
                Ok(report)
            })();
            match result {Ok(report)=>println!("{}",serde_json::json!({"outcome":report["outcome"],"evidenceSaved":true})),Err(error)=>println!("{}",serde_json::json!({"error":error}))}
            handle.exit(0);
        });
        Ok(())
    }).run(context).expect("isolated fixture runtime");
}

#[cfg(test)]
mod control_tests {
    use super::*;
    #[test]
    fn human_document_redaction_covers_current_and_future_diagnostic_fields() {
        for state in ["challenge", "consent"] {
            let input=serde_json::json!({"state":state,"origin":"https://www.google.com","path":"/sorry/index","document":{"title":"SECRET_IP","body":"SECRET_IP","outline":["SECRET_TOKEN"],"textBlocks":[{"text":"SECRET_IP"}],"futureField":"SECRET_TOKEN"}});
            let output=redact_evidence(input).to_string();
            assert!(!output.contains("SECRET_"));
            assert!(!output.contains("textBlocks"));
            assert!(output.contains("document omitted"));
        }
        let result=serde_json::json!({"state":"complete","document":{"title":"Observed result"}});
        assert_eq!(redact_evidence(result.clone()),result);
    }
    #[test]
    fn control_html_is_json_encoded_not_nested_escaped_javascript() {
        let script = control_script(
            "https://atsumi-probe.invalid/continue/fixture",
            "https://atsumi-probe.invalid/cancel/fixture",
            true,
            "확인 '완료'\n다음",
        );
        let config = script
            .split("const config=")
            .nth(1)
            .unwrap()
            .split(";\n")
            .next()
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(config).unwrap();
        assert_eq!(value["html"], CONTROL_HTML);
        assert_eq!(value["waiting"], true);
        assert_eq!(value["message"], "확인 '완료'\n다음");
        assert!(!script.contains("__CONFIG__"));
        assert!(script.contains("event.isTrusted"));
        assert!(script.contains("ready:true,version:1"));
    }
    #[test]
    fn upload_is_reached_only_after_local_control_readiness() {
        let source = include_str!("x_media_search_probe.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let worker = source.split("spawn_blocking(move ||").nth(1).unwrap();
        assert!(
            worker.find("prepare_control(").unwrap() < worker.find("provider::probe(").unwrap()
        );
        assert!(!CONTROL_HTML.contains("src="));
        assert!(!CONTROL_SCRIPT.contains("fetch("));
        assert!(source.contains("phase(\"close_requested\")"));
    }
    #[test]
    fn repeated_handoff_arms_native_resume_before_enabling_control() {
        let source = include_str!("x_media_search_probe.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let handoff = source
            .split("phase(\"handoff_received\")")
            .nth(1)
            .unwrap()
            .split("let handle=app.handle()")
            .next()
            .unwrap();
        assert!(
            handoff.find("handoff_waiting.store(true").unwrap()
                < handoff.find("prepare_control(").unwrap()
        );
        assert!(handoff.contains("handoff_waiting.store(false"));
        assert!(handoff.contains("handoff_show_dispatch_failed"));
    }
}
