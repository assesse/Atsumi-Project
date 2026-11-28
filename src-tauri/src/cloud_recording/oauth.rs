use super::{
    model::{channel_id, now, CloudError},
    upload::{self, Request, Transport},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub(crate) const UPLOAD_SCOPE: &str = "https://www.googleapis.com/auth/youtube.upload";
pub(crate) const READ_SCOPE: &str = "https://www.googleapis.com/auth/youtube.readonly";
pub(crate) const LIVE_SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
const FULL_SCOPE: &str = "https://www.googleapis.com/auth/youtube";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientConfig {
    pub client_id: String,
    pub client_secret: String,
}
impl ClientConfig {
    pub(crate) fn valid(&self) -> bool {
        self.client_id.len() <= 512
            && self.client_id.ends_with(".apps.googleusercontent.com")
            && self
                .client_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
            && !self.client_secret.is_empty()
            && self.client_secret.len() <= 4096
            && self.client_secret.bytes().all(|c| c.is_ascii_graphic())
    }
    pub(crate) fn read(path: &Path) -> Result<Self, CloudError> {
        let error = || {
            CloudError::new(
                "CLOUD_CLIENT_FILE",
                "Google Cloud의 데스크톱 앱용 OAuth JSON 파일을 선택해 주세요.",
                false,
            )
        };
        let metadata = std::fs::symlink_metadata(path).map_err(|_| error())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 65536 {
            return Err(error());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(error());
            }
        }
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| error())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| error())?;
        if bytes.len() > 65536 {
            return Err(error());
        }
        Self::parse(&bytes)
    }
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, CloudError> {
        let error = || {
            CloudError::new(
                "CLOUD_CLIENT_FILE",
                "Google Cloud의 데스크톱 앱용 OAuth JSON 파일을 선택해 주세요.",
                false,
            )
        };
        let value: Value = serde_json::from_slice(bytes).map_err(|_| error())?;
        if value.get("web").is_some() {
            return Err(error());
        }
        let installed = value
            .get("installed")
            .and_then(Value::as_object)
            .ok_or_else(error)?;
        for (key, expected) in [
            ("auth_uri", "https://accounts.google.com/o/oauth2/auth"),
            ("token_uri", "https://oauth2.googleapis.com/token"),
        ] {
            if installed
                .get(key)
                .is_some_and(|v| v.as_str() != Some(expected))
            {
                return Err(error());
            }
        }
        let client = Self {
            client_id: installed
                .get("client_id")
                .and_then(Value::as_str)
                .ok_or_else(error)?
                .into(),
            client_secret: installed
                .get("client_secret")
                .and_then(Value::as_str)
                .ok_or_else(error)?
                .into(),
        };
        if !client.valid() {
            return Err(error());
        }
        Ok(client)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Grant {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub channel_id: String,
    pub channel_name: String,
}
impl Grant {
    pub(crate) fn valid(&self) -> bool {
        secret(&self.access_token)
            && secret(&self.refresh_token)
            && channel_id(&self.channel_id)
            && self.channel_name.chars().count() <= 200
    }
}
/// A separate consent receipt. An upload grant cannot be promoted by assuming
/// that the scopes requested by the client were actually granted by Google.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LiveGrant {
    pub grant: Grant,
    pub scopes: Vec<String>,
}
impl LiveGrant {
    pub(crate) fn valid(&self) -> bool {
        self.grant.valid() && valid_live_scopes(&self.scopes)
    }
}
fn valid_live_scopes(scopes: &[String]) -> bool {
    !scopes.is_empty() && scopes.len() <= 32
        && scopes.iter().map(String::len).sum::<usize>() <= 4096
        && scopes.iter().all(|scope| !scope.is_empty() && scope.bytes().all(|c| c.is_ascii_graphic()))
        && scopes.iter().any(|scope| scope == LIVE_SCOPE || scope == FULL_SCOPE)
}
fn live_scope_error() -> CloudError {
    CloudError::new("CLOUD_LIVE_SCOPE", "YouTube 실시간 전송 권한을 허용해 주세요. 기존 업로드 연결은 유지됩니다.", false)
}
fn live_scopes(value: &Value, previous: Option<&LiveGrant>) -> Result<Vec<String>, CloudError> {
    let scopes = match value.get("scope") {
        Some(Value::String(scope)) if scope.len() <= 4096 => {
            let mut scopes: Vec<String> = scope.split_ascii_whitespace().map(str::to_owned).collect();
            scopes.sort();
            scopes.dedup();
            scopes
        }
        // Refresh may omit unchanged scopes. Only the same previously verified
        // Live grant supplies them; initial consent and legacy grants cannot.
        None => previous.filter(|grant| grant.valid()).map(|grant| grant.scopes.clone()).ok_or_else(live_scope_error)?,
        _ => return Err(live_scope_error()),
    };
    if !valid_live_scopes(&scopes) { return Err(live_scope_error()); }
    Ok(scopes)
}
fn secret(value: &str) -> bool {
    !value.is_empty() && value.len() <= 16384 && value.bytes().all(|b| b.is_ascii_graphic())
}
pub(crate) fn pkce(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn form(pairs: &[(&str, &str)]) -> Result<Vec<u8>, CloudError> {
    let mut url = Url::parse("https://localhost/").map_err(|_| CloudError::protocol())?;
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    Ok(url.query().unwrap_or_default().as_bytes().to_vec())
}
fn token_request(transport: &dyn Transport, pairs: &[(&str, &str)]) -> Result<Value, CloudError> {
    let value = token_response(transport, pairs)?;
    if let Some(scopes) = value["scope"].as_str() {
        let scopes: Vec<_> = scopes.split_ascii_whitespace().collect();
        if !scopes.contains(&UPLOAD_SCOPE) || !scopes.contains(&READ_SCOPE) {
            return Err(CloudError::new(
                "CLOUD_SCOPE_REQUIRED",
                "업로드와 업로드 결과 확인 권한을 모두 허용해 주세요.",
                false,
            ));
        }
    }
    Ok(value)
}
fn token_response(transport: &dyn Transport, pairs: &[(&str, &str)]) -> Result<Value, CloudError> {
    let response = transport.send(Request {
        method: Method::POST,
        url: "https://oauth2.googleapis.com/token".into(),
        bearer: None,
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: form(pairs)?,
    })?;
    if response.status != 200 {
        let error = response
            .json()
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned));
        let message = match error.as_deref() {
            Some("invalid_grant") if pairs.contains(&("grant_type", "refresh_token")) =>
                "Google이 기존 계정 연결을 더 이상 승인하지 않습니다. 같은 YouTube 계정으로 다시 연결해 주세요.",
            Some("invalid_grant") => "Google 인증 코드가 유효하지 않습니다. 계정 연결을 다시 시작해 주세요.",
            Some("invalid_client" | "unauthorized_client") =>
                "Google이 이 앱의 인증 설정을 승인하지 않았습니다. Google Cloud에서 발급한 데스크톱 앱 설정 파일을 확인해 주세요.",
            Some("access_denied") => "Google이 이 계정의 접근을 거부했습니다. Google Cloud의 허용 계정과 요청 권한을 확인해 주세요.",
            _ => return Err(upload::http_error(&response)),
        };
        return Err(CloudError::new("CLOUD_AUTH_REQUIRED", message, true));
    }
    let value = response.json()?;
    if value["token_type"]
        .as_str()
        .is_none_or(|s| !s.eq_ignore_ascii_case("bearer"))
    {
        return Err(CloudError::protocol());
    }
    Ok(value)
}
fn expires(value: &Value) -> Result<u64, CloudError> {
    let seconds = value["expires_in"]
        .as_u64()
        .filter(|n| (1..=86400).contains(n))
        .ok_or_else(CloudError::protocol)?;
    now()
        .checked_add(seconds * 1000)
        .ok_or_else(CloudError::protocol)
}
pub(crate) fn refresh(
    transport: &dyn Transport,
    client: &ClientConfig,
    previous: &Grant,
) -> Result<Grant, CloudError> {
    let value = token_request(
        transport,
        &[
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("refresh_token", &previous.refresh_token),
            ("grant_type", "refresh_token"),
        ],
    )?;
    refreshed_grant(previous, &value)
}
fn refreshed_grant(previous: &Grant, value: &Value) -> Result<Grant, CloudError> {
    let mut grant = previous.clone();
    grant.access_token = value["access_token"]
        .as_str()
        .filter(|v| secret(v))
        .ok_or_else(CloudError::protocol)?
        .into();
    if let Some(token) = value["refresh_token"].as_str() {
        if !secret(token) {
            return Err(CloudError::protocol());
        }
        grant.refresh_token = token.into();
    }
    grant.expires_at = expires(&value)?;
    Ok(grant)
}
pub(crate) fn refresh_live(
    transport: &dyn Transport,
    client: &ClientConfig,
    previous: &LiveGrant,
) -> Result<LiveGrant, CloudError> {
    if !previous.valid() { return Err(live_scope_error()); }
    let value = token_response(transport, &[
        ("client_id", &client.client_id),
        ("client_secret", &client.client_secret),
        ("refresh_token", &previous.grant.refresh_token),
        ("grant_type", "refresh_token"),
    ])?;
    let scopes = live_scopes(&value, Some(previous))?;
    Ok(LiveGrant { grant: refreshed_grant(&previous.grant, &value)?, scopes })
}
pub(crate) fn authorize(
    transport: &dyn Transport,
    client: &ClientConfig,
    opener: &(dyn Fn(&str) -> Result<(), String> + Send + Sync),
    cancel: &AtomicBool,
) -> Result<Grant, CloudError> {
    authorize_for(transport, client, opener, cancel, false).map(|(grant, _)| grant)
}
pub(crate) fn authorize_live(
    transport: &dyn Transport,
    client: &ClientConfig,
    opener: &(dyn Fn(&str) -> Result<(), String> + Send + Sync),
    cancel: &AtomicBool,
) -> Result<LiveGrant, CloudError> {
    let (grant, scopes) = authorize_for(transport, client, opener, cancel, true)?;
    Ok(LiveGrant { grant, scopes })
}
fn authorize_for(
    transport: &dyn Transport,
    client: &ClientConfig,
    opener: &(dyn Fn(&str) -> Result<(), String> + Send + Sync),
    cancel: &AtomicBool,
    live: bool,
) -> Result<(Grant, Vec<String>), CloudError> {
    if cancel.load(Ordering::Acquire) { return Err(CloudError::cancelled()); }
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| {
        CloudError::new(
            "CLOUD_CALLBACK",
            "인증 응답을 받을 로컬 연결을 열지 못했습니다.",
            true,
        )
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|_| CloudError::protocol())?;
    let port = listener
        .local_addr()
        .map_err(|_| CloudError::protocol())?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/oauth2/callback");
    let verifier = random();
    let state = random();
    let challenge = pkce(&verifier);
    let scopes = if live { LIVE_SCOPE.to_string() } else { format!("{UPLOAD_SCOPE} {READ_SCOPE}") };
    let mut url = Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
        .map_err(|_| CloudError::protocol())?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("response_type", "code"),
        ("scope", scopes.as_str()),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent select_account"),
    ]);
    opener(url.as_str()).map_err(|_| {
        CloudError::new(
            "CLOUD_BROWSER",
            "공식 Google 인증 페이지를 열지 못했습니다.",
            true,
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut attempts = 0;
    let code = loop {
        if cancel.load(Ordering::Acquire) {
            return Err(CloudError::cancelled());
        }
        if Instant::now() >= deadline {
            return Err(CloudError::new(
                "CLOUD_AUTH_TIMEOUT",
                "Google 연결 시간이 초과됐습니다. 연결을 다시 시작해 주세요.",
                true,
            ));
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                attempts += 1;
                if attempts > 64 {
                    return Err(CloudError::new(
                        "CLOUD_CALLBACK",
                        "인증 응답을 확인하지 못했습니다. 연결을 다시 시작해 주세요.",
                        true,
                    ));
                }
                if !peer.ip().is_loopback() {
                    continue;
                }
                let result = read_callback(&mut stream, port, &state);
                let accepted = result.as_ref().is_ok_and(|v| v.is_some());
                let text = if accepted {
                    "인증 응답을 받았습니다. 아츠미에서 연결 결과를 확인해 주세요."
                } else {
                    "이 인증 응답은 사용할 수 없습니다. 아츠미에서 연결을 다시 시작해 주세요."
                };
                let body = format!("<!doctype html><html lang=\"ko\"><meta charset=\"utf-8\"><title>Atsumi</title><p>{text}</p></html>");
                let status = if accepted {
                    "200 OK"
                } else {
                    "400 Bad Request"
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
                match result {
                    Ok(Some(code)) => break code,
                    Err(error) if error.code == "CLOUD_AUTH_DENIED" => return Err(error),
                    _ => {}
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err(CloudError::protocol()),
        }
    };
    if cancel.load(Ordering::Acquire) {
        return Err(CloudError::cancelled());
    }
    let pairs: [(&str, &str); 6] = [
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect),
            ("grant_type", "authorization_code"),
        ];
    let value = if live { token_response(transport, &pairs)? } else { token_request(transport, &pairs)? };
    let granted = if live { live_scopes(&value, None)? } else { Vec::new() };
    if cancel.load(Ordering::Acquire) { return Err(CloudError::cancelled()); }
    let access = value["access_token"]
        .as_str()
        .filter(|v| secret(v))
        .ok_or_else(CloudError::protocol)?;
    let refresh = value["refresh_token"]
        .as_str()
        .filter(|v| secret(v))
        .ok_or_else(|| {
            CloudError::new(
                "CLOUD_OFFLINE_PERMISSION",
                "백그라운드 연결 권한을 받지 못했습니다. 다시 연결해 주세요.",
                true,
            )
        })?;
    let channel = upload::api_get(
        transport,
        access,
        "channels",
        &[
            ("part", "snippet"),
            ("mine", "true"),
            ("fields", "items(id,snippet/title)"),
        ],
    )?;
    let items = channel["items"]
        .as_array()
        .ok_or_else(CloudError::protocol)?;
    if items.len() != 1 {
        return Err(CloudError::new(
            "CLOUD_CHANNEL_SELECTION",
            "보관할 YouTube 채널 하나를 선택해 다시 연결해 주세요.",
            false,
        ));
    }
    let id = items[0]["id"]
        .as_str()
        .filter(|id| channel_id(id))
        .ok_or_else(CloudError::protocol)?;
    let name = items[0]["snippet"]["title"]
        .as_str()
        .ok_or_else(CloudError::protocol)?
        .chars()
        .take(200)
        .collect();
    if cancel.load(Ordering::Acquire) {
        return Err(CloudError::cancelled());
    }
    Ok((Grant {
        access_token: access.into(),
        refresh_token: refresh.into(),
        expires_at: expires(&value)?,
        channel_id: id.into(),
        channel_name: name,
    }, granted))
}
fn read_callback(
    stream: &mut TcpStream,
    port: u16,
    state: &str,
) -> Result<Option<String>, CloudError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CloudError::protocol())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CloudError::protocol())?;
    let start = Instant::now();
    let mut bytes = Vec::new();
    let mut block = [0; 1024];
    while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
        if bytes.len() >= 16384 || start.elapsed() >= Duration::from_secs(3) {
            return Err(CloudError::protocol());
        }
        let count = stream
            .read(&mut block)
            .map_err(|_| CloudError::protocol())?;
        if count == 0 {
            return Err(CloudError::protocol());
        }
        bytes.extend_from_slice(&block[..count]);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| CloudError::protocol())?;
    parse_callback(text, port, state)
}
pub(crate) fn parse_callback(
    request: &str,
    port: u16,
    state: &str,
) -> Result<Option<String>, CloudError> {
    if request.len() > 16384 {
        return Err(CloudError::protocol());
    }
    let (headers, body) = request
        .split_once("\r\n\r\n")
        .ok_or_else(CloudError::protocol)?;
    if !body.is_empty() {
        return Err(CloudError::protocol());
    }
    let mut lines = headers.split("\r\n");
    let first: Vec<_> = lines.next().unwrap_or_default().split(' ').collect();
    if first.len() != 3
        || first[0] != "GET"
        || first[2] != "HTTP/1.1"
        || !first[1].starts_with("/oauth2/callback?")
        || first[1].len() > 8192
    {
        return Err(CloudError::protocol());
    }
    let mut host = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(CloudError::protocol)?;
        if name.eq_ignore_ascii_case("host") {
            if host.is_some() {
                return Err(CloudError::protocol());
            }
            host = Some(value.trim());
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-length") && value.trim() != "0"
        {
            return Err(CloudError::protocol());
        }
    }
    if host != Some(format!("127.0.0.1:{port}").as_str()) {
        return Err(CloudError::protocol());
    }
    let url = Url::parse(&format!("http://127.0.0.1:{port}{}", first[1]))
        .map_err(|_| CloudError::protocol())?;
    if url.path() != "/oauth2/callback" || url.fragment().is_some() {
        return Err(CloudError::protocol());
    }
    let values: Vec<_> = url.query_pairs().collect();
    let states: Vec<_> = values.iter().filter(|(key, _)| key == "state").collect();
    if states.len() != 1 || states[0].1 != state {
        return Err(CloudError::protocol());
    }
    if values.iter().any(|(key, _)| key == "error") {
        return Err(CloudError::new(
            "CLOUD_AUTH_DENIED",
            "Google 연결이 취소되었거나 허용되지 않았습니다.",
            true,
        ));
    }
    let codes: Vec<_> = values.iter().filter(|(key, _)| key == "code").collect();
    if codes.len() != 1 || !secret(&codes[0].1) {
        return Err(CloudError::protocol());
    }
    Ok(Some(codes[0].1.to_string()))
}

pub(crate) type BrowserOpener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

#[cfg(test)]
#[path = "oauth_live_tests.rs"]
mod live_tests;
