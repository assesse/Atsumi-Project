use super::model::{channel_id, video_id, CloudError, CloudVisibility};
use reqwest::{blocking::Client, Method, Url};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

pub(crate) const MAX_BYTES: u64 = 256_000_000_000;
pub(crate) const CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
pub(crate) struct Request {
    pub method: Method,
    pub url: String,
    pub bearer: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
pub(crate) struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Response {
    pub(crate) fn json(&self) -> Result<Value, CloudError> {
        serde_json::from_slice(&self.body).map_err(|_| CloudError::protocol())
    }
    pub(crate) fn header(&self, name: &str) -> Result<Option<&str>, CloudError> {
        let mut values = self
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name));
        let value = values.next().map(|(_, value)| value.as_str());
        if values.next().is_some() {
            return Err(CloudError::protocol());
        }
        Ok(value)
    }
}
pub(crate) trait Transport: Send + Sync {
    fn send(&self, request: Request) -> Result<Response, CloudError>;
}
pub(crate) struct GoogleTransport(Client);
impl GoogleTransport {
    pub(crate) fn new() -> Result<Self, CloudError> {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map(Self)
            .map_err(|_| CloudError::protocol())
    }
}
impl Transport for GoogleTransport {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        if !allowed_request(&request.url) {
            return Err(CloudError::protocol());
        }
        let mut builder = self.0.request(request.method, request.url);
        if let Some(token) = request.bearer {
            builder = builder.bearer_auth(token);
        }
        for (key, value) in request.headers {
            builder = builder.header(key, value);
        }
        let response = builder.body(request.body).send().map_err(|_| {
            CloudError::new(
                "CLOUD_NETWORK",
                "YouTube 연결이 끊겼습니다. 확인된 위치에서 다시 시도합니다.",
                true,
            )
        })?;
        let status = response.status().as_u16();
        let mut headers = Vec::new();
        for name in ["location", "range", "retry-after"] {
            for value in response.headers().get_all(name) {
                headers.push((
                    name.into(),
                    value
                        .to_str()
                        .map_err(|_| CloudError::protocol())?
                        .to_owned(),
                ));
            }
        }
        let mut body = Vec::new();
        response
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut body)
            .map_err(|_| CloudError::protocol())?;
        if body.len() as u64 > MAX_RESPONSE {
            return Err(CloudError::protocol());
        }
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}
fn allowed_request(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|p| p != 443)
    {
        return false;
    }
    matches!(
        (url.host_str(), url.path()),
        (Some("oauth2.googleapis.com"), "/token")
            | (
                Some("www.googleapis.com"),
                "/youtube/v3/channels" | "/youtube/v3/videos" | "/upload/youtube/v3/videos"
            )
            | (Some("youtube.googleapis.com"), "/upload/youtube/v3/videos")
    )
}
pub(crate) fn valid_session_url(value: &str) -> bool {
    if value.len() > 8192 || !allowed_request(value) {
        return false;
    }
    Url::parse(value).is_ok_and(|u| {
        let ids: Vec<_> = u.query_pairs().filter(|(k, _)| k == "upload_id").collect();
        matches!(
            u.host_str(),
            Some("www.googleapis.com" | "youtube.googleapis.com")
        ) && u.path() == "/upload/youtube/v3/videos"
            && ids.len() == 1
            && !ids[0].1.is_empty()
    })
}
pub(crate) fn http_error(response: &Response) -> CloudError {
    if response.status == 401 {
        return CloudError::auth();
    }
    let reason = response.json().ok().and_then(|v| {
        v.pointer("/error/errors/0/reason")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    if matches!(
        reason.as_deref(),
        Some(
            "quotaExceeded"
                | "dailyLimitExceeded"
                | "uploadLimitExceeded"
                | "rateLimitExceeded"
                | "userRateLimitExceeded"
        )
    ) || response.status == 429
    {
        return CloudError::new(
            "CLOUD_QUOTA",
            "YouTube 전송 한도에 도달했습니다. 원본을 유지하고 나중에 다시 시도합니다.",
            true,
        );
    }
    if response.status >= 500 || matches!(response.status, 408 | 425) {
        return CloudError::new(
            "CLOUD_REMOTE_RETRY",
            "YouTube가 요청을 완료하지 못했습니다. 잠시 후 다시 시도합니다.",
            true,
        );
    }
    if response.status == 403 {
        return CloudError::new(
            "CLOUD_PERMISSION",
            "이 채널에 영상을 업로드하거나 확인할 권한이 없습니다. 계정과 채널을 확인해 주세요.",
            false,
        );
    }
    CloudError::new(
        "CLOUD_REMOTE_REJECTED",
        "YouTube가 요청을 거부했습니다. 계정과 업로드 조건을 확인해 주세요.",
        false,
    )
}
pub(crate) fn api_get(
    transport: &dyn Transport,
    token: &str,
    resource: &str,
    pairs: &[(&str, &str)],
) -> Result<Value, CloudError> {
    if !matches!(resource, "channels" | "videos") {
        return Err(CloudError::protocol());
    }
    let mut url = Url::parse(&format!("https://www.googleapis.com/youtube/v3/{resource}"))
        .map_err(|_| CloudError::protocol())?;
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    let response = transport.send(Request {
        method: Method::GET,
        url: url.into(),
        bearer: Some(token.into()),
        headers: Vec::new(),
        body: Vec::new(),
    })?;
    if response.status != 200 {
        return Err(http_error(&response));
    }
    response.json()
}
pub(crate) fn create_session(
    transport: &dyn Transport,
    token: &str,
    title: &str,
    bytes: u64,
    binding: &str,
    visibility: CloudVisibility,
) -> Result<String, CloudError> {
    let metadata = json!({"snippet":{"title":title,"description":format!("Atsumi recording reference: {binding}"),"categoryId":"22"},"status":{"privacyStatus":visibility.as_str()}});
    let response = transport.send(Request {
        method: Method::POST, url: "https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status&notifySubscribers=false".into(), bearer: Some(token.into()),
        headers: vec![("Content-Type".into(), "application/json; charset=UTF-8".into()), ("X-Upload-Content-Type".into(), "application/octet-stream".into()), ("X-Upload-Content-Length".into(), bytes.to_string())],
        body: serde_json::to_vec(&metadata).map_err(|_| CloudError::protocol())?,
    })?;
    if !matches!(response.status, 200 | 201) {
        return Err(http_error(&response));
    }
    let location = response
        .header("location")?
        .ok_or_else(CloudError::protocol)?;
    if !valid_session_url(location) {
        return Err(CloudError::protocol());
    }
    Ok(location.into())
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    Offset(u64),
    Complete(String),
}
pub(crate) fn progress(response: Response, total: u64) -> Result<Progress, CloudError> {
    if response.status == 308 {
        let offset = match response.header("range")? {
            None => 0,
            Some(value) => {
                let end = value
                    .strip_prefix("bytes=0-")
                    .ok_or_else(CloudError::protocol)?;
                if end.is_empty() || end.len() > 20 || !end.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(CloudError::protocol());
                }
                end.parse::<u64>()
                    .ok()
                    .and_then(|v| v.checked_add(1))
                    .ok_or_else(CloudError::protocol)?
            }
        };
        if offset > total {
            return Err(CloudError::protocol());
        }
        return Ok(Progress::Offset(offset));
    }
    if matches!(response.status, 200 | 201) {
        let value = response.json()?;
        let id = value["id"]
            .as_str()
            .filter(|id| video_id(id))
            .ok_or_else(CloudError::protocol)?;
        return Ok(Progress::Complete(id.into()));
    }
    if matches!(response.status, 404 | 410) {
        return Err(CloudError::new("CLOUD_UPLOAD_SESSION_EXPIRED", "전송을 이어갈 수 없습니다. 중복 업로드를 막기 위해 중단했으니 YouTube에 영상이 올라갔는지 확인해 주세요.", false));
    }
    Err(http_error(&response))
}
pub(crate) fn query_session(
    transport: &dyn Transport,
    token: &str,
    url: &str,
    total: u64,
) -> Result<Progress, CloudError> {
    if !valid_session_url(url) {
        return Err(CloudError::protocol());
    }
    progress(
        transport.send(Request {
            method: Method::PUT,
            url: url.into(),
            bearer: Some(token.into()),
            headers: vec![
                ("Content-Length".into(), "0".into()),
                ("Content-Range".into(), format!("bytes */{total}")),
            ],
            body: Vec::new(),
        })?,
        total,
    )
}
pub(crate) fn send_chunk(
    transport: &dyn Transport,
    token: &str,
    url: &str,
    offset: u64,
    total: u64,
    bytes: Vec<u8>,
) -> Result<Progress, CloudError> {
    let end = offset
        .checked_add(bytes.len() as u64)
        .ok_or_else(CloudError::protocol)?;
    if !valid_session_url(url)
        || bytes.is_empty()
        || bytes.len() > CHUNK_BYTES
        || end > total
        || (end != total && bytes.len() % (256 * 1024) != 0)
    {
        return Err(CloudError::protocol());
    }
    progress(
        transport.send(Request {
            method: Method::PUT,
            url: url.into(),
            bearer: Some(token.into()),
            headers: vec![
                ("Content-Type".into(), "application/octet-stream".into()),
                ("Content-Length".into(), bytes.len().to_string()),
                (
                    "Content-Range".into(),
                    format!("bytes {offset}-{}/{total}", end - 1),
                ),
            ],
            body: bytes,
        })?,
        total,
    )
}
pub(crate) fn verify_remote(
    transport: &dyn Transport,
    token: &str,
    id: &str,
    owner: &str,
    seconds: f64,
    visibility: CloudVisibility,
) -> Result<bool, CloudError> {
    if !video_id(id) || !channel_id(owner) {
        return Err(CloudError::protocol());
    }
    let value = api_get(transport, token, "videos", &[("part", "snippet,status,contentDetails,processingDetails"), ("id", id), ("fields", "items(id,snippet/channelId,status/privacyStatus,status/uploadStatus,contentDetails/duration,processingDetails/processingStatus)")])?;
    let items = value["items"].as_array().ok_or_else(CloudError::protocol)?;
    if items.is_empty() {
        return Ok(false);
    }
    if items.len() != 1
        || items[0]["id"].as_str() != Some(id)
        || items[0]["snippet"]["channelId"].as_str() != Some(owner)
    {
        return Err(CloudError::new(
            "CLOUD_OWNER_MISMATCH",
            "업로드한 영상의 채널이 일치하지 않습니다. 원본은 유지됩니다.",
            false,
        ));
    }
    let item = &items[0];
    if item["status"]["privacyStatus"].as_str() != Some(visibility.as_str()) {
        return Err(CloudError::new(
            "CLOUD_PRIVACY_MISMATCH",
            "영상의 공개 범위가 업로드 설정과 다릅니다. YouTube 공개 설정을 확인해 주세요.",
            false,
        ));
    }
    let processing = item["processingDetails"]["processingStatus"].as_str();
    let upload = item["status"]["uploadStatus"].as_str();
    if processing == Some("failed") || matches!(upload, Some("failed" | "rejected" | "deleted")) {
        return Err(CloudError::new(
            "CLOUD_PROCESSING_FAILED",
            "YouTube 영상 처리가 실패했습니다. 로컬 원본은 유지됩니다.",
            false,
        ));
    }
    // `terminated` means processing details are no longer available, not failed.
    // Require the independent completed upload status before validating its media.
    if upload != Some("processed") || !matches!(processing, Some("succeeded" | "terminated")) {
        return Ok(false);
    }
    let duration = item["contentDetails"]["duration"]
        .as_str()
        .and_then(iso_duration)
        .ok_or_else(CloudError::protocol)?;
    if (duration - seconds).abs() > 2.0 {
        return Err(CloudError::new(
            "CLOUD_DURATION_MISMATCH",
            "업로드한 영상 길이가 녹화본과 일치하지 않습니다. 원본은 유지됩니다.",
            false,
        ));
    }
    Ok(true)
}
pub(crate) fn iso_duration(value: &str) -> Option<f64> {
    let value = value.strip_prefix("PT")?;
    if value.is_empty() || value.len() > 64 {
        return None;
    }
    let mut number = String::new();
    let mut total = 0.0;
    let mut previous = 0;
    for c in value.chars() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let (order, factor) = match c {
            'H' => (1, 3600.0),
            'M' => (2, 60.0),
            'S' => (3, 1.0),
            _ => return None,
        };
        if order <= previous || number.is_empty() {
            return None;
        }
        let component = number.parse::<f64>().ok()?;
        if !component.is_finite() || component < 0.0 {
            return None;
        }
        total += component * factor;
        previous = order;
        number.clear();
    }
    (number.is_empty() && total.is_finite() && total > 0.0 && total <= 43_202.0).then_some(total)
}
