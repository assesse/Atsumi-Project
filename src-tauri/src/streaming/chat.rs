//! Anonymous, receive-only public web chat protocol. This is not the official
//! OAuth/Socket.IO Session API. Primary implementation references:
//! https://github.com/kimcore/chzzk/blob/main/src/chat/chat.ts
//! https://github.com/kimcore/chzzk/blob/main/src/chat/types.ts
//! https://github.com/kimcore/chzzk/blob/main/src/api/chat.ts

use std::{
    io::ErrorKind,
    net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tungstenite::{client_tls_with_config, protocol::WebSocketConfig, Message};

use super::{
    model::{ChatBadge, ChatBadgeKind, ChatEmoji, ChatEvent, ChatRich, ChatState, StreamError},
    provider::ChzzkProvider,
};

const MAX_MESSAGE: usize = 256 * 1024;
pub type ChatCallback = Box<dyn FnMut(ChatEvent) + Send + 'static>;

/// Run on a dedicated thread. The callback must be quick and must not block on UI work.
pub fn run_chat(
    provider: ChzzkProvider,
    chat_channel_id: String,
    stop: Arc<AtomicBool>,
    mut on_event: ChatCallback,
) {
    let mut failures: u32 = 0;
    while !stop.load(Ordering::Acquire) {
        on_event(ChatEvent::Status(
            if failures == 0 {
                ChatState::Connecting
            } else {
                ChatState::Reconnecting
            },
            (failures > 0).then(|| {
                "채팅 연결이 끊겼습니다. 중단 구간의 메시지는 복구되지 않을 수 있습니다.".to_owned()
            }),
        ));
        let mut reached_connected = false;
        let result = connect_and_read(
            &provider,
            &chat_channel_id,
            &stop,
            &mut on_event,
            &mut reached_connected,
        );
        if reached_connected {
            failures = 0;
        }
        match result {
            Ok(()) => break,
            Err(_) if stop.load(Ordering::Acquire) => break,
            Err(error) if !error.retryable => {
                on_event(ChatEvent::Status(ChatState::Failed, Some(error.message)));
                return;
            }
            Err(_) => {
                failures = failures.saturating_add(1);
                // Bound repeated failed authentication/DNS attempts while permitting long sessions.
                if failures >= 12 {
                    on_event(ChatEvent::Status(
                        ChatState::Failed,
                        Some("채팅에 반복해서 연결하지 못했습니다. 영상 저장은 계속됩니다.".into()),
                    ));
                    return;
                }
                pause(
                    &stop,
                    Duration::from_secs((1_u64 << failures.min(5)).min(30)),
                );
            }
        }
    }
    on_event(ChatEvent::Status(ChatState::Stopped, None));
}

fn connect_and_read(
    provider: &ChzzkProvider,
    channel: &str,
    stop: &Arc<AtomicBool>,
    on_event: &mut ChatCallback,
    reached_connected: &mut bool,
) -> Result<(), StreamError> {
    // Refresh the anonymous token for every new connection. Never use account credentials.
    let token = provider.anonymous_chat_token(channel)?;
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let server_id = channel.bytes().map(u32::from).sum::<u32>() % 9 + 1;
    let host = format!("kr-ss{server_id}.chat.naver.com");
    let addresses = resolve_chat_host(host.clone(), stop)?;
    let mut stream = None;
    for address in addresses.into_iter().take(4) {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if !is_public_address(address) {
            continue;
        }
        if let Ok(socket) = TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            stream = Some(socket);
            break;
        }
    }
    let stream = stream.ok_or_else(network_error)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| network_error())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| network_error())?;
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    // Closing this cloned socket interrupts even a blocked TLS read when the user stops.
    let watch_socket = stream.try_clone().map_err(|_| network_error())?;
    let connection_done = Arc::new(AtomicBool::new(false));
    let handshake_done = Arc::new(AtomicBool::new(false));
    let _guard = ConnectionGuard(connection_done.clone());
    let watch_stop = stop.clone();
    let watch_handshake = handshake_done.clone();
    thread::spawn(move || {
        let handshake_deadline = Instant::now() + Duration::from_secs(4);
        while !connection_done.load(Ordering::Acquire) {
            if watch_stop.load(Ordering::Acquire)
                || (!watch_handshake.load(Ordering::Acquire)
                    && Instant::now() >= handshake_deadline)
            {
                let _ = watch_socket.shutdown(Shutdown::Both);
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_MESSAGE);
    config.max_frame_size = Some(MAX_MESSAGE);
    config.write_buffer_size = 0;
    config.max_write_buffer_size = MAX_MESSAGE;
    let url = format!("wss://{host}/chat");
    // The host is derived only from validated chat ID; TLS certificate verification remains enabled.
    let (mut socket, _) = client_tls_with_config(url.as_str(), stream, Some(config), None)
        .map_err(|_| network_error())?;
    handshake_done.store(true, Ordering::Release);
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    socket
        .send(Message::Text(
            json!({
                "ver":"2", "cmd":100, "svcid":"game", "cid":channel, "tid":1,
                "bdy":{"uid":null, "devType":2001, "accTkn":token, "auth":"READ"}
            })
            .to_string()
            .into(),
        ))
        .map_err(|_| network_error())?;
    let mut connected = false;
    let started = Instant::now();
    let mut last_received = Instant::now();
    let mut last_ping = Instant::now();
    while !stop.load(Ordering::Acquire) {
        if (!connected && started.elapsed() > Duration::from_secs(8))
            || last_received.elapsed() > Duration::from_secs(65)
        {
            return Err(network_error());
        }
        if last_ping.elapsed() > Duration::from_secs(20) {
            socket
                .send(Message::Text(json!({"ver":"2","cmd":0}).to_string().into()))
                .map_err(|_| network_error())?;
            last_ping = Instant::now();
        }
        let message = match socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue
            }
            Err(_) => return Err(network_error()),
        };
        let raw = match message {
            Message::Text(text) => text.to_string(),
            Message::Ping(_) => {
                socket.flush().map_err(|_| network_error())?;
                continue;
            }
            Message::Pong(_) => {
                last_received = Instant::now();
                continue;
            }
            Message::Close(_) => return Err(network_error()),
            _ => continue,
        };
        if raw.len() > MAX_MESSAGE {
            return Err(protocol_error());
        }
        let document: Value = serde_json::from_str(&raw).map_err(|_| protocol_error())?;
        last_received = Instant::now();
        if document
            .get("retCode")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0)
        {
            return Err(StreamError::new(
                "chat_restricted",
                "채팅 서버가 공개 읽기 연결을 허용하지 않았습니다.",
                false,
            ));
        }
        match document.get("cmd").and_then(Value::as_i64) {
            Some(10100) => {
                connected = true;
                *reached_connected = true;
                on_event(ChatEvent::Status(
                    ChatState::Connected,
                    Some("공개 읽기 전용 채팅입니다. 재접속 중 누락이 생길 수 있습니다.".into()),
                ));
            }
            Some(0) => {
                socket
                    .send(Message::Text(
                        json!({"ver":"2","cmd":10000}).to_string().into(),
                    ))
                    .map_err(|_| network_error())?;
            }
            Some(93101) if connected => {
                if let Some(messages) = chat_messages(&document) {
                    for message in messages.iter().take(1000) {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        if let Some(event) = parse_message(message) {
                            on_event(event);
                        }
                    }
                }
            }
            Some(94005 | 94006 | 94015) => {
                return Err(StreamError::new(
                    "chat_restricted",
                    "채팅 접근이 제한되어 연결을 종료했습니다.",
                    false,
                ))
            }
            _ => {}
        }
    }
    let _ = socket.close(None);
    Ok(())
}

