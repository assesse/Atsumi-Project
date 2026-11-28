use super::*;
use super::super::upload::Response;
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};

const OWNER: &str = "UCaaaaaaaaaaaaaaaaaaaaaa";
#[derive(Default)]
struct Mock {
    responses: Mutex<VecDeque<Value>>,
    requests: Mutex<Vec<Request>>,
}
impl Mock {
    fn new(values: Vec<Value>) -> Self {
        Self { responses: Mutex::new(values.into()), ..Default::default() }
    }
}
impl Transport for Mock {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        self.requests.lock().unwrap().push(request);
        let value = self.responses.lock().unwrap().pop_front().expect("unexpected fixture request");
        Ok(Response { status: 200, headers: vec![], body: serde_json::to_vec(&value).unwrap() })
    }
}
fn client() -> ClientConfig {
    ClientConfig { client_id: "123-fixture.apps.googleusercontent.com".into(), client_secret: "FIXTURE_CLIENT".into() }
}
fn previous() -> LiveGrant {
    LiveGrant { grant: Grant { access_token: "FIXTURE_ACCESS".into(), refresh_token: "FIXTURE_REFRESH".into(),
        expires_at: now() + 600_000, channel_id: OWNER.into(), channel_name: "Fixture".into() }, scopes: vec![LIVE_SCOPE.into()] }
}
fn token() -> Value {
    json!({"access_token":"FIXTURE_NEW_ACCESS","refresh_token":"FIXTURE_NEW_REFRESH","token_type":"Bearer","expires_in":3600})
}
fn channel() -> Value { json!({"items":[{"id":OWNER,"snippet":{"title":"Fixture"}}]}) }

#[test]
fn upload_grant_is_not_a_live_consent_receipt() {
    let old = previous().grant;
    assert!(serde_json::from_value::<LiveGrant>(serde_json::to_value(&old).unwrap()).is_err());
    assert!(!LiveGrant { grant: old.clone(), scopes: vec![UPLOAD_SCOPE.into(), READ_SCOPE.into()] }.valid());
    assert!(!LiveGrant { grant: old, scopes: vec![format!("{LIVE_SCOPE}\n")] }.valid());
}

#[test]
fn consent_requires_explicit_bounded_live_scope() {
    for value in [json!({}), json!({"scope":null}), json!({"scope":[]}), json!({"scope":UPLOAD_SCOPE}),
        json!({"scope":format!("{UPLOAD_SCOPE} {READ_SCOPE}")}), json!({"scope":"x".repeat(4097)})] {
        assert_eq!(live_scopes(&value, None).unwrap_err().code, "CLOUD_LIVE_SCOPE");
    }
    assert_eq!(live_scopes(&json!({"scope":format!("{LIVE_SCOPE} {LIVE_SCOPE}")}), None).unwrap(), vec![LIVE_SCOPE]);
    assert_eq!(live_scopes(&json!({"scope":FULL_SCOPE}), None).unwrap(), vec![FULL_SCOPE]);
}

#[test]
fn live_refresh_without_scope_carries_only_same_verified_grant() {
    let mock = Mock::new(vec![token()]);
    let old = previous();
    let refreshed = refresh_live(&mock, &client(), &old).unwrap();
    assert_eq!(refreshed.scopes, old.scopes);
    assert_eq!(refreshed.grant.channel_id, OWNER);
    assert_eq!(refreshed.grant.refresh_token, "FIXTURE_NEW_REFRESH");
    assert_eq!(old.grant.refresh_token, "FIXTURE_REFRESH");
    assert!(refreshed.grant.expires_at > now());
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
}

#[test]
fn live_refresh_explicit_downscope_is_rejected_without_mutating_old_grant() {
    let mut value = token(); value["scope"] = json!(READ_SCOPE);
    let mock = Mock::new(vec![value]);
    let old = previous();
    assert_eq!(refresh_live(&mock, &client(), &old).err().unwrap().code, "CLOUD_LIVE_SCOPE");
    assert_eq!(old.grant.access_token, "FIXTURE_ACCESS");
    assert_eq!(old.scopes, vec![LIVE_SCOPE]);
}

#[test]
fn invalid_live_grant_cannot_refresh_or_inherit_missing_scope() {
    let mut old = previous(); old.scopes = vec![UPLOAD_SCOPE.into(), READ_SCOPE.into()];
    let mock = Mock::default();
    assert_eq!(refresh_live(&mock, &client(), &old).err().unwrap().code, "CLOUD_LIVE_SCOPE");
    assert!(live_scopes(&json!({}), Some(&old)).is_err());
    assert!(mock.requests.lock().unwrap().is_empty());
}

