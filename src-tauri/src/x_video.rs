//! Bounded media leases for URLs actually requested by the official reader.
//! No credential export, guessed endpoint, filesystem cache or open proxy.
use super::*;
use tauri::http::{Request, Response};
const MAX_MEDIA_BYTES:u64=16*1024*1024;
const MAX_PLAYLIST_BYTES:u64=2*1024*1024;
const OPEN_RANGE_CHUNK_BYTES:u64=4*1024*1024;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum RequestedRange { From{start:u64,end:u64}, Suffix(u64) }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
struct PartialRange { start:u64,end:u64,total:u64 }
impl PartialRange {
    fn length(self)->u64 { self.end-self.start+1 }
    fn header(self)->String { format!("bytes {}-{}/{}",self.start,self.end,self.total) }
}
fn decimal(value:&str)->Result<u64,String>{
    if value.is_empty()||!value.bytes().all(|b|b.is_ascii_digit()){return Err(ERROR.into());}
    value.parse().map_err(|_|ERROR.into())
}
fn parse_range(value:&str)->Result<RequestedRange,String>{
    if value.len()>64{return Err(ERROR.into());}
    let (first,last)=value.strip_prefix("bytes=").and_then(|v|v.split_once('-')).ok_or(ERROR)?;
    if first.is_empty(){
        let count=decimal(last)?;
        return if count>0&&count<=MAX_MEDIA_BYTES{Ok(RequestedRange::Suffix(count))}else{Err(ERROR.into())};
    }
    let start=decimal(first)?;
    let end=if last.is_empty(){start.checked_add(OPEN_RANGE_CHUNK_BYTES-1).ok_or(ERROR)?}else{decimal(last)?};
    let length=end.checked_sub(start).and_then(|v|v.checked_add(1)).ok_or(ERROR)?;
    if length>MAX_MEDIA_BYTES{return Err(ERROR.into());}
    Ok(RequestedRange::From{start,end})
}
fn request_range(headers:&tauri::http::HeaderMap)->Result<Option<RequestedRange>,String>{
    let mut values=headers.get_all("range").iter();
    let range=values.next().map(|v|v.to_str().map_err(|_|ERROR.to_string()).and_then(parse_range)).transpose()?;
    if values.next().is_some(){return Err(ERROR.into());}
    Ok(range)
}
fn range_header(range:RequestedRange)->String{
    match range{RequestedRange::From{start,end}=>format!("bytes={start}-{end}"),RequestedRange::Suffix(count)=>format!("bytes=-{count}")}
}
fn partial_range(requested:RequestedRange,value:&str)->Result<PartialRange,String>{
    if value.len()>96{return Err(ERROR.into());}
    let (positions,total)=value.strip_prefix("bytes ").and_then(|v|v.split_once('/')).ok_or(ERROR)?;
    let (start,end)=positions.split_once('-').ok_or(ERROR)?;
    let actual=PartialRange{start:decimal(start)?,end:decimal(end)?,total:decimal(total)?};
    if actual.start>actual.end||actual.end>=actual.total{return Err(ERROR.into());}
    let expected=match requested{
        RequestedRange::From{start,end} if start<actual.total=>(start,end.min(actual.total-1)),
        RequestedRange::Suffix(count)=>(actual.total.saturating_sub(count),actual.total-1),
        _=>return Err(ERROR.into()),
    };
    if (actual.start,actual.end)!=expected||actual.length()>MAX_MEDIA_BYTES{return Err(ERROR.into());}
    Ok(actual)
}
fn unsatisfied_range(requested:RequestedRange,value:&str)->Result<u64,String>{
    if value.len()>32{return Err(ERROR.into());}
    let total=decimal(value.strip_prefix("bytes */").ok_or(ERROR)?)?;
    let unsatisfied=match requested{RequestedRange::From{start,..}=>start>=total,RequestedRange::Suffix(_)=>total==0};
    if unsatisfied{Ok(total)}else{Err(ERROR.into())}
}
fn response_range(requested:Option<RequestedRange>,status:u16,header:Option<&str>)->Result<Option<PartialRange>,String>{
    match (requested,status,header){
        (None,200,None)=>Ok(None),
        (Some(requested),206,Some(value))=>partial_range(requested,value).map(Some),
        _=>Err(ERROR.into()),
    }
}
fn valid_body_length(actual:u64,declared:Option<u64>,partial:Option<PartialRange>,limit:u64)->bool{
    actual<=limit&&declared.is_none_or(|length|length==actual)&&partial.is_none_or(|range|range.length()==actual)
}
#[derive(Clone)]
struct Resource { url: Url, scope: String, generation: u64, revision:u64, touched: Instant }
#[derive(Default)]
pub(super) struct Videos { observed: VecDeque<Url>, resources: HashMap<String,Resource>, dom_scopes:HashMap<String,Vec<String>> }
fn allowed_video_query_key(key:&str)->bool {
    matches!(key,"tag"|"container"|"v"|"variant_version")
}
pub(super) fn scope(url: &Url) -> Option<String> {
    if url.scheme()!="https" || url.host_str()!=Some("video.twimg.com") || url.port().is_some()
        || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some()
        || url.as_str().len()>2048 || url.path().contains('%')
        || url.query_pairs().any(|(k,v)|!allowed_video_query_key(k.as_ref())||v.len()>40||!v.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_'||b==b'-')) { return None; }
    let parts=url.path().trim_start_matches('/').split('/').collect::<Vec<_>>();
    if parts.iter().any(|p|p.is_empty()||!p.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'|b'.'))) { return None; }
    if !["m3u8","mp4","m4s","ts","aac"].iter().any(|ext|url.path().ends_with(&format!(".{ext}"))) { return None; }
    match parts.as_slice() {
        [kind,id,..] if matches!(*kind,"ext_tw_video"|"amplify_video")&&valid_id(id)=>Some(format!("{kind}/{id}")),
        ["tweet_video",file] if file.ends_with(".mp4")=>Some(format!("tweet_video/{}",file.trim_end_matches(".mp4"))),
        _=>None
    }
}
// Debug-only structure, never the rejected address or any query value. This
// reports the existing policy; it must not authorize a previously rejected URL.
#[cfg(any(debug_assertions,test))]
pub(super) fn rejected_scope_shape(url:&Url)->Option<Value>{
    if scope(url).is_some(){return None;}
    let boundary=if url.scheme()!="https"{Some("scheme")}
        else if url.host_str()!=Some("video.twimg.com"){Some("host")}
        else if url.port().is_some(){Some("port")}
        else if !url.username().is_empty()||url.password().is_some(){Some("userinfo")}
        else if url.fragment().is_some(){Some("fragment")}
        else if url.as_str().len()>2048{Some("length")}else{None};
    let parts=url.path().trim_start_matches('/').split('/').take(65).collect::<Vec<_>>();
    let family=match parts.first().copied(){Some("ext_tw_video")=>"ext_tw_video",Some("amplify_video")=>"amplify_video",Some("tweet_video")=>"tweet_video",_=>"other"};
    let bad_segment=parts.iter().take(64).position(|p|p.is_empty()||!p.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'|b'.')));
    let mut query_keys=Vec::new();
    let mut queries_truncated=false;
    if url.as_str().len()<=2048{
        for (index,(key,_)) in url.query_pairs().take(9).enumerate(){
            if index==8{queries_truncated=true;break;}
            if !key.is_empty()&&key.len()<=32&&key.bytes().all(|b|b.is_ascii_alphabetic()||b==b'_'){
                query_keys.push(key.into_owned());
            }
        }
    }
    let reason=if boundary.is_some(){"uri_boundary"}
        else if url.path().contains('%'){"path_encoding"}
        else if url.query_pairs().any(|(k,_)|!allowed_video_query_key(k.as_ref())){"query_key"}
        else if url.query_pairs().any(|(_,v)|v.len()>40||!v.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_'||b==b'-')){"query_value"}
        else if url.path().trim_start_matches('/').split('/').any(|p|p.is_empty()||!p.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'|b'.'))){"path_segment"}
        else if !["m3u8","mp4","m4s","ts","aac"].iter().any(|ext|url.path().ends_with(&format!(".{ext}"))){"extension"}
        else if matches!(family,"ext_tw_video"|"amplify_video")&&parts.get(1).is_some_and(|id|!valid_id(id)){"id"}
        else if family=="tweet_video"&&parts.len()==2&&!url.path().ends_with(".mp4"){"extension"}
        else{"kind"};
    let invalid_segment=match reason{
        "path_encoding"=>parts.iter().take(64).position(|p|p.contains('%')),
        "path_segment"=>bad_segment,
        "id"=>Some(1),
        "kind" if family=="other"=>Some(0),
        _=>None,
    };
    Some(json!({"reason":reason,"boundary":boundary,"queryKeys":query_keys,"queryKeysTruncated":queries_truncated,
        "pathFamily":family,"segmentCount":parts.len().min(64),"segmentsTruncated":parts.len()>64,"invalidSegmentIndex":invalid_segment}))
}
fn poster_scope(value:&str)->Option<String>{
    let url=media_url(value).ok()?;
    let parts=url.path().trim_start_matches('/').split('/').collect::<Vec<_>>();
    match parts.as_slice(){
        ["ext_tw_video_thumb",id,..] if valid_id(id)=>Some(format!("ext_tw_video/{id}")),
        ["amplify_video_thumb",id,..] if valid_id(id)=>Some(format!("amplify_video/{id}")),
        ["tweet_video_thumb",file]=>Some(format!("tweet_video/{}",file.split('.').next()?)),
        _=>None
    }
}
pub(super) fn observed(session:&XSession,url:&str,revision:u64){
    if session.revision.load(Ordering::Acquire)!=revision {return;}
    let Ok(url)=Url::parse(url) else{return};
    if scope(&url).is_none()||!matches!(url.path().rsplit('.').next(),Some("m3u8"|"mp4")){return;}
    let mut videos=session.videos.lock().unwrap_or_else(|p|p.into_inner());
    if session.revision.load(Ordering::Acquire)!=revision {return;}
    if !videos.observed.contains(&url){videos.observed.push_back(url);}
    while videos.observed.len()>400{videos.observed.pop_front();}
}
fn lease(videos:&mut Videos,url:Url,wanted:&str,generation:u64,revision:u64)->Result<String,String>{
    if scope(&url).as_deref()!=Some(wanted){return Err("동영상 리소스 범위를 확인하지 못했습니다.".into());}
    videos.resources.retain(|_,r|r.generation==generation&&r.revision==revision&&r.touched.elapsed()<Duration::from_secs(600));
    if let Some((id,_))=videos.resources.iter().find(|(_,r)|r.url==url&&r.generation==generation&&r.revision==revision){return Ok(format!("http://x-video.localhost/{id}"));}
    if videos.resources.len()>=4096{return Err("동영상 재생 리소스 한도에 도달했습니다.".into());}
    let id=uuid::Uuid::new_v4().to_string();
    videos.resources.insert(id.clone(),Resource{url,scope:wanted.into(),generation,revision,touched:Instant::now()});
    Ok(format!("http://x-video.localhost/{id}"))
}
#[derive(Serialize)]
pub struct Playback{url:String,hls:bool}
#[tauri::command(rename_all="camelCase")]
pub async fn x_video_open(window:Webview,state:State<'_,AppState>,session:State<'_,XSession>,post_id:String,generation:u64,request_id:Option<String>)->Result<Playback,String>{
    trusted(&window)?;state.begin_platform_work(||Ok(()))?;
    if !valid_id(&post_id){return Err(ERROR.into());}
    if let Ok(playback)=playback(&session,&post_id,generation){return Ok(playback);}
    let request_id=request_id.unwrap_or_else(||uuid::Uuid::new_v4().to_string());
    if uuid::Uuid::parse_str(&request_id).is_err(){return Err(ERROR.into());}
    let cancel=Arc::new(AtomicBool::new(false));
    {let mut pending=session.pending.lock().unwrap_or_else(|p|p.into_inner());if pending.len()>=8||pending.contains_key(&request_id){return Err("진행 중인 X 요청이 많습니다.".into());}pending.insert(request_id.clone(),cancel.clone());}
    let _lane=session.feed_lane.lock().await;
    let host=session.inner().clone();let app=window.app_handle().clone();
    let done=host.clone();
    let outcome=tauri::async_runtime::spawn_blocking(move||{
        if host.generation()!=generation{return Err(STALE.into());}
        let revision=host.revision.load(Ordering::Acquire);
        host.current(revision,&cancel)?;
        let feed=format!("post:{post_id}");
        let exists=host.data.lock().unwrap_or_else(|p|p.into_inner()).readers.get(&feed).is_some_and(|r|r.touched.elapsed()<READER_IDLE);
        if !exists{host.load(&app,&feed,None,&cancel)?;}
        let post=host.data.lock().unwrap_or_else(|p|p.into_inner()).posts.get(&post_id).filter(|p|p.video).cloned().ok_or("재생할 동영상을 확인하지 못했습니다.")?;
        let (label,owner)={let data=host.data.lock().unwrap_or_else(|p|p.into_inner());(data.readers.get(&feed).ok_or(STALE)?.label.clone(),data.owner.clone().ok_or(STALE)?)};
        let view=app.get_webview(&label).ok_or(STALE)?;
        let path=format!("/{}/status/{}",post.handle,post.id);
        if !view.url().is_ok_and(|u|allowed(&u)&&u.path()==path){return Err(STALE.into());}
        let mut last_intent=Value::Null;
        let mut intent_sent=false;
        for _ in 0..35{
            let nonce=uuid::Uuid::new_v4().to_string();
            let script=include_str!("x_video_intent.js").replace("__ATSUMI_X_VIDEO__",&json!({"path":path,"owner":owner,"nonce":nonce,"inspect":intent_sent}).to_string());
            let(tx,rx)=std::sync::mpsc::sync_channel(1);
            view.eval_with_callback(script,move|value|{let _=tx.send(value);}).map_err(|_|ERROR)?;
            let raw=rx.recv_timeout(Duration::from_secs(2)).map_err(|_|ERROR)?;
            host.current(revision,&cancel)?;
            if raw.len()>20000||!view.url().is_ok_and(|u|allowed(&u)&&u.path()==path){return Err(STALE.into());}
            let value:Value=serde_json::from_str(&raw).map_err(|_|ERROR)?;
            if value["nonce"]!=nonce{return Err(ERROR.into());}
            if value["state"]=="observed" && value["intentSent"]==true {intent_sent=true;}
            last_intent=json!({"state":value["state"],"intentSent":intent_sent,"video":value["video"],"ready":value["ready"],"error":value["error"],"posterCount":value["posters"].as_array().map(Vec::len),"sourceCount":value["sources"].as_array().map(Vec::len)});
            if value["state"]=="observed"{
                let posters=value["posters"].as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(|u|media_url(u).ok()).map(String::from).filter(|u|poster_scope(u).is_some()).take(4).collect::<Vec<_>>();
                if !posters.is_empty(){let mut data=host.data.lock().unwrap_or_else(|p|p.into_inner());host.current(revision,&cancel)?;if let Some(post)=data.posts.get_mut(&post_id){post.media=posters;}}
                let sources=value["sources"].as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(|u|Url::parse(u).ok()).filter(|u|scope(u).is_some()).take(4).collect::<Vec<_>>();
                if !sources.is_empty(){
                    let mut videos=host.videos.lock().unwrap_or_else(|p|p.into_inner());
                    host.current(revision,&cancel)?;
                    if videos.dom_scopes.len()<MAX_POSTS||videos.dom_scopes.contains_key(&post_id){videos.dom_scopes.insert(post_id.clone(),sources.iter().filter_map(scope).collect());}
                    for source in sources{if !videos.observed.contains(&source){videos.observed.push_back(source);}}
                    while videos.observed.len()>400{videos.observed.pop_front();}
                }
            }
            std::thread::sleep(Duration::from_millis(200));host.current(revision,&cancel)?;
            // X can create its video element/poster only after the explicit play
            // button is pressed. Match that validated DOM poster to an observed
            // resource; never infer a media URL from a post ID.
            let projected=host.evaluate(&app,&label,&feed,false,revision,&cancel)?;
            if projected["state"]=="ready" && projected["owner"].as_str()==Some(owner.as_str()) {
                if let Some(mut updated)=validated_posts(&projected["posts"])?.into_iter().find(|p|p.id==post_id&&p.video){
                    let mut data=host.data.lock().unwrap_or_else(|p|p.into_inner());
                    host.current(revision,&cancel)?;
                    retain_observed_posters(&mut data,std::slice::from_mut(&mut updated));
                    data.posts.insert(post_id.clone(),updated);
                }
            }
            if let Ok(playback)=playback(&host,&post_id,generation){return Ok(playback);}
        }
        let projected=host.evaluate(&app,&label,&feed,false,revision,&cancel)?;
        let (poster_scopes,observed_sources)={let data=host.data.lock().unwrap_or_else(|p|p.into_inner());let posters=data.posts.get(&post_id).map(|p|p.media.iter().filter_map(|u|poster_scope(u)).count()).unwrap_or(0);(posters,host.videos.lock().unwrap_or_else(|p|p.into_inner()).observed.len())};
        debug_shape("video","intent-unmatched",&json!({"intent":last_intent,"posterScopes":poster_scopes,"observedSources":observed_sources}));
        debug_shape("video","source-unmatched",&projected["shape"]);
        Err("현재 X에서 이 게시물의 재생 주소를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.".into())
    }).await.map_err(|_|ERROR.to_string()).and_then(|r|r);
    done.pending.lock().unwrap_or_else(|p|p.into_inner()).remove(&request_id);
    outcome
}
fn playback(session:&XSession,post_id:&str,generation:u64)->Result<Playback,String>{
    if session.generation()!=generation{return Err(STALE.into());}
    let mut scopes={
        let data=session.data.lock().unwrap_or_else(|p|p.into_inner());
        let post=data.posts.get(post_id).filter(|p|p.video).ok_or(STALE)?;
        post.media.iter().filter_map(|u|poster_scope(u)).collect::<Vec<_>>()
    };
    let mut videos=session.videos.lock().unwrap_or_else(|p|p.into_inner());
    if session.generation()!=generation{return Err(STALE.into());}
    if let Some(bound)=videos.dom_scopes.get(post_id){scopes.extend(bound.iter().cloned());}
    let source=videos.observed.iter().filter(|u|scope(u).is_some_and(|s|scopes.contains(&s))).max_by_key(|u|u.path().ends_with(".m3u8")).cloned()
        .ok_or("이 게시물의 재생 주소가 아직 확인되지 않았습니다. 상세를 불러온 뒤 다시 시도해 주세요.")?;
    let wanted=scope(&source).ok_or(ERROR)?;
    let hls=source.path().ends_with(".m3u8");
    let url=lease(&mut videos,source,&wanted,generation,session.revision.load(Ordering::Acquire))?;
    Ok(Playback{url,hls})
}
fn response(status:u16,body:Vec<u8>,mime:&str)->Response<Vec<u8>>{
    Response::builder().status(status).header("Content-Type",mime).header("Cache-Control","no-store")
        .header("Content-Length",body.len().to_string())
        .header("X-Content-Type-Options","nosniff").header("Access-Control-Allow-Origin","*")
        .header("Access-Control-Expose-Headers","Content-Length,Content-Range,Accept-Ranges")
        .body(body).expect("static media response")
}
fn rewrite_playlist(source:&str,base:&Url,resource:&Resource,videos:&mut Videos)->Result<String,String>{
    if !source.starts_with("#EXTM3U")||source.len() as u64>MAX_PLAYLIST_BYTES{return Err(ERROR.into());}
    let mut result=String::new();
    for line in source.lines(){
        if line.starts_with("#EXT-X-KEY")&&!line.contains("METHOD=NONE"){return Err("암호화된 동영상은 현재 재생할 수 없습니다.".into());}
        let mut next=line.to_owned();
        if !line.trim().is_empty()&&!line.starts_with('#'){
            next=lease(videos,base.join(line.trim()).map_err(|_|ERROR)?,&resource.scope,resource.generation,resource.revision)?;
        }else if let Some(start)=line.find("URI=\""){
            let offset=start+5;let end=line[offset..].find('"').ok_or(ERROR)?+offset;
            let url=lease(videos,base.join(&line[offset..end]).map_err(|_|ERROR)?,&resource.scope,resource.generation,resource.revision)?;
            next.replace_range(offset..end,&url);
        }
        result.push_str(&next);result.push('\n');
    }
    Ok(result)
}
#[derive(Clone,Copy)]
enum ProtocolStage {
    RequestPolicy,Lease,Range,Client,Fetch,HttpStatus,ContentRange,BodyLimit,BodyRead,BodyLength,Revoked,Playlist,ResponseHeaders,
}
impl ProtocolStage {
    fn name(self)->&'static str { match self {
        Self::RequestPolicy=>"request_policy",Self::Lease=>"lease",Self::Range=>"range",Self::Client=>"client",
        Self::Fetch=>"fetch",Self::HttpStatus=>"http_status",Self::ContentRange=>"content_range",Self::BodyLimit=>"body_limit",
        Self::BodyRead=>"body_read",Self::BodyLength=>"body_length",Self::Revoked=>"revoked",Self::Playlist=>"playlist",Self::ResponseHeaders=>"response_headers",
    } }
}
#[derive(Clone,Copy)]
enum ProtocolRangeKind { None,Unvalidated,From,Suffix }
impl ProtocolRangeKind {
    fn name(self)->&'static str {match self{Self::None=>"none",Self::Unvalidated=>"unvalidated",Self::From=>"from",Self::Suffix=>"suffix"}}
    fn validated(range:Option<RequestedRange>)->Self {match range{None=>Self::None,Some(RequestedRange::From{..})=>Self::From,Some(RequestedRange::Suffix(_))=>Self::Suffix}}
}
struct ProtocolDiagnostic {
    stage:ProtocolStage,http_status:Option<u16>,range_kind:ProtocolRangeKind,hls:Option<bool>,byte_count:u64,
}
impl ProtocolDiagnostic {
    fn shape(&self)->Value {
        // Typed, bounded scalars only. No request/response/error objects can
        // reach this log, including malformed URI/header values or lease IDs.
        json!({"stage":self.stage.name(),"httpStatus":self.http_status.filter(|s|(100..=599).contains(s)),
            "rangeKind":self.range_kind.name(),"hls":self.hls,"byteCount":self.byte_count.min(MAX_MEDIA_BYTES+1)})
    }
    fn failure(&self) {debug_shape("video","protocol-failure",&self.shape());}
}
// The URI handler can reject before respond() acquires its normal diagnostics.
// Record only the fixed category, never a request URL, token or account identity.
pub(crate) fn note_slot_unavailable() {
    debug_shape("video","slot-unavailable",&json!({"stage":"admission"}));
}
pub fn respond(session:&XSession,request:Request<Vec<u8>>,label:&str)->Response<Vec<u8>>{
    let mut diagnostic=ProtocolDiagnostic{stage:ProtocolStage::RequestPolicy,http_status:None,
        range_kind:if request.headers().contains_key("range"){ProtocolRangeKind::Unvalidated}else{ProtocolRangeKind::None},hls:None,byte_count:0};
    if label!="main"||request.method()!="GET"||request.uri().query().is_some(){diagnostic.failure();return response(403,vec![],"text/plain");}
    diagnostic.stage=ProtocolStage::Lease;
    let id=request.uri().path().trim_start_matches('/');
    if uuid::Uuid::parse_str(id).is_err(){diagnostic.failure();return response(404,vec![],"text/plain");}
    let resource={
        let mut videos=session.videos.lock().unwrap_or_else(|p|p.into_inner());
        let Some(r)=videos.resources.get_mut(id).filter(|r|r.generation==session.generation()&&r.revision==session.revision.load(Ordering::Acquire)&&r.touched.elapsed()<Duration::from_secs(600))else{diagnostic.failure();return response(410,vec![],"text/plain")};
        r.touched=Instant::now();r.clone()
    };
    let outcome=(||->Result<Response<Vec<u8>>,String>{
        let hls=resource.url.path().ends_with(".m3u8");
        diagnostic.hls=Some(hls);diagnostic.stage=ProtocolStage::Range;
        let requested_range=if hls{None}else{request_range(request.headers())?};
        diagnostic.range_kind=ProtocolRangeKind::validated(requested_range);diagnostic.stage=ProtocolStage::Client;
        let client=reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).redirect(reqwest::redirect::Policy::none()).build().map_err(|_|ERROR)?;
        let mut get=client.get(resource.url.clone()).header("Referer","https://x.com/").header("Accept-Encoding","identity");
        if let Some(range)=requested_range{get=get.header("Range",range_header(range));}
        diagnostic.stage=ProtocolStage::Fetch;
        let fetched=get.send().map_err(|_|ERROR)?;
        let status=fetched.status().as_u16();
        diagnostic.http_status=Some(status);
        let content_range=fetched.headers().get("content-range").and_then(|h|h.to_str().ok()).map(str::to_owned);
        if status==416{
            diagnostic.stage=ProtocolStage::ContentRange;
            let total=unsatisfied_range(requested_range.ok_or(ERROR)?,content_range.as_deref().ok_or(ERROR)?)?;
            diagnostic.stage=ProtocolStage::Revoked;
            if session.generation()!=resource.generation||session.revision.load(Ordering::Acquire)!=resource.revision{return Err(STALE.into());}
            diagnostic.stage=ProtocolStage::ResponseHeaders;
            let mut result=response(416,vec![],"text/plain");
            result.headers_mut().insert("Content-Range",format!("bytes */{total}").parse().map_err(|_|ERROR)?);
            result.headers_mut().insert("Accept-Ranges","bytes".parse().unwrap());
            return Ok(result);
        }
        diagnostic.stage=if matches!(status,200|206){ProtocolStage::ContentRange}else{ProtocolStage::HttpStatus};
        let partial=response_range(requested_range,status,content_range.as_deref())?;
        let limit=if hls{MAX_PLAYLIST_BYTES}else{MAX_MEDIA_BYTES};
        let declared=fetched.content_length();
        diagnostic.stage=if declared.is_some_and(|n|n>limit){ProtocolStage::BodyLimit}else{ProtocolStage::BodyLength};
        if declared.is_some_and(|n|n>limit||partial.is_some_and(|range|range.length()!=n)){return Err(ERROR.into());}
        diagnostic.stage=ProtocolStage::BodyRead;
        let mut bytes=Vec::new();let read=fetched.take(limit+1).read_to_end(&mut bytes);
        diagnostic.byte_count=bytes.len() as u64;read.map_err(|_|ERROR)?;
        diagnostic.stage=if bytes.len() as u64>limit{ProtocolStage::BodyLimit}else{ProtocolStage::BodyLength};
        if !valid_body_length(bytes.len() as u64,declared,partial,limit){return Err(ERROR.into());}
        diagnostic.stage=ProtocolStage::Revoked;
        if session.generation()!=resource.generation||session.revision.load(Ordering::Acquire)!=resource.revision{return Err(STALE.into());}
        let mime=if hls{
            let mut videos=session.videos.lock().unwrap_or_else(|p|p.into_inner());
            if session.revision.load(Ordering::Acquire)!=resource.revision{return Err(STALE.into());}
            diagnostic.stage=ProtocolStage::Playlist;
            bytes=rewrite_playlist(std::str::from_utf8(&bytes).map_err(|_|ERROR)?,&resource.url,&resource,&mut videos)?.into_bytes();
            "application/vnd.apple.mpegurl"
        }else if resource.url.path().ends_with(".ts"){"video/mp2t"}else{"video/mp4"};
        diagnostic.stage=ProtocolStage::ResponseHeaders;
        let mut result=response(status,bytes,mime);
        if let Some(range)=partial{result.headers_mut().insert("Content-Range",range.header().parse().map_err(|_|ERROR)?);}
        result.headers_mut().insert("Accept-Ranges","bytes".parse().unwrap());
        Ok(result)
    })();
    outcome.unwrap_or_else(|_|{diagnostic.failure();response(502,vec![],"text/plain")})
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn protocol_diagnostic_shape_is_fixed_and_bounded(){
        let diagnostic=ProtocolDiagnostic{stage:ProtocolStage::BodyRead,http_status:Some(206),range_kind:ProtocolRangeKind::From,hls:Some(false),byte_count:u64::MAX};
        assert_eq!(diagnostic.shape(),json!({"stage":"body_read","httpStatus":206,"rangeKind":"from","hls":false,"byteCount":MAX_MEDIA_BYTES+1}));
        assert_eq!(diagnostic.shape().as_object().unwrap().len(),5);
        for status in [0,99,600,u16::MAX] {
            let diagnostic=ProtocolDiagnostic{stage:ProtocolStage::Fetch,http_status:Some(status),range_kind:ProtocolRangeKind::Unvalidated,hls:None,byte_count:0};
            let shape=diagnostic.shape();assert!(shape["httpStatus"].is_null());assert!(shape["hls"].is_null());
            assert_eq!(shape["rangeKind"],"unvalidated");assert_eq!(shape["byteCount"],0);
        }
    }
    #[test]fn protocol_diagnostic_stage_and_range_names_are_closed(){
        for (stage,name) in [
            (ProtocolStage::RequestPolicy,"request_policy"),(ProtocolStage::Lease,"lease"),(ProtocolStage::Range,"range"),
            (ProtocolStage::Client,"client"),(ProtocolStage::Fetch,"fetch"),(ProtocolStage::HttpStatus,"http_status"),
            (ProtocolStage::ContentRange,"content_range"),(ProtocolStage::BodyLimit,"body_limit"),(ProtocolStage::BodyRead,"body_read"),
            (ProtocolStage::BodyLength,"body_length"),(ProtocolStage::Revoked,"revoked"),(ProtocolStage::Playlist,"playlist"),(ProtocolStage::ResponseHeaders,"response_headers"),
        ] {assert_eq!(stage.name(),name);}
        assert_eq!(ProtocolRangeKind::validated(None).name(),"none");
        assert_eq!(ProtocolRangeKind::validated(Some(RequestedRange::From{start:123456789,end:123456790})).name(),"from");
        assert_eq!(ProtocolRangeKind::validated(Some(RequestedRange::Suffix(987654321))).name(),"suffix");
        let diagnostic=ProtocolDiagnostic{stage:ProtocolStage::ContentRange,http_status:Some(206),range_kind:ProtocolRangeKind::validated(Some(RequestedRange::From{start:123456789,end:123456790})),hls:Some(false),byte_count:2};
        let serialized=diagnostic.shape().to_string();
        for forbidden in ["123456789","123456790","987654321","url","uri","token","account","postId","lease","error","contentText"] {assert!(!serialized.contains(forbidden),"{forbidden}");}
    }
    #[test]fn observed_video_version_keys_preserve_canonical_scope(){
        for family in ["ext_tw_video","amplify_video"] {
            for query in ["tag=12&variant_version=1","tag=12&v=1&variant_version=1","container=fmp4&v=release_A-1&variant_version=revision_B-2"] {
                let url=Url::parse(&format!("https://video.twimg.com/{family}/123456789/pu/pl/movie.m3u8?{query}")).unwrap();
                assert_eq!(scope(&url),Some(format!("{family}/123456789")));
                assert!(rejected_scope_shape(&url).is_none());
            }
        }
    }
    #[test]fn video_version_keys_keep_value_limits_and_reject_unknown_keys(){
        let base="https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8";
        for key in ["tag","container","v","variant_version"] {
            let url=Url::parse(&format!("{base}?{key}={}","A".repeat(40))).unwrap();
            assert!(scope(&url).is_some());assert!(rejected_scope_shape(&url).is_none());
            for value in ["A".repeat(41),"a%2Fb".into(),"a%26b".into(),"a+b".into(),"a%21b".into(),"%EB%B9%84%EB%B0%80".into()] {
                let url=Url::parse(&format!("{base}?{key}={value}")).unwrap();
                assert!(scope(&url).is_none());assert_eq!(rejected_scope_shape(&url).unwrap()["reason"],"query_value");
            }
        }
        for key in ["version","variant","V","Variant_version","variant_version_extra","token","url",""] {
            let url=Url::parse(&format!("{base}?tag=12&v=1&variant_version=1&{key}=value")).unwrap();
            assert!(scope(&url).is_none());assert_eq!(rejected_scope_shape(&url).unwrap()["reason"],"query_key");
        }
    }
    #[test]fn video_version_keys_do_not_bypass_url_boundaries(){
        for address in [
            "http://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8",
            "https://video.twimg.com.example/ext_tw_video/123456789/pu/pl/movie.m3u8",
            "https://video.twimg.com:8443/ext_tw_video/123456789/pu/pl/movie.m3u8",
            "https://user@video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8",
            "https://video.twimg.com/ext_tw_video/not_numeric/pu/pl/movie.m3u8",
            "https://video.twimg.com/other_family/123456789/pu/pl/movie.m3u8",
            "https://video.twimg.com/ext_tw_video/123456789/pu/pl/a%2Fb.m3u8",
            "https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.txt",
        ] {
            let url=Url::parse(&format!("{address}?tag=12&v=1&variant_version=1")).unwrap();
            assert!(scope(&url).is_none());assert!(rejected_scope_shape(&url).is_some());
        }
        let url=Url::parse("https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8?tag=12&v=1&variant_version=1#fragment").unwrap();
        assert!(scope(&url).is_none());assert_eq!(rejected_scope_shape(&url).unwrap()["boundary"],"fragment");
        let url=Url::parse(&format!("https://video.twimg.com/ext_tw_video/123456789/{}/movie.m3u8?tag=12&v=1&variant_version=1","a".repeat(2048))).unwrap();
        assert!(scope(&url).is_none());assert_eq!(rejected_scope_shape(&url).unwrap()["boundary"],"length");
    }
    #[test]fn rejected_scope_diagnostics_classify_without_relaxing_policy(){
        for (url,reason) in [
            ("http://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8","uri_boundary"),
            ("https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8?unexpected=value","query_key"),
            ("https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8?tag=bad%21","query_value"),
            ("https://video.twimg.com/ext_tw_video/123456789/pu/pl/a%2Fb.m3u8","path_encoding"),
            ("https://video.twimg.com/ext_tw_video/123456789/pu/pl/a!b.m3u8","path_segment"),
            ("https://video.twimg.com/other_family/123456789/pu/pl/movie.m3u8","kind"),
            ("https://video.twimg.com/ext_tw_video/not_numeric/pu/pl/movie.m3u8","id"),
            ("https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.txt","extension"),
        ]{
            let parsed=Url::parse(url).unwrap();
            assert!(scope(&parsed).is_none());
            assert_eq!(rejected_scope_shape(&parsed).unwrap()["reason"],reason);
        }
        for url in [
            "https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8?tag=12&container=fmp4",
            "https://video.twimg.com/tweet_video/fixture.mp4",
        ]{assert!(rejected_scope_shape(&Url::parse(url).unwrap()).is_none());}
    }
    #[test]fn rejected_scope_diagnostics_never_include_private_address_fields(){
        let url=Url::parse("https://PrivateUser:PrivatePassword@video.twimg.com/ext_tw_video/123456789/pu/pl/PrivateFilename.m3u8?auth_token=PrivateQueryValue&tag=PrivateTagValue#PrivateFragment").unwrap();
        let shape=rejected_scope_shape(&url).unwrap();
        assert_eq!(shape["reason"],"uri_boundary");assert_eq!(shape["boundary"],"userinfo");
        assert_eq!(shape["queryKeys"],json!(["auth_token","tag"]));
        assert_eq!(shape["pathFamily"],"ext_tw_video");
        let serialized=shape.to_string();
        for forbidden in ["PrivateUser","PrivatePassword","123456789","PrivateFilename","PrivateQueryValue","PrivateTagValue","PrivateFragment","https://","video.twimg.com"]{
            assert!(!serialized.contains(forbidden));
        }
        assert!(shape.as_object().unwrap().keys().all(|key|matches!(key.as_str(),"reason"|"boundary"|"queryKeys"|"queryKeysTruncated"|"pathFamily"|"segmentCount"|"segmentsTruncated"|"invalidSegmentIndex")));
    }
    #[test]fn rejected_scope_key_and_path_diagnostics_are_strictly_bounded(){
        let url=Url::parse("https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8?one=Secret&two=Secret&three=Secret&four=Secret&five=Secret&six=Secret&seven=Secret&eight=Secret&nine=Secret").unwrap();
        let shape=rejected_scope_shape(&url).unwrap();
        assert_eq!(shape["queryKeys"].as_array().unwrap().len(),8);assert_eq!(shape["queryKeysTruncated"],true);
        assert!(!shape.to_string().contains("Secret"));assert!(!shape.to_string().contains("nine"));
        let url=Url::parse(&format!("https://video.twimg.com/ext_tw_video/123456789/pu/pl/movie.m3u8?bad1=Secret&%EB%B9%84%EB%B0%80=Secret&{}=Secret&safe_key=Secret","a".repeat(33))).unwrap();
        assert_eq!(rejected_scope_shape(&url).unwrap()["queryKeys"],json!(["safe_key"]));
        let url=Url::parse(&format!("https://video.twimg.com/other_family/{}movie.m3u8","a/".repeat(100))).unwrap();
        let shape=rejected_scope_shape(&url).unwrap();
        assert_eq!(shape["segmentCount"],64);assert_eq!(shape["segmentsTruncated"],true);
        assert_eq!(shape["invalidSegmentIndex"],0);
        let url=Url::parse(&format!("https://video.twimg.com/ext_tw_video/123456789/{}bad!name.m3u8","a/".repeat(70))).unwrap();
        let shape=rejected_scope_shape(&url).unwrap();
        assert_eq!(shape["reason"],"path_segment");assert!(shape["invalidSegmentIndex"].is_null());
    }
    #[test]fn single_ranges_preserve_suffix_and_explicit_bounds(){
        assert_eq!(parse_range("bytes=-1024"),Ok(RequestedRange::Suffix(1024)));
        assert_eq!(parse_range("bytes=10-19"),Ok(RequestedRange::From{start:10,end:19}));
        assert_eq!(parse_range("bytes=0-0"),Ok(RequestedRange::From{start:0,end:0}));
        assert_eq!(parse_range("bytes=10-"),Ok(RequestedRange::From{start:10,end:10+OPEN_RANGE_CHUNK_BYTES-1}));
        // hls.js consumes an explicit byte range as one complete fragment.
        // Its loader does not fetch the remainder if a proxy silently clips it.
        for count in [5*1024*1024,MAX_MEDIA_BYTES]{
            let header=format!("bytes=0-{}",count-1);
            assert_eq!(range_header(parse_range(&header).unwrap()),header);
            let suffix=format!("bytes=-{count}");
            assert_eq!(range_header(parse_range(&suffix).unwrap()),suffix);
        }
    }
    #[test]fn invalid_overflowing_or_oversized_ranges_fail_without_clipping(){
        for value in [
            "", "bytes=", "bytes=-", "bytes=-0", "bytes=2-1", "bytes=1-2,4-5",
            "bytes=+1-2", "bytes=1-+2", "bytes= 1-2", "bytes=1-2 ", "bytes=1--2",
            "bytes=0-16777216", "bytes=-16777217", "bytes=18446744073709551616-",
            "bytes=0-18446744073709551615", "bytes=18446744073709551615-",
        ]{assert!(parse_range(value).is_err(),"{value}");}
        assert!(parse_range(&format!("bytes={}-1","0".repeat(65))).is_err());
    }
    #[test]fn duplicate_or_non_text_range_headers_fail_closed(){
        let mut headers=tauri::http::HeaderMap::new();
        assert_eq!(request_range(&headers),Ok(None));
        headers.append("range","bytes=-5".parse().unwrap());
        assert_eq!(request_range(&headers),Ok(Some(RequestedRange::Suffix(5))));
        headers.append("range","bytes=0-4".parse().unwrap());
        assert!(request_range(&headers).is_err());
        headers.clear();
        headers.insert("range",tauri::http::HeaderValue::from_bytes(&[0xff]).unwrap());
        assert!(request_range(&headers).is_err());
    }
    #[test]fn partial_responses_must_match_the_requested_bytes_and_eof(){
        let closed=RequestedRange::From{start:10,end:19};
        assert_eq!(partial_range(closed,"bytes 10-19/100"),Ok(PartialRange{start:10,end:19,total:100}));
        assert_eq!(partial_range(closed,"bytes 10-14/15"),Ok(PartialRange{start:10,end:14,total:15}));
        assert_eq!(partial_range(RequestedRange::Suffix(5),"bytes 95-99/100").unwrap().length(),5);
        assert_eq!(partial_range(RequestedRange::Suffix(100),"bytes 0-49/50").unwrap().length(),50);
        let open=parse_range("bytes=0-").unwrap();
        assert_eq!(partial_range(open,&format!("bytes 0-{}/5000000",OPEN_RANGE_CHUNK_BYTES-1)).unwrap().length(),OPEN_RANGE_CHUNK_BYTES);
        for value in [
            "bytes 0-9/100", "bytes 10-18/100", "bytes 10-20/100", "bytes 10-19/*",
            "bytes 10-19/19", "bytes 10-9/100", "bytes 10-19/0", "bytes 10-19/100,200",
            "bytes +10-19/100", "bytes 10-19/18446744073709551616",
        ]{assert!(partial_range(closed,value).is_err(),"{value}");}
        assert!(partial_range(RequestedRange::Suffix(5),"bytes 0-4/100").is_err());
        assert!(partial_range(RequestedRange::Suffix(5),"bytes 0-0/0").is_err());
    }
    #[test]fn explicit_hls_fragment_larger_than_four_mib_is_not_truncated(){
        let range=parse_range("bytes=0-5242879").unwrap();
        let partial=partial_range(range,"bytes 0-5242879/6291456").unwrap();
        assert!(valid_body_length(5*1024*1024,Some(5*1024*1024),Some(partial),MAX_MEDIA_BYTES));
        assert!(partial_range(range,"bytes 0-4194303/6291456").is_err());
        assert!(!valid_body_length(4*1024*1024,Some(5*1024*1024),Some(partial),MAX_MEDIA_BYTES));
    }
    #[test]fn successful_status_alone_cannot_validate_a_partial_response(){
        let range=Some(RequestedRange::From{start:10,end:19});
        assert!(response_range(range,206,Some("bytes 10-19/100")).is_ok());
        assert!(response_range(range,206,None).is_err());
        assert!(response_range(range,200,None).is_err());
        assert!(response_range(None,206,Some("bytes 10-19/100")).is_err());
        assert!(response_range(None,200,Some("bytes 10-19/100")).is_err());
        assert!(response_range(None,204,None).is_err());
        assert_eq!(response_range(None,200,None),Ok(None));
    }
    #[test]fn declared_and_received_lengths_must_match_within_existing_limits(){
        let partial=PartialRange{start:10,end:19,total:100};
        assert!(valid_body_length(10,Some(10),Some(partial),MAX_MEDIA_BYTES));
        assert!(valid_body_length(10,None,Some(partial),MAX_MEDIA_BYTES));
        assert!(!valid_body_length(9,None,Some(partial),MAX_MEDIA_BYTES));
        assert!(!valid_body_length(10,Some(9),Some(partial),MAX_MEDIA_BYTES));
        assert!(!valid_body_length(MAX_MEDIA_BYTES+1,None,None,MAX_MEDIA_BYTES));
        assert!(!valid_body_length(MAX_PLAYLIST_BYTES+1,None,None,MAX_PLAYLIST_BYTES));
        let result=response(206,vec![0;10],"video/mp4");
        assert_eq!(result.headers()["Content-Length"],"10");
    }
    #[test]fn unsatisfied_ranges_keep_416_only_for_proven_eof(){
        assert_eq!(unsatisfied_range(RequestedRange::From{start:100,end:109},"bytes */100"),Ok(100));
        assert_eq!(unsatisfied_range(RequestedRange::Suffix(5),"bytes */0"),Ok(0));
        assert!(unsatisfied_range(RequestedRange::From{start:0,end:9},"bytes */100").is_err());
        assert!(unsatisfied_range(RequestedRange::Suffix(5),"bytes */100").is_err());
        for value in ["bytes */*","bytes */-1","bytes */100 other","bytes */18446744073709551616"]{
            assert!(unsatisfied_range(RequestedRange::Suffix(5),value).is_err());
        }
    }
    #[test]fn hls_relative_variants_maps_and_byte_ranges_stay_in_the_original_scope(){
        let base=Url::parse("https://video.twimg.com/ext_tw_video/123456789/pu/pl/root.m3u8").unwrap();
        let resource=Resource{url:base.clone(),scope:"ext_tw_video/123456789".into(),generation:2,revision:3,touched:Instant::now()};
        let source="#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",URI=\"../aud/track.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=1500000,AUDIO=\"audio\"\n../vid/720p.m3u8\n#EXT-X-MAP:URI=\"../vid/init.mp4\"\n#EXT-X-BYTERANGE:5242880@0\n../vid/segment.m4s\n";
        let mut videos=Videos::default();
        let rewritten=rewrite_playlist(source,&base,&resource,&mut videos).unwrap();
        assert!(rewritten.contains("#EXT-X-BYTERANGE:5242880@0\n"));
        assert!(rewritten.contains("URI=\"http://x-video.localhost/"));
        assert_eq!(videos.resources.len(),4);
        assert!(videos.resources.values().all(|r|r.scope==resource.scope&&r.generation==2&&r.revision==3));
        assert!(videos.resources.values().any(|r|r.url.path().ends_with("/pu/aud/track.m3u8")));
        assert!(videos.resources.values().any(|r|r.url.path().ends_with("/pu/vid/segment.m4s")));
        for source in [
            "#EXTM3U\n#EXT-X-MAP:URI=\"//evil.test/init.mp4\"\n",
            "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"/ext_tw_video/999999999/pu/a.m3u8\"\n",
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"../vid/key.mp4\"\n",
        ]{assert!(rewrite_playlist(source,&base,&resource,&mut Videos::default()).is_err());}
    }
    #[test]fn no_arbitrary_media_or_cross_video_resources(){
        assert_eq!(scope(&Url::parse("https://video.twimg.com/ext_tw_video/123456789/pu/pl/test.m3u8?tag=12&container=fmp4").unwrap()).as_deref(),Some("ext_tw_video/123456789"));
        for url in ["http://video.twimg.com/ext_tw_video/123456789/a.mp4","https://video.twimg.com.evil.test/ext_tw_video/123456789/a.mp4","https://video.twimg.com/ext_tw_video/123456789/a.mp4?token=secret","https://video.twimg.com/ext_tw_video/123456789/a.html"]{assert!(scope(&url.parse().unwrap()).is_none());}
        let base=Url::parse("https://video.twimg.com/ext_tw_video/123456789/pu/pl/root.m3u8").unwrap();
        let r=Resource{url:base.clone(),scope:"ext_tw_video/123456789".into(),generation:1,revision:1,touched:Instant::now()};
        let mut videos=Videos::default();
        assert!(rewrite_playlist("#EXTM3U\n../vid/test.mp4\n",&base,&r,&mut videos).unwrap().contains("http://x-video.localhost/"));
        assert!(rewrite_playlist("#EXTM3U\nhttps://evil.test/a.mp4\n",&base,&r,&mut videos).is_err());
        assert!(rewrite_playlist("#EXTM3U\n/ext_tw_video/999999999/pu/pl/a.m3u8\n",&base,&r,&mut videos).is_err());
    }
}