fn chat_messages(document: &Value) -> Option<&[Value]> {
    let body = document.get("bdy")?;
    // The public upstream reader accepts both a direct array and messageList.
    // No recent-history request is sent; this only normalizes received envelopes.
    body.get("messageList")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
        .map(Vec::as_slice)
}

pub(super) fn parse_message(message: &Value) -> Option<ChatEvent> {
    if message
        .get("msgStatusType")
        .or_else(|| message.get("messageStatusType"))
        .and_then(Value::as_str)
        == Some("HIDDEN")
    {
        return None;
    }
    let kind = message
        .get("msgTypeCode")
        .or_else(|| message.get("messageTypeCode"))
        .and_then(Value::as_u64)?;
    if kind != 1 {
        return None;
    }
    let text: String = message
        .get("msg")
        .or_else(|| message.get("content"))
        .and_then(Value::as_str)?
        .chars()
        .take(4096)
        .collect();
    let profile = metadata_object(message.get("profile")).unwrap_or(Value::Null);
    let sender = profile
        .get("nickname")
        .and_then(Value::as_str)
        .unwrap_or("알 수 없음")
        .chars()
        .take(128)
        .collect();
    let server_time = message
        .get("msgTime")
        .or_else(|| message.get("messageTime"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        });
    Some(ChatEvent::Message {
        sender,
        server_time,
        rich: parse_rich(&profile, message.get("extras"), &text),
        text,
    })
}