#[test]
fn legacy_upload_refresh_scope_contract_is_unchanged() {
    let old = previous().grant;
    assert!(refresh(&Mock::new(vec![token()]), &client(), &old).is_ok());
    let mut live_only = token(); live_only["scope"] = json!(LIVE_SCOPE);
    assert_eq!(refresh(&Mock::new(vec![live_only]), &client(), &old).err().unwrap().code, "CLOUD_SCOPE_REQUIRED");
}

fn simulated_consent(mock: &Mock, live: bool, denied: bool) -> (Result<(Grant, Vec<String>), CloudError>, Url) {
    let observed = Mutex::new(None);
    let callback = Mutex::new(None);
    let opener = |text: &str| -> Result<(), String> {
        let url = Url::parse(text).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().map(|(k,v)| (k.into_owned(),v.into_owned())).collect();
        let mut redirect = Url::parse(&query["redirect_uri"]).unwrap();
        assert_eq!(redirect.host_str(), Some("127.0.0.1"));
        let port = redirect.port().unwrap();
        redirect.query_pairs_mut().append_pair("state", &query["state"])
            .append_pair(if denied { "error" } else { "code" }, if denied { "access_denied" } else { "FIXTURE_CODE" });
        *observed.lock().unwrap() = Some(url);
        *callback.lock().unwrap() = Some(std::thread::spawn(move || {
            let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            write!(socket, "GET {}?{} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n", redirect.path(), redirect.query().unwrap()).unwrap();
            let mut response = String::new(); socket.read_to_string(&mut response).unwrap();
            assert!(response.starts_with(if denied { "HTTP/1.1 400" } else { "HTTP/1.1 200" }));
        }));
        Ok(())
    };
    let result = authorize_for(mock, &client(), &opener, &AtomicBool::new(false), live);
    if let Some(thread) = callback.lock().unwrap().take() { thread.join().unwrap(); }
    (result, observed.into_inner().unwrap().unwrap())
}

#[test]
fn live_loopback_consent_checks_pkce_scope_and_actual_owner() {
    let mut value = token(); value["scope"] = json!(LIVE_SCOPE);
    let mock = Mock::new(vec![value, channel()]);
    let (result, url) = simulated_consent(&mock, true, false);
    let (grant, scopes) = result.unwrap();
    let query: std::collections::HashMap<_,_> = url.query_pairs().collect();
    assert_eq!(query["scope"], LIVE_SCOPE);
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["prompt"], "consent select_account");
    assert!(!query.contains_key("include_granted_scopes"));
    let requests = mock.requests.lock().unwrap();
    let mut encoded_form = Url::parse("http://127.0.0.1/fixture").unwrap();
    encoded_form.set_query(Some(std::str::from_utf8(&requests[0].body).unwrap()));
    let form: std::collections::HashMap<_,_> = encoded_form.query_pairs().collect();
    assert_eq!(pkce(&form["code_verifier"]), query["code_challenge"]);
    assert_eq!(form["redirect_uri"], query["redirect_uri"]);
    assert_eq!(grant.channel_id, OWNER);
    assert_eq!(scopes, vec![LIVE_SCOPE]);
    assert_eq!(requests.len(), 2);
    assert!(requests[1].url.contains("/channels?"));
}

#[test]
fn live_initial_missing_scope_never_queries_owner_or_creates_resources() {
    let mock = Mock::new(vec![token()]);
    let (result, _) = simulated_consent(&mock, true, false);
    assert_eq!(result.err().unwrap().code, "CLOUD_LIVE_SCOPE");
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
}

#[test]
fn denied_live_consent_sends_no_token_or_channel_request() {
    let mock = Mock::default();
    let (result, _) = simulated_consent(&mock, true, true);
    assert_eq!(result.err().unwrap().code, "CLOUD_AUTH_DENIED");
    assert!(mock.requests.lock().unwrap().is_empty());
}

#[test]
fn legacy_loopback_consent_still_requests_only_upload_and_read() {
    let mock = Mock::new(vec![token(), channel()]);
    let (result, url) = simulated_consent(&mock, false, false);
    let (grant, scopes) = result.unwrap();
    let query: std::collections::HashMap<_,_> = url.query_pairs().collect();
    assert_eq!(query["scope"], format!("{UPLOAD_SCOPE} {READ_SCOPE}"));
    assert_eq!(grant.channel_id, OWNER);
    assert!(scopes.is_empty());
}

#[test]
fn cancelled_live_consent_does_not_open_browser_or_transport() {
    let mock = Mock::default();
    let opener = |_url: &str| -> Result<(), String> { panic!("cancelled authorization opened browser") };
    assert_eq!(authorize_live(&mock, &client(), &opener, &AtomicBool::new(true)).err().unwrap().code, CloudError::cancelled().code);
    assert!(mock.requests.lock().unwrap().is_empty());
}
