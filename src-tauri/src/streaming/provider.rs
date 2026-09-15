//! Anonymous CHZZK metadata/chat adapter, separate from the official Open API.
//! Public protocol references (checked 2026-09-10):
//! https://github.com/kimcore/chzzk/blob/main/src/api/chat.ts
//! https://github.com/kimcore/chzzk/blob/main/src/const.ts
//! API JSON only: no media downloads, signed-CDN token forwarding or account cookies.

use std::{
    io::Read,
    time::{Duration, Instant},
};

use reqwest::{blocking::Client, header::LOCATION, redirect::Policy, Url};
use serde_json::Value;

use super::model::{LiveInfo, LiveStatus, StreamError};

const JSON_LIMIT: usize = 1024 * 1024;
const NOTICE: &str = "실험적 공개 웹 연결입니다. 서비스 변경에 따라 연결이 중단될 수 있습니다.";

#[derive(Clone)]
pub struct ChzzkProvider {
    client: Client,
}

impl ChzzkProvider {
    pub fn new() -> Result<Self, StreamError> {
        let client = Client::builder()
            .redirect(Policy::none())
            .referer(false)
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(15))
            .user_agent("Atsumi/1.6 (experimental anonymous CHZZK reader)")
            .build()
            .map_err(|_| {
                StreamError::new("network_setup", "공개 연결을 초기화하지 못했습니다.", false)
            })?;
        Ok(Self { client })
    }

    pub fn inspect(&self, input: &str) -> Result<LiveInfo, StreamError> {
        let channel_id = parse_channel(input)?;
        let detail = self.detail(&channel_id)?;
        Ok(live_info(&channel_id, &detail))
    }

    /// Scheduling metadata only. Access to restricted broadcasts is still
    /// decided by the official page and its existing authenticated profile.
    pub(crate) fn inspect_for_browser(&self, input: &str) -> Result<LiveInfo, StreamError> {
        let channel = parse_channel(input)?;
        let detail = self.detail(&channel)?;
        let mut info = live_info(&channel, &detail);
        info.status = if detail.get("status").and_then(Value::as_str) == Some("OPEN") {
            LiveStatus::Live
        } else {
            LiveStatus::Offline
        };
        Ok(info)
    }

    /// Public channel identity, independent of live/adult video access. One
    /// bounded anonymous request at recording start; never a playback request.
    /// Protocol reference: kimcore/chzzk src/client.ts channel() and api/channel.ts.
    pub fn channel_profile(&self, input: &str) -> Result<(String, Option<String>), StreamError> {
        let channel_id = parse_channel(input)?;
        let url = Url::parse(&format!(
            "https://api.chzzk.naver.com/service/v1/channels/{channel_id}"
        ))
        .map_err(|_| invalid_url())?;
        let content = self.json_content(&url, Duration::from_secs(4))?;
        if content.get("channelId").and_then(Value::as_str) != Some(channel_id.as_str()) {
            return Err(invalid_url());
        }
        let name = content
            .get("channelName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect::<String>();
        let image = content
            .get("channelImageUrl")
            .and_then(Value::as_str)
            .and_then(super::chat_assets::sanitize_chat_asset_url);
        Ok((name, image))
    }

    /// Read anonymous chat metadata independently of the selected video player.
    /// Only live-detail is requested; no playlist, playback fallback or DRM data
    /// is fetched. The chat token endpoint still enforces its own access rights.
    pub fn chat_context(&self, input: &str) -> Result<(LiveInfo, String), StreamError> {
        let channel_id = parse_channel(input)?;
        let detail = self.detail(&channel_id)?;
        chat_context_from_detail(&channel_id, &detail)
    }

    fn detail(&self, channel_id: &str) -> Result<Value, StreamError> {
        let url = Url::parse(&format!(
            "https://api.chzzk.naver.com/service/v2/channels/{channel_id}/live-detail"
        ))
        .map_err(|_| invalid_url())?;
        self.json_content(&url, Duration::from_secs(10))
    }

    fn json_content(&self, url: &Url, timeout: Duration) -> Result<Value, StreamError> {
        let bytes = self.fetch_with_timeout(url, JSON_LIMIT, timeout)?;
        let document: Value = serde_json::from_slice(&bytes).map_err(|_| {
            StreamError::new(
                "invalid_response",
                "공개 서비스 응답을 해석할 수 없습니다.",
                true,
            )
        })?;
        match document.get("code").and_then(Value::as_i64) {
            Some(200) => document
                .get("content")
                .filter(|v| !v.is_null())
                .cloned()
                .ok_or_else(|| {
                    StreamError::new("unavailable", "채널의 공개 정보가 없습니다.", false)
                }),
            Some(401 | 403 | 42601) => Err(StreamError::new(
                "restricted",
                "공개 연결의 접근 권한이 제한되었습니다.",
                false,
            )),
            Some(404) => Err(StreamError::new(
                "not_found",
                "채널을 찾을 수 없습니다.",
                false,
            )),
            Some(429) => Err(StreamError::new(
                "rate_limited",
                "요청이 잠시 제한되었습니다. 잠시 후 다시 시도하세요.",
                true,
            )),
            _ => Err(StreamError::new(
                "unavailable",
                "공개 서비스가 요청을 처리하지 못했습니다.",
                true,
            )),
        }
    }

    pub(crate) fn anonymous_chat_token(&self, channel_id: &str) -> Result<String, StreamError> {
        if !valid_chat_channel_id(channel_id) {
            return Err(StreamError::new(
                "invalid_chat",
                "채팅 채널 정보가 올바르지 않습니다.",
                false,
            ));
        }
        let mut url = Url::parse("https://comm-api.game.naver.com/nng_main/v1/chats/access-token")
            .map_err(|_| invalid_url())?;
        url.query_pairs_mut()
            .append_pair("channelId", channel_id)
            .append_pair("chatType", "STREAMING");
        let content = self.json_content(&url, Duration::from_secs(3))?;
        content
            .get("accessToken")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 8192)
            .map(str::to_owned)
            .ok_or_else(|| {
                StreamError::new(
                    "chat_unavailable",
                    "읽기 전용 채팅 연결을 받을 수 없습니다.",
                    false,
                )
            })
    }

    fn fetch_with_timeout(
        &self,
        url: &Url,
        limit: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, StreamError> {
        if limit == 0 || limit > JSON_LIMIT {
            return Err(StreamError::new(
                "response_limit",
                "응답 크기 제한이 올바르지 않습니다.",
                false,
            ));
        }
        let deadline = Instant::now() + timeout;
        validate_fetch_url(url)?;
        let mut current = url.clone();
        for _ in 0..=3 {
            validate_fetch_url(&current)?;
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    StreamError::new(
                        "network_timeout",
                        "공개 서비스 응답 시간이 초과되었습니다.",
                        true,
                    )
                })?;
            let mut response = self
                .client
                .get(current.clone())
                .timeout(remaining)
                .send()
                .map_err(|_| {
                    StreamError::new("network", "공개 서비스 연결이 일시적으로 끊겼습니다.", true)
                })?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(invalid_url)?;
                let next = current.join(location).map_err(|_| invalid_url())?;
                validate_fetch_url(&next)?;
                current = next;
                continue;
            }
            match response.status().as_u16() {
                200..=299 => {}
                401 | 403 => return Err(access_error()),
                404 => {
                    return Err(StreamError::new(
                        "not_found",
                        "요청한 공개 자료가 더 이상 없습니다.",
                        true,
                    ))
                }
                429 => {
                    return Err(StreamError::new(
                        "rate_limited",
                        "공개 서비스 요청이 잠시 제한되었습니다.",
                        true,
                    ))
                }
                _ => {
                    return Err(StreamError::new(
                        "upstream",
                        "공개 서비스가 응답하지 못했습니다.",
                        true,
                    ))
                }
            }
            if response
                .content_length()
                .is_some_and(|size| size > limit as u64)
            {
                return Err(StreamError::new(
                    "response_limit",
                    "응답이 허용된 크기를 초과했습니다.",
                    false,
                ));
            }
            let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
            response
                .by_ref()
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| {
                    StreamError::new(
                        "network_read",
                        "공개 자료를 받는 중 연결이 끊겼습니다.",
                        true,
                    )
                })?;
            if bytes.len() > limit {
                return Err(StreamError::new(
                    "response_limit",
                    "응답이 허용된 크기를 초과했습니다.",
                    false,
                ));
            }
            return Ok(bytes);
        }
        Err(StreamError::new(
            "redirect_limit",
            "공개 서비스 주소가 너무 많이 변경되었습니다.",
            false,
        ))
    }
}