fn metadata_object(value: Option<&Value>) -> Option<Value> {
    let value = match value? {
        Value::String(raw) if raw.len() <= 32 * 1024 => serde_json::from_str(raw).ok()?,
        value @ Value::Object(_) if serde_json::to_vec(value).ok()?.len() <= 32 * 1024 => {
            value.clone()
        }
        _ => return None,
    };
    value.is_object().then_some(value)
}

fn supplied_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn supplied_badge(
    kind: ChatBadgeKind,
    value: &Value,
    title: Option<String>,
    id: Option<String>,
) -> Option<ChatBadge> {
    Some(ChatBadge {
        kind,
        id,
        title,
        image_url: value.get("imageUrl")?.as_str()?.to_owned(),
    })
}

fn parse_rich(profile: &Value, extras: Option<&Value>, text: &str) -> Option<ChatRich> {
    // Public web protocol fields are documented by kimcore/chzzk src/chat/types.ts.
    // viewerBadges is additionally modeled by chzzkpy/unofficial/chat/profile.py.
    // Badge IDs, titles, and images are preserved as supplied, without inferring
    // paid privileges, founder status, or account identity from them.
    let property = &profile["streamingProperty"];
    let nickname_color = [
        &property["nicknameColor"]["colorCode"],
        &profile["title"]["color"],
    ]
    .into_iter()
    .filter_map(Value::as_str)
    .find(|color| {
        color.len() == 7
            && color.starts_with('#')
            && color.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit())
    })
    .map(str::to_owned);
    let mut badges = Vec::new();
    let subscription = &property["subscription"];
    if let Some(badge) = supplied_badge(
        ChatBadgeKind::Subscription,
        &subscription["badge"],
        subscription["accumulativeMonth"]
            .as_u64()
            .map(|month| format!("구독 {month}개월")),
        None,
    ) {
        badges.push(badge);
    }
    if let Some(badge) = supplied_badge(
        ChatBadgeKind::Profile,
        &profile["badge"],
        supplied_string(profile["title"].get("name")),
        None,
    ) {
        badges.push(badge);
    }
    let donation_badge = &property["realTimeDonationRanking"]["badge"];
    if let Some(badge) = supplied_badge(
        ChatBadgeKind::Donation,
        donation_badge,
        supplied_string(donation_badge.get("title")),
        None,
    ) {
        badges.push(badge);
    }
    for badge in profile["activityBadges"]
        .as_array()
        .into_iter()
        .flatten()
        .take(32)
    {
        if badge["activated"].as_bool() != Some(true) {
            continue;
        }
        if let Some(badge) = supplied_badge(
            ChatBadgeKind::Activity,
            badge,
            supplied_string(badge.get("title").or_else(|| badge.get("name"))),
            supplied_string(badge.get("badgeId")),
        ) {
            badges.push(badge);
        }
    }
    for viewer in profile["viewerBadges"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
    {
        let badge = &viewer["badge"];
        if let Some(badge) = supplied_badge(
            ChatBadgeKind::Profile,
            badge,
            supplied_string(badge.get("title").or_else(|| badge.get("name"))),
            supplied_string(
                badge
                    .get("badgeId")
                    .or_else(|| badge.get("badge_id"))
                    .or_else(|| viewer.get("type")),
            ),
        ) {
            badges.push(badge);
        }
    }
    let extras = metadata_object(extras).unwrap_or(Value::Null);
    let emojis = extras["emojis"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, _)| text.contains(&format!("{{:{id}:}}")))
        .filter_map(|(id, url)| {
            Some(ChatEmoji {
                id: id.clone(),
                image_url: url.as_str()?.to_owned(),
            })
        })
        .take(32)
        .collect();
    ChatRich {
        nickname_color,
        badges,
        emojis,
    }
    .bounded()
}

