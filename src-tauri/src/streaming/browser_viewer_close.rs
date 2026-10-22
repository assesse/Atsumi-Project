//! Close one presentation, never its shared recording receiver or other chats.
use super::*;
use crate::platform_native_geometry::{Hole, NativeViewport, Rect};
use crate::platform_native_player_intent::{Binding, Current, Intent};

fn binding(pane: &Pane, intent: &Intent) -> Option<Binding> {
    let epoch = pane.epoch.load(Ordering::Acquire);
    if epoch == 0 || pane.dead.load(Ordering::Acquire) || pane.kind != PaneKind::Video { return None; }
    let viewport = pane.viewport.try_lock().ok()?.clone();
    if !viewport.visible || viewport.occluded || viewport.suspend_audio || viewport.epoch != epoch { return None; }
    let generation = pane.capture.as_ref()?.inner.view.try_lock().ok()?.page_generation;
    let clip = viewport.clip.clone().unwrap_or(BrowserClip { x:0., y:0., width:viewport.width, height:viewport.height });
    let token = format!("{}:{epoch}:{}", pane.id, pane.revision.load(Ordering::Acquire));
    let mut holes:Vec<Hole> = viewport.occlusions.iter().map(|r| Hole { x:viewport.x+r.x,y:viewport.y+r.y,width:r.width,height:r.height,radii:[0.;4] }).collect();
    // Conservatively exclude rounded-popup bounding boxes from close gestures.
    holes.extend(viewport.rounded_occlusions.iter().map(|r|Hole{x:viewport.x+r.x,y:viewport.y+r.y,width:r.width,height:r.height,radii:[0.;4]}));
    intent.present(&token, generation, &NativeViewport {
        sequence:1, player:Some(Rect {x:viewport.x+clip.x,y:viewport.y+clip.y,width:clip.width,height:clip.height}),
        chat:None,visible:true,muted:false,occlusions:holes,
    });
    intent.current(generation)
}

pub(super) fn script(nonce: &str) -> String {
    format!("window.__atsumiCloseViewerNonce={};", serde_json::json!(nonce))
}

#[cfg(windows)]
pub(super) fn install(view: &Webview, pane: Arc<Pane>, nonce: String) -> Result<(), StreamError> {
    use tauri::Emitter;
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows::{core::PWSTR, Win32::Foundation::HWND};
    let app = view.app_handle().clone();
    let root = view.window().hwnd().map_err(|_| unavailable())?.0 as usize;
    let event_view = view.clone();
    let intent = Arc::new(Intent::default());
    let current: Current = { let pane=pane.clone();let intent=intent.clone();Arc::new(move|| binding(&pane,&intent)) };
    view.with_webview(move |platform| unsafe {
        let result = (|| -> windows::core::Result<()> {
            let controller = platform.controller(); let mut child=HWND::default(); controller.ParentWindow(&mut child)?;
            let child=child.0 as usize;
            crate::platform_native_player_intent::install(&controller,root,child,intent.clone(),current.clone())?;
            let core=controller.CoreWebView2()?;let mut registration=0;
            core.add_WebMessageReceived(&WebMessageReceivedEventHandler::create(Box::new(move|_,args|{
                let Some(args)=args else{return Ok(());};
                let mut source=PWSTR::null();args.Source(&mut source)?;
                if CoTaskMemPWSTR::from(source).to_string()!=pane_url(&pane.channel,PaneKind::Video){return Ok(());}
                let mut body=PWSTR::null();if args.TryGetWebMessageAsString(&mut body).is_err(){return Ok(());}
                let body=CoTaskMemPWSTR::from(body).to_string();
                let Some(body)=body.strip_prefix("ATSUMI_VIEWER_CLOSE:") else{return Ok(());};
                if body.len()>256{return Ok(());}
                #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Request{id:String,nonce:String}
                let Ok(request)=serde_json::from_str::<Request>(body) else{return Ok(());};
                if request.nonce!=nonce||uuid::Uuid::parse_str(&request.id).is_err(){return Ok(());}
                let now=current();
                let Some(proof)=crate::platform_native_player_intent::consume(&intent,now.as_ref(),root,child) else{
                    reply(&event_view,&request.id,Err(control_stale()));return Ok(());
                };
                let pane=pane.clone();let current=current.clone();let app=app.clone();let event_view=event_view.clone();
                tauri::async_runtime::spawn_blocking(move||{
                    let result=host(&app).and_then(|host|host.close_presented_channel(&app,&pane,&proof,&current));
                    if let Ok(snapshot)=&result {let _=app.emit_to("main","chzzk-viewer-closed",json!({"snapshot":snapshot,"channelId":pane.channel}));}
                    reply(&event_view,&request.id,result.map(|_|json!({"accepted":true})));
                });
                Ok(())
            })),&mut registration)?;
            Ok(())
        })();
        if result.is_err(){tracing::warn!(diag_stage="chzzk_viewer_close_unavailable");}
    }).map_err(|_|unavailable())
}
#[cfg(not(windows))]
pub(super) fn install(_: &Webview,_:Arc<Pane>,_:String)->Result<(),StreamError>{Ok(())}

impl OfficialBrowser {
    fn close_presented_channel(&self, app:&AppHandle, target:&Arc<Pane>, proof:&Binding, current:&Current)->Result<MultiViewSnapshot,StreamError>{
        let _mutation=self.inner.multiview.mutations.lock().map_err(|_|busy())?;
        if current().as_ref()!=Some(proof){return Err(control_stale());}
        let removed={
            let mut state=self.inner.multiview.state.lock().map_err(|_|unavailable())?;
            if !self.multiview_active()||!state.panes.iter().any(|p|Arc::ptr_eq(p,target))||state.epoch!=target.epoch.load(Ordering::Acquire){return Err(control_stale());}
            // All currently created video panes are receiver backed. Never tear
            // down a legacy active capture if an older layout survived upgrade.
            if !target.id.starts_with("chzzk-auto-"){return Err(error("VIEWER_LEGACY","이전 시청 화면은 방송 조작에서 종료해 주세요."));}
            let removed=state.panes.iter().filter(|p|p.channel==target.channel).cloned().collect::<Vec<_>>();
            state.panes.retain(|p|p.channel!=target.channel);
            state.entries.retain(|e|e.channel_id!=target.channel);
            if state.audio_owner.as_deref()==Some(target.channel.as_str()){state.audio_owner=None;}
            if state.panes.is_empty(){self.inner.multiview.active.store(false,Ordering::Release);}
            removed
        };
        for pane in removed {
            if pane.kind==PaneKind::Video {
                let revision=pane.release_presentation();
                if let Some(view)=app.get_webview(&pane.id){
                    let _=native_mute(&view,pane.clone(),true);
                    let _=view.eval(format!("window.__atsumiAutoReceiver?.configure({{revision:{revision},viewing:false,multiview:false}});"));
                    let _=auto_watch::apply_auto_receiver_viewport(&view,&BrowserViewport::default(),pane.revision.clone(),revision);
                }
            } else {
                pane.dead.store(true,Ordering::Release);pane.revision.fetch_add(1,Ordering::AcqRel);
                if let Some(view)=app.get_webview(&pane.id){let _=view.hide();if view.close().is_err(){tracing::warn!(diag_stage="chzzk_closed_chat_dispose_failed");}}
            }
        }
        self.multiview_snapshot()
    }
}