pub fn normalize_channel_input(input: &str) -> Result<String, StreamError> {
    parse_channel(input)
}

fn parse_channel(input: &str) -> Result<String, StreamError> {
    let input = input.trim();
    let channel = if input.len() == 32 && input.bytes().all(|b| b.is_ascii_hexdigit()) {
        input.to_owned()
    } else {
        let url = Url::parse(input).map_err(|_| invalid_channel())?;
        if url.scheme() != "https"
            || url.host_str() != Some("chzzk.naver.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid_channel());
        }
        let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
        match parts.as_slice() {
            ["live", channel] | [channel] => (*channel).to_owned(),
            _ => return Err(invalid_channel()),
        }
    };
    if channel.len() != 32 || !channel.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid_channel());
    }
    Ok(channel.to_ascii_lowercase())
}

fn live_info(channel_id: &str, detail: &Value) -> LiveInfo {
    let restricted = detail.get("adult").and_then(Value::as_bool) == Some(true)
        || ["paidProduct", "watchPartyPaidProductId"]
            .iter()
            .any(|key| detail.get(*key).is_some_and(meaningful))
        || detail
            .get("blindType")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty() && value != "NONE");
    let status = if restricted {
        LiveStatus::Restricted
    } else if detail.get("status").and_then(Value::as_str) != Some("OPEN") {
        LiveStatus::Offline
    } else {
        LiveStatus::Live
    };
    let chat_available = status == LiveStatus::Live
        && detail.get("chatActive").and_then(Value::as_bool) != Some(false)
        && detail
            .get("chatChannelId")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
    LiveInfo {
        channel_id: channel_id.to_owned(),
        channel_name: bounded_text(
            detail
                .pointer("/channel/channelName")
                .and_then(Value::as_str)
                .unwrap_or(channel_id),
            200,
        ),
        title: bounded_text(
            detail
                .get("liveTitle")
                .and_then(Value::as_str)
                .unwrap_or(""),
            500,
        ),
        live_id: detail.get("liveId").and_then(|v| {
            v.as_u64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(|s| bounded_text(s, 80)))
        }),
        status,
        broadcast_started_at: detail
            .get("openDate")
            .and_then(Value::as_str)
            .and_then(parse_broadcast_start),
        chat_available,
        chat_channel_id: detail
            .get("chatChannelId")
            .and_then(Value::as_str)
            .filter(|value| chat_available && valid_chat_channel_id(value))
            .map(str::to_owned),
        notice: Some(NOTICE.into()),
    }
}

