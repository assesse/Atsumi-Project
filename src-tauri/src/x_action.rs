use super::*;
fn valid_action_request(post_id:&str,action:&str,text:&str)->bool {
    valid_id(post_id) && matches!(action,"like"|"bookmark") && text.is_empty()
}
fn removes_from_collection(feed:&str,action:&str,expected:bool)->bool {
    expected && matches!((feed,action),("likes","like")|("bookmarks","bookmark"))
}
fn action_elapsed_ms(started:Instant)->u64 {
    started.elapsed().as_millis().min(600_000) as u64
}
fn probe_outcome(result:&Result<String,String>)->&'static str {
    match result.as_deref() {
        Ok("ready")=>"ready",Ok("pending")=>"postMissing",Ok("pendingOwner")=>"ownerPending",
        Ok("offscreen")=>"offscreen",Ok("disabled")=>"disabled",Ok("unavailable")=>"controlMissing",
        Ok("stateChanged")=>"stateChanged",Ok("rejected")=>"rejected",Ok(_)=>"invalidOutcome",
        Err(error) if error==STALE=>"stale",Err(_)=>"evaluationError",
    }
}
fn probe_may_wait(state:&str)->bool {
    matches!(state,"pending"|"pendingOwner"|"offscreen"|"disabled"|"unavailable")
}
fn action_error(outcome:&str)->String {
    if outcome=="stateChanged" {"게시물 상태가 바뀌었습니다. 다시 불러온 뒤 확인해 주세요."}
    else {"X에서 이 동작을 확인하지 못했습니다. 다시 불러온 뒤 확인해 주세요."}.into()
}
fn current_action(session:&XSession,revision:u64,generation:u64,owner:&str,post:&Post)->Result<(),String> {
    session.current(revision,&AtomicBool::new(false))?;
    if session.generation()!=generation {return Err(STALE.into());}
    let data=session.data.lock().unwrap_or_else(|p|p.into_inner());
    if data.owner.as_deref()!=Some(owner)||!data.posts.get(&post.id).is_some_and(|known|known.handle==post.handle) {return Err(STALE.into());}
    Ok(())
}
fn valid_action_route(value:&Value,feed:&str,owner:&str,path:&str)->bool {
    value["state"]=="navigate"&&value["owner"].as_str()==Some(owner)&&value["path"].as_str()==Some(path)&&feed_target(feed,owner,path)
}
fn confirmed_action_post(cached:&Post,observed:&Post,action:&str,expected:bool)->Option<Post> {
    if cached.id!=observed.id||!cached.handle.eq_ignore_ascii_case(&observed.handle) {return None;}
    let mut result=cached.clone();
    match action {
        "like" if observed.stats.can_like&&observed.stats.liked!=expected=>{
            result.stats.liked=observed.stats.liked;result.stats.likes=observed.stats.likes.clone();result.stats.can_like=true;
        },
        "bookmark" if observed.stats.can_bookmark&&observed.stats.bookmarked!=expected=>{
            result.stats.bookmarked=observed.stats.bookmarked;result.stats.bookmarks=observed.stats.bookmarks.clone();result.stats.can_bookmark=true;
        },
        _=>return None,
    }
    Some(result)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields,rename_all="camelCase")]
