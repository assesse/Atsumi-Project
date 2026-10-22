//! YouTube-only canonical document and bridge admission. No foreign account state.
use std::time::{Duration, Instant};
use tauri::Url;
#[path = "original_account_regions.rs"]
mod account_regions;
pub(super) const PREFIX: &str = "ATSUMI_YOUTUBE_V1:";
pub(super) const MAX_MESSAGE: usize = 190_000;
pub(super) fn valid_channel(id: &str) -> bool { id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'_'|b'-')) }
fn https(url:&Url)->bool { url.scheme()=="https" && url.port().is_none() && url.username().is_empty() && url.password().is_none() }
pub(super) fn player_url(video:&str,_:&str)->Result<Url,String> {
    if !valid_channel(video) { return Err("YouTube 영상 ID를 확인해 주세요.".into()); }
    format!("https://www.youtube.com/watch?v={video}&autoplay=1").parse().map_err(|_|"YouTube 주소 오류".into())
}
pub(super) fn chat_url(video:&str)->Result<Url,String> {
    if !valid_channel(video) { return Err("YouTube 영상 ID를 확인해 주세요.".into()); }
    // Select YouTube's matching server/client palette for this app-owned
    // document; never modify the account's persisted theme preference.
    format!("https://www.youtube.com/live_chat?v={video}&is_popout=1&theme=dark").parse().map_err(|_|"YouTube 채팅 주소 오류".into())
}
pub(super) fn matches_document(url:&Url,video:&str,chat:bool)->bool {
    https(url)&&url.host_str()==Some("www.youtube.com")&&url.path()==if chat {"/live_chat"} else {"/watch"}
        &&url.query_pairs().filter(|(key,_)|key=="v").count()==1&&url.query_pairs().any(|(key,value)|key=="v"&&value==video)
        &&(!chat||url.fragment().is_none()&&url.query_pairs().count()==3
            &&url.query_pairs().filter(|(key,value)|key=="is_popout"&&value=="1").count()==1
            &&url.query_pairs().filter(|(key,value)|key=="theme"&&value=="dark").count()==1)
}
pub(super) fn matches_player(url:&Url,video:&str,broadcast:&str)->bool {video==broadcast&&matches_document(url,video,false)}
pub(super) fn account_url(url:&Url)->bool {
    https(url)&&match url.host_str() {
        Some("accounts.google.com"|"accounts.youtube.com"|"consent.google.com"|"consent.youtube.com")=>true,
        Some("www.youtube.com")=>matches!(url.path(),"/"|"/signin"|"/logout"|"/account"|"/oops"),
        _=>false,
    }
}
/// Account-window-only interoperability. Never reuse this for player popups,
/// bridge admission or completion; the common account_url stays unchanged.
fn account_window_url(url:&Url)->bool {
    account_url(url)||(https(url)&&url.path()=="/accounts/SetSID"
        &&url.host_str().is_some_and(account_regions::is_known_regional_account_host))
}
pub(super) fn account_entry(sign_out:bool)->Url {
    // Let YouTube supply its current Google continuation/session handoff.
    Url::parse(if sign_out {"https://www.youtube.com/logout"} else {"https://www.youtube.com/signin"}).expect("static official account URL")
}
/// Fixed diagnostic categories only; never return URL/query/path text.
pub(super) fn account_stage(url:&Url)->&'static str {
    if !https(url){return "non-https";}
    match (url.host_str(),url.path()) {
        (Some("www.youtube.com"),"/")=>"youtube-home",
        (Some("www.youtube.com"),"/signin")=>"youtube-signin",
        (Some("www.youtube.com"),"/logout")=>"youtube-logout",
        (Some("www.youtube.com"),"/check_connection")=>"youtube-connection",
        (Some("www.google.com"|"www.google.co.kr"),"/accounts/SetSID")=>"google-transfer",
        (Some("accounts.google.com"),_)=>"google-account",
        (Some(host),path) if account_regions::is_known_regional_account_host(host)=> {
            if path=="/accounts/SetSID" {"google-regional-transfer"} else {"google-regional-account-other"}
        },
        (Some("accounts.youtube.com"),_)=>"youtube-account",
        (Some("consent.google.com"|"consent.youtube.com"),_)=>"consent",
        _=>"other",
    }
}
pub(super) fn popup_url(url:&Url)->bool {
    account_url(url)||https(url)&&matches!(url.host_str(),Some("www.youtube.com"|"payments.google.com"|"pay.google.com"))
        &&!matches!(url.path(),"/watch"|"/embed"|"/live_chat")
}
pub(super) fn account_home(url:&Url)->bool{
    https(url)&&url.host_str()==Some("www.youtube.com")&&url.path()=="/"
        &&url.query().is_none()&&url.fragment().is_none()
}
pub(super) fn account_completed(url:&Url,observation:&serde_json::Value,sign_out:bool)->bool{
    account_home(url)
        &&observation["authenticated"].as_bool()==Some(!sign_out)&&observation["interactive"].as_bool()==Some(false)
}
/// A short-lived observer, not a login timeout: expiry leaves the window alone.
pub(super) const ACCOUNT_PROBE_LIMIT:u16=120;
#[derive(Default)]
pub(super) struct AccountFlow {
    nonce:Option<String>, epoch:u64, sign_out:bool,
    worker_epoch:Option<u64>, confirmed:Option<bool>,
    diagnostic_count:u16,
}
impl AccountFlow {
    pub fn begin(&mut self,nonce:String,sign_out:bool){
        self.nonce=Some(nonce);self.sign_out=sign_out;self.advance();self.confirmed=None;self.diagnostic_count=0;
    }
    fn advance(&mut self){self.epoch=self.epoch.saturating_add(1);self.worker_epoch=None;}
    pub fn nonce(&self)->Option<String>{self.nonce.clone()}
    pub fn diagnostic(&mut self,nonce:&str)->bool{
        if self.nonce.as_deref()!=Some(nonce)||self.diagnostic_count>=64{return false;}
        self.diagnostic_count+=1;true
    }
    /// A fixed diagnostic category only, never navigation or popup permission.
    pub fn popup_diagnostic(&mut self,nonce:&str,url:&Url)->Option<&'static str>{
        self.diagnostic(nonce).then(||account_stage(url))
    }
    pub fn intent(&mut self,nonce:&str,sign_out:bool)->Option<bool>{
        if self.nonce.as_deref()!=Some(nonce){return None;}
        let changed=self.sign_out!=sign_out;
        self.sign_out=sign_out;self.advance();self.confirmed=None;Some(changed)
    }
    pub fn navigate(&mut self,nonce:&str)->Option<u64>{
        if self.nonce.as_deref()!=Some(nonce){return None;}
        self.advance();Some(self.epoch)
    }
    /// Only the captured, still-current account window may use its own policy.
    pub fn navigate_account_window(&mut self,nonce:&str,url:&Url)->bool{
        account_window_url(url)&&self.navigate(nonce).is_some()
    }
    pub fn current(&self,nonce:&str,epoch:u64)->bool{
        self.nonce.as_deref()==Some(nonce)&&self.epoch==epoch
    }
    pub fn start_probe(&mut self,nonce:&str)->Option<u64>{
        if self.nonce.as_deref()!=Some(nonce)||self.worker_epoch==Some(self.epoch){return None;}
        self.worker_epoch=Some(self.epoch);Some(self.epoch)
    }
    pub fn probe_active(&self,nonce:&str,epoch:u64,started:Instant,attempts:u16,now:Instant)->bool{
        self.current(nonce,epoch)&&self.worker_epoch==Some(epoch)&&attempts<ACCOUNT_PROBE_LIMIT
            &&now.saturating_duration_since(started)<Duration::from_secs(120)
    }
    pub fn observe(&mut self,nonce:&str,epoch:u64,value:bool)->Option<bool>{
        if !self.current(nonce,epoch){return None;}
        let changed=self.confirmed!=Some(value);self.confirmed=Some(value);Some(changed)
    }
    pub fn is_sign_out(&self,nonce:&str,epoch:u64)->Option<bool>{
        self.current(nonce,epoch).then_some(self.sign_out)
    }
    pub fn invalidate(&mut self,nonce:&str){
        if self.nonce.as_deref()==Some(nonce){self.nonce=None;self.advance();self.confirmed=None;}
    }
}
#[derive(Clone,Debug)]pub(super) struct Arm { pub id:String,pub epoch:u64,pub created:Instant }
impl Arm { pub fn accepts(&self,id:&str,epoch:u64,now:Instant)->bool { self.id==id&&self.epoch==epoch&&now.saturating_duration_since(self.created)<=Duration::from_secs(25) } }
pub(super) fn ack_expired(since:Instant,now:Instant,stopping:bool)->bool{now.saturating_duration_since(since)>=Duration::from_secs(if stopping{15}else{25})}
pub(super) fn sequence_newer(last:u64,incoming:u64)->bool{incoming>last&&incoming<=9_007_199_254_740_991}
#[derive(Default)]pub(super) struct AccountObservation{pub value:Option<bool>,pub revision:u64}
impl AccountObservation{
    pub fn confirm(&mut self,value:bool){self.revision=self.revision.saturating_add(1);self.value=Some(value);}
    pub fn observe(&mut self,revision:u64,value:bool){if revision==self.revision{self.value=Some(value);}}
}
pub(super) fn confirmed_ended(player:&serde_json::Value,video:&str)->bool{
    let live=&player["microformat"]["playerMicroformatRenderer"]["liveBroadcastDetails"];
    player["videoDetails"]["videoId"].as_str()==Some(video)
        && player["videoDetails"]["isLive"].as_bool()!=Some(true)
        && live["isLiveNow"].as_bool()==Some(false)
        && live["endTimestamp"].as_str().is_some_and(|stamp|stamp.len()<64&&chrono::DateTime::parse_from_rfc3339(stamp).is_ok())
}
pub(super) fn validate_envelope(source:&str,label:&str,expected:&str,video:&str,epoch:u64,current:u64,body:&str)->bool {
    label==expected&&epoch>0&&epoch==current&&body.len()<=MAX_MESSAGE&&Url::parse(source).ok().is_some_and(|u|matches_document(&u,video,false))
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn exact_documents_and_case_sensitive_video_identity(){
        let id="AbCdEf123_-";let url=player_url(id,id).unwrap();assert!(matches_player(&url,id,id));assert!(!matches_player(&url,"abcdef123_-","abcdef123_-"));
        assert!(matches_document(&chat_url(id).unwrap(),id,true));
        for text in ["https://www.youtube.com.evil.test/watch?v=AbCdEf123_-","http://www.youtube.com/watch?v=AbCdEf123_-","https://user@www.youtube.com/watch?v=AbCdEf123_-","https://www.youtube.com/watch?v=AbCdEf123_-&v=Other123456","https://www.youtube.com/shorts/AbCdEf123_-"]{assert!(!matches_document(&text.parse().unwrap(),id,false));}
    }
    #[test]fn chat_palette_uses_official_document_option_and_exact_query_identity(){
        let id="AbCdEf123_-";
        assert_eq!(chat_url(id).unwrap().as_str(),"https://www.youtube.com/live_chat?v=AbCdEf123_-&is_popout=1&theme=dark");
        for text in [
            "https://www.youtube.com/live_chat?v=AbCdEf123_-&is_popout=1&theme=dark",
            "https://www.youtube.com/live_chat?theme=dark&v=AbCdEf123_-&is_popout=1",
            "https://www.youtube.com/live_chat?is_popout=1&theme=dark&v=AbCdEf123_-",
        ]{assert!(matches_document(&text.parse().unwrap(),id,true),"{text}");}
        for query in [
            "v=AbCdEf123_-&is_popout=1",
            "v=AbCdEf123_-&theme=dark",
            "v=AbCdEf123_-&is_popout=1&theme=light",
            "v=AbCdEf123_-&is_popout=1&theme=auto",
            "v=AbCdEf123_-&is_popout=1&theme=DARK",
            "v=AbCdEf123_-&is_popout=1&theme=dark&theme=dark",
            "v=AbCdEf123_-&is_popout=1&theme=dark&theme=light",
            "v=AbCdEf123_-&v=AbCdEf123_-&is_popout=1&theme=dark",
            "v=AbCdEf123_-&v=Other123456&is_popout=1&theme=dark",
            "v=AbCdEf123_-&is_popout=1&is_popout=1&theme=dark",
            "v=AbCdEf123_-&is_popout=0&theme=dark",
            "v=Other123456&is_popout=1&theme=dark",
            "v=AbCdEf123_-&is_popout=1&theme=dark&extra=1",
            "v=AbCdEf123_-&is_popout=1&theme=dark#changed",
        ]{
            let url:Url=format!("https://www.youtube.com/live_chat?{query}").parse().unwrap();
            assert!(!matches_document(&url,id,true),"{query}");
        }
        for prefix in ["http://www.youtube.com/live_chat","https://www.youtube.com.evil.test/live_chat",
            "https://www.youtube.com/watch","https://www.youtube.com/live_chat/","https://user@www.youtube.com/live_chat",
            "https://user:secret@www.youtube.com/live_chat","https://www.youtube.com:444/live_chat"]{
            let url:Url=format!("{prefix}?v={id}&is_popout=1&theme=dark").parse().unwrap();
            assert!(!matches_document(&url,id,true),"{prefix}");
        }
        assert!(chat_url("invalid").is_err());
    }
    #[test]fn chat_theme_option_does_not_change_player_or_account_admission(){
        let id="AbCdEf123_-";
        assert_eq!(player_url(id,id).unwrap().as_str(),"https://www.youtube.com/watch?v=AbCdEf123_-&autoplay=1");
        for query in ["v=AbCdEf123_-","v=AbCdEf123_-&autoplay=1","v=AbCdEf123_-&theme=light&other=1"]{
            let url:Url=format!("https://www.youtube.com/watch?{query}").parse().unwrap();
            assert!(matches_document(&url,id,false));assert!(!matches_document(&url,id,true));
        }
        assert!(!account_url(&chat_url(id).unwrap()));
        assert!(!popup_url(&chat_url(id).unwrap()));
        assert_eq!(account_entry(false).as_str(),"https://www.youtube.com/signin");
        assert_eq!(account_entry(true).as_str(),"https://www.youtube.com/logout");
        assert!(account_url(&"https://accounts.google.com/v3/signin/identifier".parse().unwrap()));
    }
    #[test]fn account_boundary_never_admits_chzzk_or_soop(){for url in ["https://nid.naver.com/","https://login.sooplive.com/","https://accounts.google.com.evil.test/"]{assert!(!account_url(&url.parse().unwrap()));}assert!(account_url(&"https://accounts.google.com/v3/signin/identifier".parse().unwrap()));}
    #[test]fn account_entry_uses_provider_handoff_and_diagnostics_are_bounded_fixed_categories(){
        let entry=account_entry(false);assert_eq!(entry.as_str(),"https://www.youtube.com/signin");assert!(account_url(&entry));assert_eq!(account_stage(&entry),"youtube-signin");
        assert_eq!(account_entry(true).path(),"/logout");
        assert_eq!(account_stage(&"https://accounts.google.com/v3/signin/challenge?secret=never-log".parse().unwrap()),"google-account");
        assert_eq!(account_stage(&"https://evil.invalid/private-account?token=never-log".parse().unwrap()),"other");
        assert_eq!(account_stage(&"https://www.google.com/accounts/SetSID?token=never-log".parse().unwrap()),"google-transfer");
        assert!(!account_url(&"https://www.google.com/accounts/SetSID?token=never-log".parse().unwrap()));
        let mut flow=AccountFlow::default();flow.begin("one".into(),false);
        for _ in 0..64 {assert!(flow.diagnostic("one"));}assert!(!flow.diagnostic("one"));assert!(!flow.diagnostic("other"));
        flow.begin("two".into(),false);assert!(flow.diagnostic("two"));
    }
    #[test]fn account_cleanup_needs_fresh_verified_home_not_cached_or_interactive_state(){let home:Url="https://www.youtube.com/".parse().unwrap();let ready=serde_json::json!({"authenticated":true,"interactive":false});assert!(account_completed(&home,&ready,false));for value in [serde_json::Value::Null,serde_json::json!({"authenticated":true}),serde_json::json!({"authenticated":false,"interactive":false}),serde_json::json!({"authenticated":true,"interactive":true})]{assert!(!account_completed(&home,&value,false));}for url in ["https://accounts.google.com/","https://consent.youtube.com/","https://www.youtube.com/account","https://www.youtube.com.evil.test/","http://www.youtube.com/"]{assert!(!account_completed(&url.parse().unwrap(),&ready,false));}}
    #[test]fn account_popup_diagnostics_reject_stale_nonce_without_affecting_replacement(){
        let url:Url="https://accounts.google.co.kr/accounts/SetSID?token=private#secret".parse().unwrap();
        let mut flow=AccountFlow::default();flow.begin("old".into(),false);
        let epoch=flow.start_probe("old").unwrap();
        assert_eq!(flow.popup_diagnostic("old",&url),Some("google-regional-transfer"));
        assert!(flow.current("old",epoch));assert!(flow.start_probe("old").is_none());
        assert!(!account_url(&url));assert!(!popup_url(&url));
        flow.invalidate("old");assert_eq!(flow.popup_diagnostic("old",&url),None);
        flow.begin("new".into(),false);let replacement=flow.start_probe("new").unwrap();
        for _ in 0..128 {assert_eq!(flow.popup_diagnostic("old",&url),None);}
        flow.invalidate("old");assert!(flow.current("new",replacement));
        for _ in 0..64 {assert_eq!(flow.popup_diagnostic("new",&url),Some("google-regional-transfer"));}
        assert_eq!(flow.popup_diagnostic("new",&url),None);
    }
    #[test]fn account_popup_diagnostics_share_lifetime_budget_without_intent_or_navigation_reset(){
        let url:Url="https://accounts.google.com/v3/signin/challenge?token=private".parse().unwrap();
        let mut flow=AccountFlow::default();flow.begin("one".into(),false);
        for index in 0..64 {
            if index%4==3 {assert_eq!(flow.popup_diagnostic("one",&url),Some("google-account"));}
            else {assert!(flow.diagnostic("one"));}
            assert!(flow.navigate("one").is_some());
            assert!(flow.intent("one",index%2==0).is_some());
        }
        for sign_out in [false,true,false] {
            assert!(flow.intent("one",sign_out).is_some());assert!(flow.navigate("one").is_some());
            assert!(!flow.diagnostic("one"));assert_eq!(flow.popup_diagnostic("one",&url),None);
        }
        flow.begin("two".into(),false);
        for _ in 0..64 {assert_eq!(flow.popup_diagnostic("two",&url),Some("google-account"));}
        assert!(!flow.diagnostic("two"));assert_eq!(flow.popup_diagnostic("two",&url),None);
    }
    #[test]fn regional_account_diagnostics_never_grant_navigation_popup_or_completion(){
        let ready=serde_json::json!({"authenticated":true,"interactive":false});
        for host in ["accounts.google.co.kr","accounts.google.ro","accounts.google.fr","accounts.google.co.uk"]{
            for (path,stage) in [("/accounts/SetSID","google-regional-transfer"),("/accounts/setsid","google-regional-account-other"),
                ("/accounts/SetSID/","google-regional-account-other"),("/accounts/%53etSID","google-regional-account-other"),("/private-account-path","google-regional-account-other")] {
                let url:Url=format!("https://{host}{path}?token=never-log&continue=private#secret").parse().unwrap();
                assert_eq!(account_stage(&url),stage);
                assert!(!account_url(&url));assert!(!popup_url(&url));assert!(!account_home(&url));
                assert!(!account_completed(&url,&ready,false));
            }
        }
        for text in ["https://accounts.google.co.kr.evil.test/accounts/SetSID","https://evil.accounts.google.co.kr/accounts/SetSID",
            "https://accounts.google.invalid/accounts/SetSID","https://accounts.google.com.evil.test/accounts/SetSID"]{
            let url:Url=text.parse().unwrap();assert_eq!(account_stage(&url),"other");assert!(!account_url(&url));
        }
        for text in ["http://accounts.google.co.kr/accounts/SetSID","https://user@accounts.google.co.kr/accounts/SetSID",
            "https://user:private@accounts.google.co.kr/accounts/SetSID","https://accounts.google.co.kr:444/accounts/SetSID"]{
            let url:Url=text.parse().unwrap();assert_eq!(account_stage(&url),"non-https");assert!(!account_url(&url));
        }
    }
    #[test]fn diagnostic_expansion_preserves_existing_account_admission_table(){
        for text in ["https://accounts.google.com/v3/signin/challenge","https://accounts.youtube.com/accounts/SetSID",
            "https://consent.google.com/","https://consent.youtube.com/","https://www.youtube.com/",
            "https://www.youtube.com/signin","https://www.youtube.com/logout","https://www.youtube.com/account","https://www.youtube.com/oops"]{
            assert!(account_url(&text.parse().unwrap()),"{text}");
        }
        for text in ["https://www.google.com/accounts/SetSID","https://www.google.co.kr/accounts/SetSID",
            "https://www.youtube.com/check_connection","https://www.youtube.com/watch?v=AbCdEf123_-",
            "https://accounts.google.co.kr/accounts/SetSID","https://accounts.google.ro/accounts/SetSID"]{
            assert!(!account_url(&text.parse().unwrap()),"{text}");
        }
        assert_eq!(account_stage(&"https://accounts.google.com/accounts/SetSID?private=value".parse().unwrap()),"google-account");
        assert_eq!(account_stage(&"https://accounts.youtube.com/accounts/SetSID?private=value".parse().unwrap()),"youtube-account");
        assert_eq!(account_entry(false).as_str(),"https://www.youtube.com/signin");
        assert_eq!(account_entry(true).as_str(),"https://www.youtube.com/logout");
    }
    #[test]fn regional_account_window_exact_endpoints_preserve_all_other_permissions(){
        let suffixes=account_regions::known_suffixes_for_test();assert_eq!(suffixes.len(),269);
        let ready=serde_json::json!({"authenticated":true,"interactive":false});
        for suffix in suffixes {
            let url:Url=format!("https://accounts.google.{suffix}/accounts/SetSID?opaque=sentinel#opaque").parse().unwrap();
            let mut flow=AccountFlow::default();flow.begin("current".into(),false);
            assert!(flow.navigate_account_window("current",&url),"{suffix}");
            assert_eq!(account_url(&url),*suffix=="com","{suffix}");
            assert_eq!(popup_url(&url),*suffix=="com","{suffix}");
            assert!(!account_home(&url));assert!(!account_completed(&url,&ready,false));
            assert!(!matches_document(&url,"AbCdEf123_-",false));
            assert!(!matches_document(&url,"AbCdEf123_-",true));
            assert_eq!(account_stage(&url),if *suffix=="com" {"google-account"} else {"google-regional-transfer"});
        }
    }
    #[test]fn regional_account_window_rejects_every_suffix_path_and_authority_near_miss(){
        for suffix in account_regions::known_suffixes_for_test().iter().filter(|suffix|**suffix!="com") {
            for path in ["/","/v3/signin/challenge","/accounts/setsid","/accounts/SetSID/","/accounts/%53etSID",
                "/accounts/SetSID%2f","/accounts//SetSID","/Accounts/SetSID"] {
                let url:Url=format!("https://accounts.google.{suffix}{path}?continue=sentinel#opaque").parse().unwrap();
                assert!(!account_window_url(&url),"{suffix} {path}");
            }
            for authority in [format!("www.google.{suffix}"),format!("evil.accounts.google.{suffix}"),
                format!("accounts.google.{suffix}.evil.test"),format!("accounts.google.{suffix}."),
                format!("user@accounts.google.{suffix}"),format!("user:sentinel@accounts.google.{suffix}"),
                format!("accounts.google.{suffix}:444")] {
                let url:Url=format!("https://{authority}/accounts/SetSID").parse().unwrap();
                assert!(!account_window_url(&url),"{authority}");
            }
            let http:Url=format!("http://accounts.google.{suffix}/accounts/SetSID").parse().unwrap();
            assert!(!account_window_url(&http));
        }
        for text in ["https://accounts.google.invalid/accounts/SetSID","https://accounts.google.evil.test/accounts/SetSID",
            "https://www.google.com/accounts/SetSID","https://accounts.google.com.evil.test/accounts/SetSID",
            "https://nid.naver.com/accounts/SetSID","https://login.sooplive.com/accounts/SetSID"] {
            assert!(!account_window_url(&text.parse().unwrap()),"{text}");
        }
    }
    #[test]fn regional_account_window_rechecks_each_redirect_without_query_authority(){
        let regional:Url="https://accounts.google.co.kr/accounts/SetSID?continue=https%3A%2F%2Fevil.test%2F#opaque".parse().unwrap();
        let external:Url="https://evil.test/?continue=https%3A%2F%2Faccounts.google.co.kr%2Faccounts%2FSetSID".parse().unwrap();
        let mut flow=AccountFlow::default();flow.begin("current".into(),false);
        assert!(flow.navigate_account_window("current",&regional));
        let epoch=flow.start_probe("current").unwrap();
        assert!(!flow.navigate_account_window("current",&external));assert!(flow.current("current",epoch));
        assert!(!popup_url(&regional));assert!(!account_url(&regional));
        assert!(flow.navigate_account_window("current",&account_entry(false)));
        assert!(flow.navigate_account_window("current",&"https://www.youtube.com/".parse().unwrap()));
    }
    #[test]fn regional_account_window_nonce_and_diagnostic_budget_remain_independent(){
        let url:Url="https://accounts.google.ro/accounts/SetSID".parse().unwrap();
        let mut flow=AccountFlow::default();
        assert!(!flow.navigate_account_window("old",&url));
        flow.begin("old".into(),false);let epoch=flow.start_probe("old").unwrap();
        assert!(!flow.navigate_account_window("foreign",&url));assert!(flow.current("old",epoch));
        assert!(flow.navigate_account_window("old",&url));assert!(!flow.current("old",epoch));
        flow.invalidate("old");assert!(!flow.navigate_account_window("old",&url));
        flow.begin("new".into(),false);let replacement=flow.start_probe("new").unwrap();
        assert!(!flow.navigate_account_window("old",&url));flow.invalidate("old");
        assert!(flow.current("new",replacement));
        for _ in 0..64 {assert!(flow.diagnostic("new"));}
        assert!(!flow.diagnostic("new"));assert!(flow.navigate_account_window("new",&url));
        assert!(!flow.diagnostic("new"));assert!(!flow.current("new",replacement));
        assert!(flow.intent("new",true).is_some());assert!(flow.navigate_account_window("new",&url));
        assert!(!flow.diagnostic("new"));
    }
    #[test]fn logout_completion_requires_fresh_false_and_waits_for_interaction_to_finish(){
        let home:Url="https://www.youtube.com/".parse().unwrap();
        for sign_out in [false,true]{
            let ready=serde_json::json!({"authenticated":!sign_out,"interactive":false});
            assert!(account_completed(&home,&ready,sign_out));
            assert!(!account_completed(&home,&ready,!sign_out));
            let interactive=serde_json::json!({"authenticated":!sign_out,"interactive":true});
            assert!(!account_completed(&home,&interactive,sign_out));
            assert!(!account_completed(&home,&serde_json::json!({"authenticated":null,"interactive":false}),sign_out));
            assert!(!account_completed(&"https://accounts.google.com/".parse().unwrap(),&ready,sign_out));
        }
    }
    #[test]fn account_reused_window_obeys_latest_intent_and_deduplicates_confirmation(){
        let mut flow=AccountFlow::default();flow.begin("one".into(),false);
        let first=flow.start_probe("one").unwrap();assert!(flow.start_probe("one").is_none());
        assert_eq!(flow.observe("one",first,true),Some(true));assert_eq!(flow.observe("one",first,true),Some(false));
        assert_eq!(flow.intent("one",true),Some(true));assert!(!flow.current("one",first));
        let logout=flow.start_probe("one").unwrap();assert_eq!(flow.is_sign_out("one",logout),Some(true));
        assert_eq!(flow.observe("one",first,true),None);assert_eq!(flow.observe("one",logout,false),Some(true));
        assert_eq!(flow.intent("one",false),Some(true));let login=flow.start_probe("one").unwrap();
        assert_eq!(flow.is_sign_out("one",login),Some(false));assert!(!flow.current("one",logout));
        assert_eq!(flow.intent("one",false),Some(false));assert!(!flow.current("one",login));
    }
    #[test]fn account_worker_is_bounded_and_stale_navigation_or_destruction_cannot_affect_replacement(){
        let mut flow=AccountFlow::default();let start=Instant::now();flow.begin("old".into(),false);
        let old=flow.start_probe("old").unwrap();
        assert!(flow.probe_active("old",old,start,119,start+Duration::from_secs(119)));
        assert!(!flow.probe_active("old",old,start,120,start));
        assert!(!flow.probe_active("old",old,start,0,start+Duration::from_secs(120)));
        flow.navigate("old");assert!(!flow.probe_active("old",old,start,0,start));
        let next=flow.start_probe("old").unwrap();flow.invalidate("old");assert!(!flow.current("old",next));
        flow.begin("new".into(),false);let replacement=flow.start_probe("new").unwrap();flow.invalidate("old");
        assert!(flow.current("new",replacement));assert!(!flow.current("old",old));
        for url in ["https://www.youtube.com/?auth=1","https://www.youtube.com/#login","https://www.youtube.com/signin"]{
            assert!(!account_home(&url.parse().unwrap()));
        }
    }
    #[test]fn current_arm_and_bridge_identity_required(){let now=Instant::now();let arm=Arm{id:uuid::Uuid::new_v4().to_string(),epoch:2,created:now};assert!(arm.accepts(&arm.id,2,now));assert!(!arm.accepts(&arm.id,3,now));assert!(!arm.accepts(&arm.id,2,now+Duration::from_secs(26)));let u=player_url("AbCdEf123_-","").unwrap();assert!(validate_envelope(u.as_str(),"y-1","y-1","AbCdEf123_-",2,2,"{}"));assert!(!validate_envelope(u.as_str(),"s-1","y-1","AbCdEf123_-",2,2,"{}"));}
    #[test]fn lost_begin_and_stop_acknowledgements_have_bounded_fallback(){let now=Instant::now();assert!(!ack_expired(now,now+Duration::from_secs(12),false));assert!(ack_expired(now,now+Duration::from_secs(25),false));assert!(!ack_expired(now,now+Duration::from_secs(14),true));assert!(ack_expired(now,now+Duration::from_secs(15),true));}
    #[test]fn stale_visible_cannot_replace_new_hidden_intent(){let mut last=0;let mut hidden=false;for (sequence,next_hidden) in [(1,false),(3,true),(2,false),(3,false)]{if sequence_newer(last,sequence){last=sequence;hidden=next_hidden;}}assert_eq!(last,3);assert!(hidden);assert!(!sequence_newer(0,0));assert!(!sequence_newer(0,u64::MAX));assert!(sequence_newer(0,1));}
    #[test]fn old_receiver_cannot_overwrite_confirmed_account_but_reloaded_document_can(){let mut state=AccountObservation::default();state.observe(0,false);assert_eq!(state.value,Some(false));state.confirm(true);state.observe(0,false);assert_eq!(state.value,Some(true));state.observe(state.revision,false);assert_eq!(state.value,Some(false));let old=state.revision;state.confirm(false);state.observe(old,true);assert_eq!(state.value,Some(false));state.observe(state.revision,true);assert_eq!(state.value,Some(true));}
    #[test]fn normal_broadcast_end_needs_exact_public_video_and_explicit_end_time(){let mut value=serde_json::json!({"videoDetails":{"videoId":"abcdefghijk","isLive":false},"microformat":{"playerMicroformatRenderer":{"liveBroadcastDetails":{"isLiveNow":false,"endTimestamp":"2026-10-14T12:00:00+00:00"}}}});assert!(confirmed_ended(&value,"abcdefghijk"));assert!(!confirmed_ended(&value,"bcdefghijkl"));value["videoDetails"]["isLive"]=true.into();assert!(!confirmed_ended(&value,"abcdefghijk"));value["videoDetails"]["isLive"]=false.into();value["microformat"]["playerMicroformatRenderer"]["liveBroadcastDetails"]["endTimestamp"]=serde_json::Value::Null;assert!(!confirmed_ended(&value,"abcdefghijk"));}
}