fn resolve_chat_host(host: String, stop: &AtomicBool) -> Result<Vec<SocketAddr>, StreamError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = (host.as_str(), 443)
            .to_socket_addrs()
            .map(|addresses| addresses.take(8).collect::<Vec<_>>());
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(addresses)) => return Ok(addresses),
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    Err(network_error())
}

fn is_public_address(address: SocketAddr) -> bool {
    match address.ip() {
        std::net::IpAddr::V4(ip) => {
            !(ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast())
        }
        std::net::IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
    }
}

fn pause(stop: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
}
fn network_error() -> StreamError {
    StreamError::new(
        "chat_network",
        "공개 채팅 연결이 일시적으로 끊겼습니다.",
        true,
    )
}
fn protocol_error() -> StreamError {
    StreamError::new(
        "chat_protocol",
        "지원되지 않는 공개 채팅 응답을 받았습니다.",
        false,
    )
}
struct ConnectionGuard(Arc<AtomicBool>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_public_text_and_preserves_server_time() {
        let value = json!({"msgTypeCode":1,"msg":"hello","msgTime":1720000000123_u64,"profile":"{\"nickname\":\"viewer\"}"});
        match parse_message(&value).unwrap() {
            ChatEvent::Message {
                sender,
                text,
                server_time,
                ..
            } => {
                assert_eq!(sender, "viewer");
                assert_eq!(text, "hello");
                assert_eq!(server_time, Some(1720000000123));
            }
            _ => panic!("expected message"),
        }
        let hidden =
            json!({"msgTypeCode":1,"msgStatusType":"HIDDEN","msg":"hidden","profile":"{}"});
        assert!(parse_message(&hidden).is_none());
        assert!(
            parse_message(&json!({"msgTypeCode":10,"msg":"donation","profile":"{}"})).is_none()
        );
    }
    #[test]
    fn unsafe_network_addresses_are_rejected() {
        for address in [
            "127.0.0.1:443",
            "10.0.0.1:443",
            "192.168.0.1:443",
            "[::1]:443",
            "[fd00::1]:443",
        ] {
            assert!(!is_public_address(address.parse().unwrap()));
        }
    }
    #[test]
    fn public_chat_accepts_array_and_wrapped_message_envelopes() {
        let message = json!({"messageTypeCode":1,"content":"fixture","messageTime":"1720000000123","profile":{"nickname":"viewer"}});
        for envelope in [
            json!({"cmd":93101,"bdy":[message.clone()]}),
            json!({"cmd":93101,"bdy":{"messageList":[message.clone()]}}),
        ] {
            let messages = chat_messages(&envelope).unwrap();
            assert_eq!(messages.len(), 1);
            assert!(matches!(
                parse_message(&messages[0]),
                Some(ChatEvent::Message {
                    server_time: Some(1720000000123),
                    ..
                })
            ));
        }
        assert!(chat_messages(&json!({"cmd":93101,"bdy":{"other":[]}})).is_none());
    }

    #[test]
    fn keeps_supplied_decorations_without_storing_raw_profile_or_extras() {
        let profile = json!({
            "nickname":"viewer", "userIdHash":"must-not-be-stored",
            "title":{"name":"supplied profile title","color":"#123abc"},
            "badge":{"imageUrl":"https://ssl.pstatic.net/profile.png"},
            "streamingProperty":{
                "nicknameColor":{"colorCode":"#A1B2C3"},
                "subscription":{"accumulativeMonth":13,"tier":2,"badge":{"imageUrl":"https://nng-phinf.pstatic.net/sub.png"}},
                "realTimeDonationRanking":{"badge":{"title":"supplied ranking title","imageUrl":"https://ssl.pstatic.net/donation.png"}}
            },
            "activityBadges":[
                {"badgeId":"active-id","title":"supplied activity title","imageUrl":"https://ssl.pstatic.net/active.png","activated":true},
                {"badgeId":"hidden-id","title":"hidden badge","imageUrl":"https://ssl.pstatic.net/hidden.png","activated":false}
            ],
            "viewerBadges":[{"type":"uninterpreted-type","badge":{"imageUrl":"https://ssl.pstatic.net/viewer.png"}}]
        });
        let extras = json!({"extraToken":"must-not-be-stored", "emojis":{
            "wave_1":"https://nng-phinf.pstatic.net/wave.png?type=w80",
            "unused":"https://nng-phinf.pstatic.net/unused.png"
        }});
        for (profile, extras) in [
            (profile.clone(), extras.clone()),
            (
                Value::String(profile.to_string()),
                Value::String(extras.to_string()),
            ),
        ] {
            let event = parse_message(&json!({"msgTypeCode":1,"msg":"hello {:wave_1:} {:missing:}","profile":profile,"extras":extras})).unwrap();
            let ChatEvent::Message {
                sender,
                text,
                rich: Some(rich),
                ..
            } = event
            else {
                panic!("rich message expected")
            };
            assert_eq!(sender, "viewer");
            assert_eq!(text, "hello {:wave_1:} {:missing:}");
            assert_eq!(rich.nickname_color.as_deref(), Some("#A1B2C3"));
            assert_eq!(rich.badges.len(), 5);
            assert_eq!(rich.badges[0].kind, ChatBadgeKind::Subscription);
            assert_eq!(rich.badges[0].title.as_deref(), Some("구독 13개월"));
            assert_eq!(rich.badges[4].kind, ChatBadgeKind::Profile);
            assert_eq!(rich.badges[4].id.as_deref(), Some("uninterpreted-type"));
            assert_eq!(rich.badges[4].title, None);
            assert_eq!(rich.emojis.len(), 1);
            assert_eq!(rich.emojis[0].id, "wave_1");
            let serialized = serde_json::to_string(&rich).unwrap();
            for secret in [
                "userIdHash",
                "extraToken",
                "must-not-be-stored",
                "hidden badge",
                "unused",
            ] {
                assert!(!serialized.contains(secret));
            }
        }
    }

    #[test]
    fn broken_or_oversized_metadata_does_not_drop_text_and_unsafe_urls_are_omitted() {
        for metadata in [
            Value::Null,
            json!("not json"),
            json!(" ".repeat(32 * 1024 + 1)),
            json!([]),
        ] {
            let event = parse_message(
                &json!({"msgTypeCode":1,"msg":"preserved","profile":metadata,"extras":metadata}),
            );
            assert!(
                matches!(event, Some(ChatEvent::Message { text, rich:None, .. }) if text == "preserved")
            );
        }
        let event = parse_message(&json!({"msgTypeCode":1,"msg":"{:wave:}","profile":{
            "nickname":"viewer", "streamingProperty":{"nicknameColor":{"colorCode":"url(https://example.com)"}},
            "badge":{"imageUrl":"https://ssl.pstatic.net.evil.example/badge.png"}
        }, "extras":{"emojis":{"wave":"file:///private.png"}}}));
        assert!(
            matches!(event, Some(ChatEvent::Message { text, rich:None, .. }) if text == "{:wave:}")
        );
    }

    #[test]
    fn recognizes_only_literal_received_nickname_colors() {
        let parse = |color: &str, legacy: &str| {
            parse_rich(
                &json!({
                    "streamingProperty":{"nicknameColor":{"colorCode":color}}, "title":{"color":legacy}
                }),
                None,
                "",
            )
        };
        assert_eq!(
            parse("DEFAULT", "#123456")
                .unwrap()
                .nickname_color
                .as_deref(),
            Some("#123456")
        );
        assert!(parse("#123", "gradient").is_none());
        assert!(parse("DEFAULT", "").is_none());
    }
}