struct ActionObservation {
    post_id:String,handle:String,owner:String,path:String,active:bool,count:String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionConfirmation {
    nonce:String,state:String,observation:Option<ActionObservation>,
}
fn confirmation_result(value:Value,nonce:&str,cached:&Post,action:&str,expected:bool,owner:&str,path:&str)->Result<(Option<Post>,&'static str),String>{
    let value:ActionConfirmation=serde_json::from_value(value).map_err(|_|ERROR)?;
    if value.nonce!=nonce{return Err(ERROR.into());}
    if value.state!="observed" {
        if value.observation.is_some(){return Err(ERROR.into());}
        let outcome=match value.state.as_str(){
            "pending"=>"targetMissing","pendingOwner"=>"ownerPending","unavailable"=>"controlMissing",
            "disabled"=>"controlDisabled","ambiguous"=>"controlAmbiguous","identityMissing"=>"identityMissing",
            "rejected"=>return Err(STALE.into()),_=>return Err(ERROR.into()),
        };
        return Ok((None,outcome));
    }
    let observed=value.observation.ok_or(ERROR)?;
    if observed.post_id!=cached.id||!observed.handle.eq_ignore_ascii_case(&cached.handle)
        ||observed.owner!=owner||observed.path!=path{return Err(STALE.into());}
    if observed.count.chars().count()>32||observed.count.chars().any(char::is_control){return Err(ERROR.into());}
    // Populate only the independently proved requested action. Source media,
    // text and unrelated reactions are not read or overwritten by confirmation.
    let mut post=cached.clone();
    match action {
        "like"=>{post.stats.can_like=true;post.stats.liked=observed.active;post.stats.likes=observed.count;},
        "bookmark"=>{post.stats.can_bookmark=true;post.stats.bookmarked=observed.active;post.stats.bookmarks=observed.count;},
        _=>return Err(ERROR.into()),
    }
    let updated=confirmed_action_post(cached,&post,action,expected);
    let outcome=if updated.is_some(){"confirmed"}else{"unchanged"};
    Ok((updated,outcome))
}
fn evaluate_value(session:&XSession,app:&AppHandle,label:&str,request:&Value,revision:u64)->Result<Value,String>{
    session.current(revision,&AtomicBool::new(false))?;
    let view=app.get_webview(label).ok_or(STALE)?;
    let path=request["documentPath"].as_str().or(request["path"].as_str()).ok_or(ERROR)?;
    if !view.url().is_ok_and(|u|allowed(&u)&&u.path()==path){return Err(STALE.into());}
    let script=include_str!("x_action.js").replace("__ATSUMI_X_ACTION__",&request.to_string());
    let(tx,rx)=std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script,move|v|{let _=tx.send(v);}).map_err(|_|ERROR)?;
    let raw=rx.recv_timeout(Duration::from_secs(3)).map_err(|_|ERROR)?;
    session.current(revision,&AtomicBool::new(false))?;
    if raw.len()>2048||!view.url().is_ok_and(|u|allowed(&u)&&u.path()==path){return Err(STALE.into());}
    let value:Value=serde_json::from_str(&raw).map_err(|_|ERROR)?;
    if value["nonce"]!=request["nonce"]{return Err(ERROR.into());}
    Ok(value)
}
fn evaluate(session:&XSession,app:&AppHandle,label:&str,request:&Value,revision:u64)->Result<String,String>{
    let value=evaluate_value(session,app,label,request,revision)?;
    Ok(value["state"].as_str().unwrap_or("rejected").to_string())
}
fn prepare_detail(session:&XSession,app:&AppHandle,feed:&str,owner:&str,post:&Post,action:&str,expected:bool,revision:u64,generation:u64)->Result<(String,String,String),String> {
    current_action(session,revision,generation,owner,post)?;
    let started=Instant::now();
    let created=session.create_reader(app,feed,revision,&AtomicBool::new(false));
    debug_shape("action","prepare-create",&json!({"kind":action,"createMs":action_elapsed_ms(started),"ready":created.is_ok()}));
    let label=created?;
    let path=format!("/{}/status/{}",post.handle,post.id);
    let mut navigated=false;let mut route_ms=None;let mut readiness_started=None;
    let mut probe_count=0u32;let mut last_outcome="routePending";
    let result=(||->Result<(),String>{
        while started.elapsed()<Duration::from_secs(25) {
            current_action(session,revision,generation,owner,post)?;
            if !navigated {
                // Reuse the existing rendered owner/navigation proof; no direct
                // endpoint, guessed destination or unbound profile is admitted.
                let value=session.evaluate(app,&label,feed,false,revision,&AtomicBool::new(false))?;
                current_action(session,revision,generation,owner,post)?;
                if value["state"]=="signedOut"||value["state"]=="ownerChanged" {session.detach(app);return Err(STALE.into());}
                if value["state"]=="navigate" {
                    if !valid_action_route(&value,feed,owner,&path) {return Err(STALE.into());}
                    {
                        let mut data=session.data.lock().unwrap_or_else(|p|p.into_inner());
                        session.current(revision,&AtomicBool::new(false))?;
                        let reader=data.readers.get_mut(feed).filter(|r|r.label==label).ok_or(STALE)?;
                        if reader.owner.as_deref().is_some_and(|bound|bound!=owner) {return Err(STALE.into());}
                        reader.owner=Some(owner.into());
                        *reader.target.lock().unwrap_or_else(|p|p.into_inner())=Some(path.clone());
                    }
                    current_action(session,revision,generation,owner,post)?;
                    app.get_webview(&label).ok_or(STALE)?.navigate(format!("https://x.com{path}").parse().map_err(|_|ERROR)?).map_err(|_|ERROR)?;
                    navigated=true;route_ms=Some(action_elapsed_ms(started));readiness_started=Some(Instant::now());
                } else if value["state"]!="pending" {return Err(STALE.into());}
            } else {
                let url=app.get_webview(&label).ok_or(STALE)?.url().map_err(|_|ERROR)?;
                if !reader_url(&url,Some(&path)) {return Err(STALE.into());}
                if url.path()=="/login"||url.path().starts_with("/i/flow/") {session.detach(app);return Err(STALE.into());}
                if url.path()==path {
                    let request=json!({"nonce":uuid::Uuid::new_v4().to_string(),"documentPath":path,"path":path,"owner":owner,"action":action,"expected":expected,"phase":"probe"});
                    let probe_started=Instant::now();
                    let probe=evaluate(session,app,&label,&request,revision);
                    probe_count=probe_count.saturating_add(1);
                    let outcome=probe_outcome(&probe);
                    if outcome!=last_outcome {debug_shape("action","prepare-probe",&json!({"kind":action,"probeMs":action_elapsed_ms(probe_started),"outcome":outcome}));last_outcome=outcome;}
                    current_action(session,revision,generation,owner,post)?;
                    let state=probe?;
                    if state=="ready" {
                        let mut data=session.data.lock().unwrap_or_else(|p|p.into_inner());
                        session.current(revision,&AtomicBool::new(false))?;
                        let reader=data.readers.get_mut(feed).filter(|r|r.label==label&&r.owner.as_deref()==Some(owner)).ok_or(STALE)?;
                        reader.touched=Instant::now();reader.seen.insert(post.id.clone());
                        return Ok(());
                    }
                    if !probe_may_wait(&state) {return Err(action_error(&state));}
                }
            }
            std::thread::sleep(Duration::from_millis(220));
        }
        Err(action_error("timeout"))
    })();
    debug_shape("action","prepare",&json!({"kind":action,"prepareMs":action_elapsed_ms(started),"routeMs":route_ms,"readinessMs":readiness_started.map(action_elapsed_ms),"probes":probe_count,"outcome":last_outcome,"ready":result.is_ok()}));
    result?;
    Ok((feed.into(),label,path))
}
#[tauri::command(rename_all="camelCase")]
pub async fn x_post_action(window:Webview,state:State<'_,AppState>,session:State<'_,XSession>,post_id:String,action:String,generation:u64,expected:bool,text:String)->Result<Post,String>{
    trusted(&window)?;state.begin_platform_work(||Ok(()))?;
    if !valid_action_request(&post_id,&action,&text){return Err("좋아요와 북마크만 변경할 수 있습니다.".into());}
    // Capture admission before waiting. Suspend intentionally retains the UI
    // generation, so an action must not adopt its newer revision after the wait.
    let revision=session.revision.load(Ordering::Acquire);
    let queued=Instant::now();
    let _lane=session.feed_lane.lock().await;
    debug_shape("action","lane",&json!({"kind":action,"queueMs":action_elapsed_ms(queued)}));
    session.current(revision,&AtomicBool::new(false))?;
    let host=session.inner().clone();let app=window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move||{
        host.current(revision,&AtomicBool::new(false))?;
        if host.generation()!=generation{return Err(STALE.into());}
        let (post,owner)={let data=host.data.lock().unwrap_or_else(|p|p.into_inner());(data.posts.get(&post_id).cloned().ok_or(STALE)?,data.owner.clone().ok_or(STALE)?)};
        let reader_started=Instant::now();
        let detail_feed=format!("post:{post_id}");
        let mut candidates={let data=host.data.lock().unwrap_or_else(|p|p.into_inner());data.readers.values()
            .filter(|r|r.owner.as_deref()==Some(owner.as_str())&&r.touched.elapsed()<READER_IDLE&&(r.seen.contains(&post_id)||r.feed==detail_feed))
            .map(|r|(r.feed.clone(),r.label.clone(),r.touched)).collect::<Vec<_>>()};
        candidates.sort_by_key(|(feed,_,touched)|(feed!=&detail_feed,std::cmp::Reverse(*touched)));
        let mut selected=None;
        for (feed,label,_) in candidates.into_iter().take(3){
            current_action(&host,revision,generation,&owner,&post)?;
            let Some(view)=app.get_webview(&label) else{continue};
            let Ok(url)=view.url() else{continue};
            if !allowed(&url)||!feed_target(&feed,&owner,url.path()){continue;}
            let probe=json!({"nonce":uuid::Uuid::new_v4().to_string(),"documentPath":url.path(),"path":format!("/{}/status/{}",post.handle,post.id),"owner":owner,"action":action,"expected":expected,"phase":"probe"});
            let probe_started=Instant::now();
            let result=evaluate(&host,&app,&label,&probe,revision);
            debug_shape("action","probe",&json!({"kind":action,"reader":if feed==detail_feed {"detail"}else{"feed"},"probeMs":action_elapsed_ms(probe_started),"outcome":probe_outcome(&result),"collectionRemoval":removes_from_collection(&feed,&action,expected)}));
            current_action(&host,revision,generation,&owner,&post)?;
            if result.as_deref()==Ok("stateChanged") {return Err(action_error("stateChanged"));}
            if result.as_deref()==Ok("ready"){
                // Removing a collection entry also removes its rendered row.
                // Open detail before the only dispatch so confirmation cannot
                // mistake that disappearance for an unconfirmed mutation.
                if !removes_from_collection(&feed,&action,expected){selected=Some((feed,label,url.path().to_owned()));break;}
            }
        }
        // A read-only probe may fall back. After the single mutation dispatch,
        // no failure path retries or submits through another reader.
        let reader_path=if selected.is_some(){"fast"}else{"detail"};
        let reader_result=if let Some(candidate)=selected{Ok(candidate)}else{
            prepare_detail(&host,&app,&detail_feed,&owner,&post,&action,expected,revision,generation)
        };
        debug_shape("action","reader",&json!({"kind":action,"reader":reader_path,"readerMs":action_elapsed_ms(reader_started),"ready":reader_result.is_ok()}));
        let (_feed,label,document_path)=reader_result?;
        current_action(&host,revision,generation,&owner,&post)?;
        let request=json!({"nonce":uuid::Uuid::new_v4().to_string(),"documentPath":document_path,"path":format!("/{}/status/{}",post.handle,post.id),"owner":owner,"action":action,"expected":expected,"phase":"start"});
        let dispatch_started=Instant::now();
        let dispatch=evaluate(&host,&app,&label,&request,revision);
        debug_shape("action","dispatch",&json!({"kind":action,"dispatchMs":action_elapsed_ms(dispatch_started),"sent":dispatch.as_deref().is_ok_and(|s|s=="sent")}));
        let outcome=dispatch?;
        if outcome!="sent"{return Err(action_error(&outcome));}
        // No optimistic count changes and no repeat submission on uncertainty.
        let started=Instant::now();
        let mut polls=0u32;
        let mut last_outcome="notObserved";
        while started.elapsed()<Duration::from_secs(8){
            std::thread::sleep(Duration::from_millis(300));
            current_action(&host,revision,generation,&owner,&post)?;
            let mut confirm=request.clone();
            let nonce=uuid::Uuid::new_v4().to_string();
            confirm["nonce"]=json!(nonce);confirm["phase"]=json!("confirm");
            polls=polls.saturating_add(1);
            let observed=evaluate_value(&host,&app,&label,&confirm,revision);
            if let Err(error)=&observed{debug_shape("action","confirmation-error",&json!({"kind":action,"outcome":if error==STALE{"stale"}else{"evaluationError"},"confirmationMs":action_elapsed_ms(started),"polls":polls}));}
            let value=observed?;
            current_action(&host,revision,generation,&owner,&post)?;
            let observation=confirmation_result(value,&nonce,&post,&action,expected,&owner,request["path"].as_str().ok_or(ERROR)?);
            if let Err(error)=&observation{debug_shape("action","confirmation-error",&json!({"kind":action,"outcome":if error==STALE{"stale"}else{"invalidObservation"},"confirmationMs":action_elapsed_ms(started),"polls":polls}));}
            let (updated,normalized)=observation?;
            if normalized!=last_outcome{
                debug_shape("action","confirmation-observation",&json!({"kind":action,"outcome":normalized,"confirmationMs":action_elapsed_ms(started),"polls":polls}));
                last_outcome=normalized;
            }
            let Some(updated)=updated else{continue};
            {
                current_action(&host,revision,generation,&owner,&post)?;
                let mut data=host.data.lock().unwrap_or_else(|p|p.into_inner());
                host.current(revision,&AtomicBool::new(false))?;
                if host.generation()!=generation||data.owner.as_deref()!=Some(owner.as_str()) {return Err(STALE.into());}
                data.posts.insert(post_id.clone(),updated.clone());
                debug_shape("action","confirmed",&json!({"kind":action,"confirmationMs":action_elapsed_ms(started)}));
                return Ok(updated);
            }
        }
        debug_shape("action","confirmation-timeout",&json!({"kind":action,"outcome":last_outcome,"confirmationMs":action_elapsed_ms(started),"polls":polls}));
        Err("X의 처리 결과를 아직 확인하지 못했습니다. 중복 전송하지 말고 목록을 새로고침해 확인해 주세요.".into())
    }).await.map_err(|_|ERROR.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_post()->Post {
        serde_json::from_value(json!({"id":"123456789","handle":"creator","name":"Creator","text":"Cached complete text","time":"2026-10-19T00:00:00Z","media":["https://pbs.twimg.com/media/example.jpg"],"video":false,"truncated":false})).unwrap()
    }
    fn fixture_confirmation(active:bool)->Value {
        json!({"nonce":"confirm-nonce","state":"observed","observation":{"postId":"123456789","handle":"creator","owner":"alice","path":"/creator/status/123456789","active":active,"count":"5"}})
    }
    fn confirm_fixture(value:Value,post:&Post,action:&str,expected:bool)->Result<(Option<Post>,&'static str),String>{
        confirmation_result(value,"confirm-nonce",post,action,expected,"alice","/creator/status/123456789")
    }
    #[test]
    fn action_confirmation_observation_is_action_only_and_changed_state_is_required(){
        let mut cached=fixture_post();cached.stats.bookmarked=true;cached.stats.bookmarks="9".into();cached.stats.likes="4".into();
        let (updated,outcome)=confirm_fixture(fixture_confirmation(true),&cached,"like",false).unwrap();
        assert_eq!(outcome,"confirmed");let updated=updated.unwrap();
        assert!(updated.stats.liked);assert_eq!(updated.stats.likes,"5");assert!(updated.stats.bookmarked);assert_eq!(updated.stats.bookmarks,"9");
        assert_eq!(updated.media,cached.media);assert_eq!(updated.text,cached.text);assert_eq!(updated.name,cached.name);
        let (updated,outcome)=confirm_fixture(fixture_confirmation(false),&cached,"like",false).unwrap();
        assert_eq!(outcome,"unchanged");assert!(updated.is_none());
        let (updated,outcome)=confirm_fixture(fixture_confirmation(false),&cached,"bookmark",true).unwrap();
        assert_eq!(outcome,"confirmed");let updated=updated.unwrap();
        assert!(!updated.stats.bookmarked);assert_eq!(updated.stats.bookmarks,"5");assert_eq!(updated.stats.likes,"4");assert_eq!(updated.media,cached.media);
    }
    #[test]
    fn action_confirmation_observation_requires_exact_binding_and_nonce(){
        let cached=fixture_post();
        for (field,value) in [("postId","987654321"),("handle","another"),("owner","bob"),("path","/creator/status/987654321")]{
            let mut observation=fixture_confirmation(true);observation["observation"][field]=json!(value);
            assert!(confirm_fixture(observation,&cached,"like",false).is_err(),"{field}");
        }
        let mut observation=fixture_confirmation(true);observation["nonce"]=json!("other");
        assert!(confirm_fixture(observation,&cached,"like",false).is_err());
        assert!(confirm_fixture(fixture_confirmation(true),&cached,"repost",false).is_err());
        let mut observation=fixture_confirmation(true);observation["observation"]["handle"]=json!("CREATOR");
        assert_eq!(confirm_fixture(observation,&cached,"like",false).unwrap().1,"confirmed");
    }
    #[test]
    fn action_confirmation_observation_rejects_malformed_fields_and_unbounded_counts(){
        let cached=fixture_post();
        for bad in [Value::Null,json!("true"),json!(1),json!({})]{
            let mut observation=fixture_confirmation(true);observation["observation"]["active"]=bad;
            assert!(confirm_fixture(observation,&cached,"like",false).is_err());
        }
        for count in ["9".repeat(33),"5\nprivate".into()]{
            let mut observation=fixture_confirmation(true);observation["observation"]["count"]=json!(count);
            assert!(confirm_fixture(observation,&cached,"like",false).is_err());
        }
        let mut observation=fixture_confirmation(true);observation["observation"]["extra"]=json!("not permitted");
        assert!(confirm_fixture(observation,&cached,"like",false).is_err());
        let mut observation=fixture_confirmation(true);observation["extra"]=json!(true);
        assert!(confirm_fixture(observation,&cached,"like",false).is_err());
        let mut observation=fixture_confirmation(true);observation["observation"]["count"]=json!(null);
        assert!(confirm_fixture(observation,&cached,"like",false).is_err());
        let mut observation=fixture_confirmation(true);observation["observation"]["count"]=json!("9".repeat(32));
        assert_eq!(confirm_fixture(observation,&cached,"like",false).unwrap().1,"confirmed");
    }
    #[test]
    fn action_confirmation_wait_states_cannot_be_misread_as_success(){
        let cached=fixture_post();
        for (state,outcome) in [("pending","targetMissing"),("pendingOwner","ownerPending"),("unavailable","controlMissing"),("disabled","controlDisabled"),("ambiguous","controlAmbiguous"),("identityMissing","identityMissing")]{
            let value=json!({"nonce":"confirm-nonce","state":state});
            let (updated,normalized)=confirm_fixture(value,&cached,"like",true).unwrap();
            assert!(updated.is_none());assert_eq!(normalized,outcome);
        }
        for state in ["ready","sent","arbitrary source contents","rejected"]{
            assert!(confirm_fixture(json!({"nonce":"confirm-nonce","state":state}),&cached,"like",false).is_err());
        }
        assert!(confirm_fixture(json!({"nonce":"confirm-nonce","state":"observed"}),&cached,"like",false).is_err());
        let mut value=fixture_confirmation(true);value["state"]=json!("pending");
        assert!(confirm_fixture(value,&cached,"like",false).is_err());
    }
    #[test]
    fn action_probe_diagnostics_are_closed_and_only_hydration_states_wait() {
        for (value,normalized) in [("ready","ready"),("pending","postMissing"),("pendingOwner","ownerPending"),("offscreen","offscreen"),("disabled","disabled"),("unavailable","controlMissing"),("stateChanged","stateChanged"),("rejected","rejected")] {
            assert_eq!(probe_outcome(&Ok(value.into())),normalized);
        }
        assert_eq!(probe_outcome(&Ok("private post/owner payload".into())),"invalidOutcome");
        assert_eq!(probe_outcome(&Err("private evaluation failure".into())),"evaluationError");
        assert_eq!(probe_outcome(&Err(STALE.into())),"stale");
        for state in ["pending","pendingOwner","offscreen","disabled","unavailable"] {assert!(probe_may_wait(state));}
        for state in ["ready","sent","stateChanged","rejected","signedOut",""] {assert!(!probe_may_wait(state));}
    }
    #[test]
    fn action_preparation_route_requires_exact_cached_identity_and_owner() {
        let route=json!({"state":"navigate","owner":"alice","path":"/creator/status/123456789"});
        assert!(valid_action_route(&route,"post:123456789","alice","/creator/status/123456789"));
        for wrong in [json!({"state":"ready","owner":"alice","path":"/creator/status/123456789"}),json!({"state":"navigate","owner":"bob","path":"/creator/status/123456789"}),json!({"state":"navigate","owner":"alice","path":"/another/status/123456789"}),json!({"state":"navigate","owner":"alice","path":"https://evil.test/creator/status/123456789"})] {
            assert!(!valid_action_route(&wrong,"post:123456789","alice","/creator/status/123456789"));
        }
        assert!(!valid_action_route(&route,"post:987654321","alice","/creator/status/123456789"));
        assert!(!valid_action_route(&route,"post:123456789","alice","/creator/status/123456789?other=1"));
    }
    #[test]
    fn action_confirmation_updates_only_the_proved_action_without_erasing_media() {
        let mut cached=fixture_post();cached.stats.bookmarked=true;cached.stats.bookmarks="7".into();cached.stats.likes="4".into();
        let mut observed=cached.clone();observed.media.clear();observed.text.clear();observed.stats.bookmarked=false;observed.stats.bookmarks.clear();observed.stats.liked=true;observed.stats.likes="5".into();observed.stats.can_like=true;
        let updated=confirmed_action_post(&cached,&observed,"like",false).unwrap();
        assert!(updated.stats.liked);assert_eq!(updated.stats.likes,"5");assert!(updated.stats.bookmarked);assert_eq!(updated.stats.bookmarks,"7");
        assert_eq!(updated.media,cached.media);assert_eq!(updated.text,cached.text);
        observed.stats.bookmarked=false;observed.stats.bookmarks="6".into();observed.stats.can_bookmark=true;observed.stats.likes="0".into();
        let updated=confirmed_action_post(&cached,&observed,"bookmark",true).unwrap();
        assert!(!updated.stats.bookmarked);assert_eq!(updated.stats.bookmarks,"6");assert!(!updated.stats.liked);assert_eq!(updated.stats.likes,"4");assert_eq!(updated.media,cached.media);
    }
    #[test]
    fn action_confirmation_requires_a_real_control_and_changed_exact_post_state() {
        let cached=fixture_post();let mut observed=cached.clone();
        // A missing control defaults to false; that is not proof of an unlike.
        assert!(confirmed_action_post(&cached,&observed,"like",true).is_none());
        assert!(confirmed_action_post(&cached,&observed,"bookmark",true).is_none());
        observed.stats.can_like=true;
        assert!(confirmed_action_post(&cached,&observed,"like",false).is_none());
        observed.stats.liked=true;
        assert!(confirmed_action_post(&cached,&observed,"like",false).is_some());
        observed.handle="another".into();assert!(confirmed_action_post(&cached,&observed,"like",false).is_none());
        observed.handle=cached.handle.clone();observed.id="987654321".into();assert!(confirmed_action_post(&cached,&observed,"like",false).is_none());
        observed.id=cached.id.clone();assert!(confirmed_action_post(&cached,&observed,"repost",false).is_none());
    }
    #[test]
    fn action_preparation_cannot_adopt_a_new_generation_or_account() {
        let session=XSession::default();let post=fixture_post();
        {let mut data=session.data.lock().unwrap();data.owner=Some("alice".into());data.posts.insert(post.id.clone(),post.clone());}
        let revision=session.revision.load(Ordering::Acquire);let generation=session.generation();
        assert!(current_action(&session,revision,generation,"alice",&post).is_ok());
        session.generation.fetch_add(1,Ordering::AcqRel);
        assert!(current_action(&session,revision,generation,"alice",&post).is_err());
        assert!(current_action(&session,revision,session.generation(),"bob",&post).is_err());
        session.revision.fetch_add(1,Ordering::AcqRel);
        assert!(current_action(&session,revision,session.generation(),"alice",&post).is_err());
    }
    #[test]
    fn only_like_and_bookmark_can_reach_the_browser() {
        for action in ["like","bookmark"] { assert!(valid_action_request("123456789",action,"")); }
        for action in ["reply","repost","retweet","delete",""] { assert!(!valid_action_request("123456789",action,"")); }
        assert!(!valid_action_request("123456789","like","unwanted reply"));
        assert!(!valid_action_request("not-a-post","bookmark",""));
    }
    #[test]
    fn collection_removals_require_a_persistent_detail_row() {
        assert!(removes_from_collection("likes","like",true));
        assert!(removes_from_collection("bookmarks","bookmark",true));
        for (feed,action,expected) in [("likes","like",false),("bookmarks","bookmark",false),("foryou","like",true),("post:123456789","bookmark",true),("likes","bookmark",true),("bookmarks","like",true)] {
            assert!(!removes_from_collection(feed,action,expected));
        }
    }
    #[test]
    fn queued_action_revision_cannot_adopt_a_suspend_revision() {
        tauri::async_runtime::block_on(async {
            let session=XSession::default();
            let occupied=session.feed_lane.lock().await;
            let admitted_revision=session.revision.load(Ordering::Acquire);
            let generation=session.generation();
            // Suspend revokes revision but retains the visible generation/cache.
            session.revision.fetch_add(1,Ordering::AcqRel);
            drop(occupied);
            let _lane=session.feed_lane.lock().await;
            assert_eq!(session.generation(),generation);
            assert!(session.current(admitted_revision,&AtomicBool::new(false)).is_err());
        });
    }
}