fn chat_context_from_detail(
    channel_id: &str,
    detail: &Value,
) -> Result<(LiveInfo, String), StreamError> {
    let info = live_info(channel_id, detail);
    match info.status {
        LiveStatus::Offline => {
            return Err(StreamError::new(
                "offline",
                "현재 방송 중이 아니어서 공개 채팅을 기록할 수 없습니다.",
                false,
            ))
        }
        LiveStatus::Restricted => {
            return Err(StreamError::new(
                "restricted",
                "인증·유료·연령 또는 접근 제한이 있는 방송의 채팅은 공개 연결로 지원하지 않습니다.",
                false,
            ))
        }
        LiveStatus::Unsupported => {
            return Err(StreamError::new(
                "chat_unavailable",
                "이 방송의 공개 채팅 정보를 확인할 수 없습니다.",
                false,
            ))
        }
        LiveStatus::Live => {}
    }
    if !info.chat_available {
        return Err(StreamError::new(
            "chat_unavailable",
            "이 방송의 공개 채팅이 비활성화되어 있거나 채널 정보가 없습니다.",
            false,
        ));
    }
    let chat_id = detail
        .get("chatChannelId")
        .and_then(Value::as_str)
        .filter(|value| valid_chat_channel_id(value))
        .ok_or_else(|| {
            StreamError::new("invalid_chat", "채팅 채널 정보가 올바르지 않습니다.", false)
        })?;
    Ok((info, chat_id.to_owned()))
}

