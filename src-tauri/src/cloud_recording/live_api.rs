//! Native Live provisioning. Never accepts an endpoint/token from page IPC.
//! The durable observer owns admission and recovery; creation is never retried.
//! A prepared destination means ready to ingest, not a playable archive.
use super::{
    live_relay::{NativeDestinationOwner, PreparedLiveDestination},
    model::{channel_id, video_id, CloudError},
    upload::{Request, Response, Transport},
};
use reqwest::{blocking::Client, Method, Url};
use serde::Serialize;
use serde_json::{json, Value};
use std::{io::Read, time::Duration};
use uuid::Uuid;

const MAX_RESPONSE: usize = 512 * 1024;
const LIVE_SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
const FULL_SCOPE: &str = "https://www.googleapis.com/auth/youtube";

/// Created only from the native token endpoint's granted scopes. Deliberately
/// not serializable or Debug; the old upload-only grant cannot construct this.
pub(crate) struct LiveAuthorization {
    token: String,
    owner: NativeDestinationOwner,
}
impl LiveAuthorization {
    pub(super) fn archive_authorization(&self) -> Result<super::live_archive::Authorization, super::live_archive::ArchiveReason> {
        super::live_archive::Authorization::new(self.token.clone(), self.owner.channel_id.clone(), self.owner.generation)
    }
    pub(crate) fn from_granted_scopes(
        token: String,
        owner: NativeDestinationOwner,
        scopes: &str,
    ) -> Result<Self, CloudError> {
        if !scopes
            .split_ascii_whitespace()
            .any(|s| matches!(s, LIVE_SCOPE | FULL_SCOPE))
        {
            return Err(error(
                "CLOUD_LIVE_SCOPE",
                "실시간 전송을 위한 YouTube 권한 연결이 필요합니다.",
            ));
        }
        if token.is_empty()
            || token.len() > 16384
            || !token.bytes().all(|b| b.is_ascii_graphic())
            || !channel_id(&owner.channel_id)
            || owner.generation == 0
        {
            return Err(protocol());
        }
        Ok(Self { token, owner })
    }
}