pub(super) fn valid_chat_channel_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn parse_broadcast_start(value: &str) -> Option<u64> {
    // CHZZK's public openDate is Korean local time when no offset is present.
    // Never infer a broadcast start from the recording start or local PC timezone.
    use chrono::{DateTime, FixedOffset, NaiveDateTime};
    if value.len() > 64 {
        return None;
    }
    let timestamp = DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.timestamp_millis())
        .or_else(|| {
            let date = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok()?;
            Some(
                date.and_local_timezone(FixedOffset::east_opt(9 * 3600)?)
                    .single()?
                    .timestamp_millis(),
            )
        })?;
    u64::try_from(timestamp).ok().filter(|stamp| *stamp > 0)
}

fn meaningful(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_i64() != Some(0),
        Value::String(text) => !text.is_empty() && !matches!(text.as_str(), "NONE" | "0" | "false"),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
        Value::Bool(true) => true,
    }
}

fn bounded_text(value: &str, count: usize) -> String {
    value.chars().take(count).collect()
}
fn invalid_channel() -> StreamError {
    StreamError::new(
        "invalid_channel",
        "치지직 채널 주소 또는 32자리 채널 ID를 입력하세요.",
        false,
    )
}
fn invalid_url() -> StreamError {
    StreamError::new(
        "unsupported_url",
        "허용되지 않거나 지원되지 않는 공개 서비스 주소입니다.",
        false,
    )
}
fn is_api_host(host: &str) -> bool {
    matches!(host, "api.chzzk.naver.com" | "comm-api.game.naver.com")
}
fn validate_fetch_url(url: &Url) -> Result<(), StreamError> {
    if url.scheme() != "https"
        || url.port().is_some_and(|port| port != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > 16384
        || !url.host_str().is_some_and(is_api_host)
    {
        return Err(invalid_url());
    }
    Ok(())
}
fn access_error() -> StreamError {
    StreamError::new(
        "restricted",
        "공개 연결의 접근 권한이 제한되었습니다.",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_open_date_uses_kst_and_rejects_unknown_dates() {
        let expected = parse_broadcast_start("2026-09-10T10:43:42Z");
        assert_eq!(parse_broadcast_start("2026-09-10 19:43:42"), expected);
        assert_eq!(parse_broadcast_start("2026-09-10T19:43:42+09:00"), expected);
        assert!(expected.is_some());
        for value in ["", "unknown", "2026-02-30 19:43:42", "2026-09-10 25:43:42"] {
            assert_eq!(parse_broadcast_start(value), None);
        }
    }
    #[test]
    fn channel_and_url_validation_rejects_arbitrary_targets() {
        assert_eq!(
            parse_channel("https://chzzk.naver.com/live/fedb21a1e1d5f9862eb47b15c69beadd").unwrap(),
            "fedb21a1e1d5f9862eb47b15c69beadd"
        );
        for input in [
            "https://localhost/live/fedb21a1e1d5f9862eb47b15c69beadd",
            "https://chzzk.naver.com@evil.test/live/fedb21a1e1d5f9862eb47b15c69beadd",
            "file:///etc/passwd",
            "fedb21a1e1d5f9862eb47b15c69bead",
            "fedb21a1e1d5f9862eb47b15c69beadd0",
            "gedb21a1e1d5f9862eb47b15c69beadd",
            "https://chzzk.naver.com/live/%66edb21a1e1d5f9862eb47b15c69beadd",
            "https://chzzk.naver.com/video/fedb21a1e1d5f9862eb47b15c69beadd",
        ] {
            assert!(parse_channel(input).is_err());
        }
        for input in [
            "https://127.0.0.1/a.m3u8",
            "https://livecloud.pstatic.net.evil.test/a.m3u8",
            "http://livecloud.pstatic.net/a.m3u8",
            "https://livecloud.pstatic.net:8080/a.m3u8",
            "https://user:pass@livecloud.pstatic.net/a.m3u8",
        ] {
            assert!(validate_fetch_url(&Url::parse(input).unwrap()).is_err());
        }
    }
    #[test]
    fn restrictions_and_offline_state_are_not_resolved_as_live() {
        let detail = serde_json::json!({"status":"CLOSE", "adult":false, "livePlaybackJson":"{\"media\":null}"});
        assert_eq!(live_info("test", &detail).status, LiveStatus::Offline);
        let restricted = serde_json::json!({"status":"OPEN", "adult":true});
        assert_eq!(
            live_info("test", &restricted).status,
            LiveStatus::Restricted
        );
    }

    #[test]
    fn chat_context_uses_broadcast_metadata_without_any_video_information() {
        let channel = "fedb21a1e1d5f9862eb47b15c69beadd";
        let detail = serde_json::json!({
            "status":"OPEN", "adult":false, "chatActive":true, "chatChannelId":"chat_A-123",
            "liveTitle":"Chat only context", "liveId":123, "openDate":"2026-09-10 19:43:42",
            "channel":{"channelName":"Broadcaster"}, "timeMachineActive":true
        });
        // Metadata and chat do not require or fetch any playback manifest.
        assert_eq!(live_info(channel, &detail).status, LiveStatus::Live);
        let (info, chat_id) = chat_context_from_detail(channel, &detail).unwrap();
        assert_eq!(chat_id, "chat_A-123");
        assert_eq!(info.chat_channel_id.as_deref(), Some("chat_A-123"));
        assert!(!serde_json::to_string(&info).unwrap().contains("chat_A-123"));
        assert!(!serde_json::to_value(&info)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("chatChannelId"));
        assert_eq!(info.status, LiveStatus::Live);
        assert!(info.chat_available);
        assert_eq!(info.channel_name, "Broadcaster");
        assert_eq!(info.live_id.as_deref(), Some("123"));
        assert_eq!(
            info.broadcast_started_at,
            parse_broadcast_start("2026-09-10T10:43:42Z")
        );
    }

    #[test]
    fn public_chat_context_does_not_depend_on_video_protocol_or_drm_fields() {
        for playback in [
            Value::Null,
            serde_json::json!("not valid playback JSON"),
            serde_json::json!({"media":[{"protocol":"LLHLS", "mediaId":"LLHLS", "drm":{"type":"widevine"}}]}),
        ] {
            let detail = serde_json::json!({"status":"OPEN", "chatChannelId":"12345", "livePlaybackJson":playback});
            let (info, chat_id) = chat_context_from_detail("channel", &detail).unwrap();
            assert_eq!(info.status, LiveStatus::Live);
            assert_eq!(chat_id, "12345");
        }
    }

    #[test]
    fn chat_context_honors_broadcast_restrictions_offline_and_disabled_chat() {
        for restriction in [
            serde_json::json!({"adult":true}),
            serde_json::json!({"paidProduct":{"id":1}}),
            serde_json::json!({"watchPartyPaidProductId":"product"}),
            serde_json::json!({"blindType":"BLOCK"}),
        ] {
            let mut detail = serde_json::json!({"status":"OPEN", "chatChannelId":"12345"});
            detail
                .as_object_mut()
                .unwrap()
                .extend(restriction.as_object().unwrap().clone());
            let denied = chat_context_from_detail("channel", &detail).unwrap_err();
            assert_eq!(denied.code, "restricted");
            assert!(!denied.retryable);
        }
        for status in ["CLOSE", "UNKNOWN", ""] {
            let detail = serde_json::json!({"status":status,"chatChannelId":"12345"});
            assert_eq!(
                chat_context_from_detail("channel", &detail)
                    .unwrap_err()
                    .code,
                "offline"
            );
        }
        for detail in [
            serde_json::json!({"status":"OPEN","chatActive":false,"chatChannelId":"12345"}),
            serde_json::json!({"status":"OPEN"}),
            serde_json::json!({"status":"OPEN","chatChannelId":""}),
        ] {
            assert_eq!(
                chat_context_from_detail("channel", &detail)
                    .unwrap_err()
                    .code,
                "chat_unavailable"
            );
        }
    }

    #[test]
    fn chat_context_shares_token_channel_id_validation() {
        for chat_id in ["chat_ABC-123".into(), "a".repeat(128)] {
            let detail = serde_json::json!({"status":"OPEN", "chatChannelId":chat_id});
            assert!(chat_context_from_detail("channel", &detail).is_ok());
        }
        for chat_id in [
            "../private".into(),
            "id?token=secret".into(),
            "chat room".into(),
            "채팅".into(),
            "a".repeat(129),
        ] {
            let detail = serde_json::json!({"status":"OPEN", "chatChannelId":chat_id});
            let denied = chat_context_from_detail("channel", &detail).unwrap_err();
            assert!(live_info("channel", &detail).chat_channel_id.is_none());
            assert_eq!(denied.code, "invalid_chat");
            assert!(!denied.message.contains("secret"));
        }
    }
    #[test]
    fn invalid_response_bounds_fail_without_network() {
        let provider = ChzzkProvider::new().unwrap();
        let url =
            Url::parse("https://api.chzzk.naver.com/service/v2/channels/id/live-detail").unwrap();
        assert_eq!(
            provider
                .fetch_with_timeout(&url, 0, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "response_limit"
        );
        assert_eq!(
            provider
                .fetch_with_timeout(&url, JSON_LIMIT + 1, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "response_limit"
        );
    }
    #[test]
    fn media_hosts_are_rejected_before_fetch_and_api_denials_never_retry() {
        let provider = ChzzkProvider::new().unwrap();
        for endpoint in [
            "https://livecloud.pstatic.net/chzzk/master.m3u8",
            "https://nvelop-livecloud.pstatic.net/chzzk/segment.m4v",
            "https://livecloud.akamaized.net/chzzk/segment.m4v",
            "https://ssl.pstatic.net/image.png",
        ] {
            assert_eq!(
                provider
                    .fetch_with_timeout(
                        &Url::parse(endpoint).unwrap(),
                        JSON_LIMIT,
                        Duration::from_secs(1)
                    )
                    .unwrap_err()
                    .code,
                "unsupported_url"
            );
        }
        for endpoint in [
            "https://api.chzzk.naver.com/service/v2/channels/id/live-detail",
            "https://comm-api.game.naver.com/nng_main/v1/chats/access-token",
        ] {
            assert!(validate_fetch_url(&Url::parse(endpoint).unwrap()).is_ok());
        }
        let denied = access_error();
        assert_eq!(denied.code, "restricted");
        assert!(!denied.retryable);
    }
}