pub(crate) struct PreparationPlan {
    pub request_id: Uuid,
    pub title: String,
    pub scheduled_start_ms: u64,
    /// Must be supplied by the user's selection, never inferred from a source.
    pub made_for_kids: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PreparationStage {
    Selected,
    CreatingStream,
    StreamCreated,
    CreatingBroadcast,
    BroadcastCreated,
    Binding,
    Prepared,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceReceipt {
    pub id: String,
    /// An ID in an invalid response is recovery information, not authority.
    pub owner_verified: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparationProgress {
    pub request_id: String,
    pub stage: PreparationStage,
    pub stream: Option<ResourceReceipt>,
    pub broadcast: Option<ResourceReceipt>,
    /// The last mutating request might have reached Google. Never auto-create
    /// another resource solely because its response was missing or malformed.
    pub outcome_uncertain: bool,
}
#[derive(Debug)]
pub(crate) struct PreparationFailure {
    pub error: CloudError,
    pub progress: PreparationProgress,
}
pub(crate) struct PreparationSuccess {
    pub destination: PreparedLiveDestination,
    pub progress: PreparationProgress,
}

fn error(code: &str, message: &str) -> CloudError {
    CloudError::new(code, message, false)
}
fn protocol() -> CloudError {
    error(
        "CLOUD_LIVE_RESPONSE",
        "YouTube 실시간 전송 응답을 확인하지 못했습니다. 생성 상태를 확인해 주세요.",
    )
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn remote_error(response: &Response) -> CloudError {
    let value = serde_json::from_slice::<Value>(&response.body).ok();
    match value
        .as_ref()
        .and_then(|v| v.pointer("/error/errors/0/reason"))
        .and_then(Value::as_str)
    {
        Some("liveStreamingNotEnabled") => error(
            "CLOUD_LIVE_NOT_ENABLED",
            "YouTube 실시간 스트리밍 활성화를 기다리고 있습니다.",
        ),
        Some("livePermissionBlocked") => error(
            "CLOUD_LIVE_BLOCKED",
            "YouTube에서 이 채널의 실시간 스트리밍을 허용하지 않았습니다.",
        ),
        Some("insufficientLivePermissions" | "insufficientPermissions") => error(
            "CLOUD_LIVE_SCOPE",
            "실시간 전송을 위한 YouTube 권한 연결이 필요합니다.",
        ),
        Some("quotaExceeded" | "dailyLimitExceeded" | "rateLimitExceeded") => error(
            "CLOUD_LIVE_QUOTA",
            "YouTube 요청 한도에 도달했습니다. 생성 상태를 확인한 뒤 다시 진행해 주세요.",
        ),
        _ if response.status == 401 => {
            error("CLOUD_AUTH_REQUIRED", "YouTube 연결을 다시 확인해 주세요.")
        }
        _ => error(
            "CLOUD_LIVE_REJECTED",
            "YouTube가 실시간 전송 준비 요청을 완료하지 못했습니다.",
        ),
    }
}

/// Separate from file-upload transport: adding Live does not broaden the old
/// resumable-upload endpoint policy or change its retry behavior.
pub(crate) struct GoogleLiveTransport(Client);
impl GoogleLiveTransport {
    pub(crate) fn new() -> Result<Self, CloudError> {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map(Self)
            .map_err(|_| protocol())
    }
}
fn allowed(request: &Request) -> bool {
    let Ok(url) = Url::parse(&request.url) else {
        return false;
    };
    request.method == Method::POST
        && url.scheme() == "https"
        && url.host_str() == Some("www.googleapis.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.port().is_none_or(|p| p == 443)
        && matches!(
            url.path(),
            "/youtube/v3/liveStreams"
                | "/youtube/v3/liveBroadcasts"
                | "/youtube/v3/liveBroadcasts/bind"
        )
        && request.body.len() <= 16384
        && request.bearer.is_some()
}
impl Transport for GoogleLiveTransport {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        if !allowed(&request) {
            return Err(protocol());
        }
        let response = self
            .0
            .post(&request.url)
            .bearer_auth(request.bearer.unwrap_or_default())
            .header("Content-Type", "application/json")
            .body(request.body)
            .send()
            .map_err(|_| {
                error(
                    "CLOUD_LIVE_NETWORK",
                    "YouTube 응답이 끊겼습니다. 중복 생성을 막기 위해 생성 상태 확인이 필요합니다.",
                )
            })?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        response
            .take((MAX_RESPONSE + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| protocol())?;
        if body.len() > MAX_RESPONSE {
            return Err(protocol());
        }
        Ok(Response {
            status,
            headers: Vec::new(),
            body,
        })
    }
}

fn request(
    auth: &LiveAuthorization,
    resource: &str,
    pairs: &[(&str, &str)],
    body: Value,
) -> Result<Request, CloudError> {
    let mut url = Url::parse(&format!("https://www.googleapis.com/youtube/v3/{resource}"))
        .map_err(|_| protocol())?;
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    let body = if body.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(&body).map_err(|_| protocol())?
    };
    Ok(Request {
        method: Method::POST,
        url: url.into(),
        bearer: Some(auth.token.clone()),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body,
    })
}
fn send(
    transport: &dyn Transport,
    request: Request,
    progress: &mut PreparationProgress,
) -> Result<Value, CloudError> {
    let response = transport.send(request)?;
    if response.body.len() > MAX_RESPONSE {
        return Err(protocol());
    }
    if !matches!(response.status, 200 | 201) {
        // A complete validation/auth rejection cannot have created a resource;
        // 5xx/timeouts remain ambiguous, even with a complete response body.
        if (400..500).contains(&response.status) && response.status != 408 {
            progress.outcome_uncertain = false;
        }
        return Err(remote_error(&response));
    }
    serde_json::from_slice(&response.body).map_err(|_| protocol())
}
fn receipt(value: &Value, owner: &str, video: bool) -> Option<ResourceReceipt> {
    let id = value["id"].as_str()?;
    if !(if video { video_id(id) } else { identifier(id) }) {
        return None;
    }
    Some(ResourceReceipt {
        id: id.into(),
        owner_verified: value["snippet"]["channelId"].as_str() == Some(owner),
    })
}
fn stream_valid(value: &Value, owner: &str) -> Result<String, CloudError> {
    if value["kind"] != "youtube#liveStream"
        || value["snippet"]["channelId"] != owner
        || value["cdn"]["ingestionType"] != "hls"
        || value["contentDetails"]["isReusable"] != false
    {
        return Err(protocol());
    }
    let target = value["cdn"]["ingestionInfo"]["ingestionAddress"]
        .as_str()
        .ok_or_else(protocol)?;
    crate::streaming::relay_hls::HlsTarget::from_api(target).map_err(|_| protocol())?;
    Ok(target.into())
}
fn broadcast_valid(
    value: &Value,
    owner: &str,
    made_for_kids: bool,
    stream_id: Option<&str>,
) -> Result<(), CloudError> {
    if value["kind"] != "youtube#liveBroadcast"
        || value["snippet"]["channelId"] != owner
        || value["status"]["privacyStatus"] != "unlisted"
        || value["status"]["selfDeclaredMadeForKids"] != made_for_kids
        || !matches!(
            value["status"]["lifeCycleStatus"].as_str(),
            Some("created" | "ready")
        )
    {
        return Err(protocol());
    }
    let details = &value["contentDetails"];
    for key in [
        "enableAutoStart",
        "enableAutoStop",
        "enableDvr",
        "enableEmbed",
        "recordFromStart",
    ] {
        if details[key] != true {
            return Err(protocol());
        }
    }
    if details["monitorStream"]["enableMonitorStream"] != false
        || stream_id.is_some_and(|id| details["boundStreamId"] != id)
    {
        return Err(protocol());
    }
    Ok(())
}

/// `observe` MUST durably record each receipt and check current selection,
/// account generation and shutdown before returning success. An error aborts
/// before the next mutation. Failure contains the latest known response IDs.
/// The caller must not call this again with the same uncertain attempt.
pub(crate) fn prepare(
    transport: &dyn Transport,
    auth: &LiveAuthorization,
    plan: &PreparationPlan,
    mut observe: impl FnMut(&PreparationProgress) -> Result<(), CloudError>,
) -> Result<PreparationSuccess, PreparationFailure> {
    let mut progress = PreparationProgress {
        request_id: plan.request_id.to_string(),
        stage: PreparationStage::Selected,
        stream: None,
        broadcast: None,
        outcome_uncertain: false,
    };
    let result = (|| {
        if plan.request_id.is_nil()
            || plan.title.trim().is_empty()
            || plan.title.chars().count() > 100
            || plan
                .title
                .chars()
                .any(|c| c.is_control() || matches!(c, '<' | '>'))
        {
            return Err(protocol());
        }
        let time = i64::try_from(plan.scheduled_start_ms)
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .ok_or_else(protocol)?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        observe(&progress)?;
        progress.stage = PreparationStage::CreatingStream;
        progress.outcome_uncertain = true;
        observe(&progress)?;
        let stream = send(
            transport,
            request(
                auth,
                "liveStreams",
                &[("part", "snippet,cdn,contentDetails,status")],
                json!({
                "snippet":{"title":plan.title,"description":format!("Atsumi live archive reference: {}",plan.request_id)}, "cdn":{"ingestionType":"hls","frameRate":"variable","resolution":"variable"},
                        "contentDetails":{"isReusable":false}
                    }),
            )?,
            &mut progress,
        )?;
        progress.stream = receipt(&stream, &auth.owner.channel_id, false);
        // Record even a valid ID in an otherwise invalid response for recovery.
        if progress.stream.as_ref().is_some_and(|r| r.owner_verified) {
            progress.outcome_uncertain = false;
        }
        observe(&progress)?;
        let stream_id = progress
            .stream
            .as_ref()
            .filter(|r| r.owner_verified)
            .ok_or_else(protocol)?
            .id
            .clone();
        let target = stream_valid(&stream, &auth.owner.channel_id)?;
        progress.stage = PreparationStage::StreamCreated;
        observe(&progress)?;

        progress.stage = PreparationStage::CreatingBroadcast;
        progress.outcome_uncertain = true;
        observe(&progress)?;
        let broadcast = send(
            transport,
            request(
                auth,
                "liveBroadcasts",
                &[("part", "snippet,status,contentDetails")],
                json!({
                    "snippet":{"title":plan.title,"scheduledStartTime":time,"description":format!("Atsumi live archive reference: {}",plan.request_id)},
                    "status":{"privacyStatus":"unlisted","selfDeclaredMadeForKids":plan.made_for_kids},
                    "contentDetails":{"enableAutoStart":true,"enableAutoStop":true,"enableDvr":true,"enableEmbed":true,"recordFromStart":true,
                        "monitorStream":{"enableMonitorStream":false}}
                }),
            )?,
            &mut progress,
        )?;
        progress.broadcast = receipt(&broadcast, &auth.owner.channel_id, true);
        if progress
            .broadcast
            .as_ref()
            .is_some_and(|r| r.owner_verified)
        {
            progress.outcome_uncertain = false;
        }
        observe(&progress)?;
        let broadcast_id = progress
            .broadcast
            .as_ref()
            .filter(|r| r.owner_verified)
            .ok_or_else(protocol)?
            .id
            .clone();
        broadcast_valid(&broadcast, &auth.owner.channel_id, plan.made_for_kids, None)?;
        progress.stage = PreparationStage::BroadcastCreated;
        observe(&progress)?;

        progress.stage = PreparationStage::Binding;
        progress.outcome_uncertain = true;
        observe(&progress)?;
        let bound = send(
            transport,
            request(
                auth,
                "liveBroadcasts/bind",
                &[
                    ("part", "snippet,status,contentDetails"),
                    ("id", &broadcast_id),
                    ("streamId", &stream_id),
                ],
                Value::Null,
            )?,
            &mut progress,
        )?;
        if bound["id"] != broadcast_id {
            return Err(protocol());
        }
        broadcast_valid(
            &bound,
            &auth.owner.channel_id,
            plan.made_for_kids,
            Some(&stream_id),
        )?;
        progress.outcome_uncertain = false;
        progress.stage = PreparationStage::Prepared;
        observe(&progress)?;
        PreparedLiveDestination::from_native_response(
            auth.owner.clone(),
            broadcast_id,
            stream_id,
            "unlisted",
            &target,
        )
        .map_err(|_| protocol())
    })();
    match result {
        Ok(destination) => Ok(PreparationSuccess {
            destination,
            progress,
        }),
        Err(error) => {
            // Best effort final receipt. Keep the original failure if recording
            // this receipt also fails; the caller has the returned partial IDs.
            let _ = observe(&progress);
            Err(PreparationFailure { error, progress })
        }
    }
}

#[cfg(test)]
#[path = "live_api_tests.rs"]
mod tests;
