//! Anonymous, receive-only SOOP chat. Current protocol was checked against the
//! official LivePlayer.js?_=202610071400 and public guest frames on 2026-10-13.
//! Only login/join/keepalive control frames are sent; no message/account API.
//! The caller owns the archive file, size limit, synchronization and lifecycle.
use reqwest::{blocking::Client, redirect::Policy};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    io::{ErrorKind, Read},
    net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tungstenite::{
    client::IntoClientRequest, client_tls_with_config, protocol::WebSocketConfig, Message,
};
#[path = "chat_gift.rs"]
pub(crate) mod gifts;
#[path="chat_mission.rs"]
pub(crate) mod missions;
#[path = "chat_ogq.rs"]
pub(crate) mod ogq;
#[path = "chat_subscription.rs"]
pub(crate) mod subscriptions;
#[path = "chat_personalcon.rs"]
pub(crate) mod personalcon;
#[path="chat_portrait.rs"]
pub(crate) mod portrait;

const MAX_FRAME: usize = 1024 * 1024;
const METADATA_LIMIT: u64 = 512 * 1024;
const ZERO_ID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatEvent {
    #[serde(skip)]
    pub received_monotonic: Instant,
    pub elapsed_ms: u64,
    pub received_at_ms: u64,
    pub message_id: Option<String>,
    pub kind: &'static str,
    pub nickname: String,
    pub text: String,
    pub flags: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
    #[serde(skip)]
    pub gift_asset: Option<gifts::assets::Request>,
    #[serde(skip)]
    pub mission_asset: Option<missions::Request>,
    #[serde(skip)]
    pub emoticon_assets: Vec<ogq::Request>,
    #[serde(skip)]
    pub subscription_assets: Vec<subscriptions::Request>,
    #[serde(skip)]
    pub personalcon_asset: Option<personalcon::Request>,
    #[serde(skip)]
    pub avatar_asset: Option<portrait::Request>,
    #[serde(skip)]
    presentation_partial: bool,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatSummary {
    pub messages: u64,
    pub gaps: u32,
    pub connections: u32,
    pub stopped: bool,
    pub broadcast_ended: bool,
    pub unsupported_rich_events: u64,
    pub malformed_rich_events: u64,
}
enum Failure {
    Network,
    Protocol,
    Restricted,
    Ended,
    Identity,
    Sink,
}
impl Failure {
    fn diagnostic_reason(&self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Protocol => "protocol",
            Self::Restricted => "restricted",
            Self::Ended => "ended",
            Self::Identity => "identity",
            Self::Sink => "sink",
        }
    }
    fn message(&self) -> &'static str {
        match self {
            Self::Network => "SOOP 채팅에 연결하지 못했습니다. 영상 저장은 계속됩니다.",
            Self::Protocol => "SOOP 채팅 형식이 변경됐습니다. 영상 저장은 계속됩니다.",
            Self::Restricted => "익명으로 읽을 수 없는 SOOP 채팅입니다.",
            Self::Ended => "SOOP 방송이 종료되었거나 다른 방송으로 변경됐습니다.",
            Self::Identity => {
                "SOOP 채팅의 방송이 녹화 중인 방송과 다릅니다. 기존 기록은 보존합니다."
            }
            Self::Sink => "SOOP 채팅 기록을 더 저장하지 못했습니다. 기존 기록은 보존합니다.",
        }
    }
}
struct Endpoint {
    host: String,
    address: SocketAddr,
    room: String,
    ticket: String,
    broadcast: String,
    subscription_labels: [Option<String>; 2],
    subscription_labels_partial: bool,
    personalcon: personalcon::Context,
    gift_assets: gifts::assets::Context,
}
fn integer(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}
fn numeric(value: &Value) -> Option<String> {
    let n = integer(value)?;
    (n > 0).then(|| n.to_string())
}
fn public_ip(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !matches!(a, 0 | 10 | 127 | 224..=255)
        && !(a == 100 && (64..=127).contains(&b))
        && !(a == 169 && b == 254)
        && !(a == 172 && (16..=31).contains(&b))
        && !(a == 192 && (b == 168 || b == 0))
        && !(a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        && !(a == 203 && b == 0 && c == 113)
}
fn endpoint_from_value(channel: &str, value: &Value) -> Result<Endpoint, Failure> {
    let row = &value["CHANNEL"];
    let result = integer(&row["RESULT"]).ok_or(Failure::Protocol)?;
    if result == 0 || result == -2 {
        return Err(Failure::Ended);
    }
    if result != 1
        || row["BPWD"] != "N"
        || integer(&row["GRADE"]) != Some(0)
        || integer(&row["P_MIN_TIER"]) != Some(0)
        || row["BSTATUS"] != "BROADING"
    {
        return Err(Failure::Restricted);
    }
    if !row["BJID"]
        .as_str()
        .is_some_and(|id| id.eq_ignore_ascii_case(channel))
    {
        return Err(Failure::Protocol);
    }
    let host = row["CHDOMAIN"]
        .as_str()
        .ok_or(Failure::Protocol)?
        .to_ascii_lowercase();
    let encoded = host
        .strip_prefix("chat-")
        .and_then(|h| h.strip_suffix(".sooplive.com"))
        .ok_or(Failure::Protocol)?;
    if encoded.len() != 8 || !encoded.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Failure::Protocol);
    }
    // The official HTTPS player adds one to the server-issued clear port.
    // Current servers use both 8040 and 9000; never pin all rooms to 9001.
    let port = secure_chat_port(&row["CHPT"])?;
    // Official CHDOMAIN encodes the IPv4 address. Pin the transport to that
    // validated public address; do not expose arbitrary DNS/URL input or SSRF.
    let ip = Ipv4Addr::from(u32::from_str_radix(encoded, 16).map_err(|_| Failure::Protocol)?);
    if !public_ip(ip) {
        return Err(Failure::Protocol);
    }
    let ticket = row["FTK"].as_str().ok_or(Failure::Protocol)?;
    if ticket.is_empty() || ticket.len() > 4096 || ticket.chars().any(char::is_control) {
        return Err(Failure::Protocol);
    }
    Ok(Endpoint {
        host,
        address: SocketAddr::from((ip, port)),
        room: numeric(&row["CHATNO"]).ok_or(Failure::Protocol)?,
        ticket: ticket.into(),
        broadcast: numeric(&row["BNO"]).ok_or(Failure::Protocol)?,
        personalcon: personalcon::Context::from_channel(row),
        gift_assets: gifts::assets::Context::new(),
        subscription_labels: ["TIER1_NICK", "TIER2_NICK"].map(|key|row[key].as_str().and_then(subscriptions::label)),
        subscription_labels_partial: ["TIER1_NICK", "TIER2_NICK"].iter().any(|key|!row[*key].is_null()
            && row[*key].as_str().is_none_or(|value|!value.is_empty() && subscriptions::label(value).is_none())),
    })
}
fn secure_chat_port(value: &Value) -> Result<u16, Failure> {
    let port = integer(value).ok_or(Failure::Protocol)?;
    if !(1024..=65534).contains(&port) {
        return Err(Failure::Protocol);
    }
    Ok((port + 1) as u16)
}
fn endpoint(channel: &str) -> Result<Endpoint, Failure> {
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .user_agent("Mozilla/5.0 (compatible; Atsumi/2; public-chat-reader)")
        .build()
        .map_err(|_| Failure::Network)?;
    let response = client
        .post("https://live.sooplive.com/afreeca/player_live_api.php")
        .header("Referer", format!("https://play.sooplive.com/{channel}"))
        .form(&[
            ("bid", channel),
            ("type", "live"),
            ("pwd", ""),
            ("player_type", "html5"),
            ("stream_type", "common"),
            ("quality", "HD"),
            ("mode", "landing"),
        ])
        .send()
        .map_err(|_| Failure::Network)?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(Failure::Restricted);
    }
    if !response.status().is_success() {
        return Err(Failure::Network);
    }
    if response
        .content_length()
        .is_some_and(|n| n > METADATA_LIMIT)
    {
        return Err(Failure::Protocol);
    }
    let mut bytes = Vec::new();
    response
        .take(METADATA_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Network)?;
    if bytes.len() as u64 > METADATA_LIMIT {
        return Err(Failure::Protocol);
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| Failure::Protocol)?;
    endpoint_from_value(channel, &value)
}
fn packet(service: u16, fields: &[&str]) -> Result<Vec<u8>, Failure> {
    if !matches!(service, 0 | 1 | 2) || fields.iter().any(|s| s.contains('\x0c')) {
        return Err(Failure::Protocol);
    }
    let body = if service == 0 {
        "\x0c".into()
    } else {
        format!("\x0c{}\x0c", fields.join("\x0c"))
    };
    if body.len() > 8192 {
        return Err(Failure::Protocol);
    }
    Ok(format!("\x1d\x09{service:04}{:06}00{ZERO_ID}{body}", body.len()).into_bytes())
}
struct Frame<'a> {
    service: u16,
    result: u16,
    id: Option<&'a str>,
    body: &'a str,
}
fn parse(bytes: &[u8]) -> Result<Frame<'_>, Failure> {
    let header = match bytes.first() {
        Some(29) => 50,
        Some(27) => 14,
        _ => return Err(Failure::Protocol),
    };
    if bytes.len() < header || bytes.len() > MAX_FRAME || bytes[1] != 9 {
        return Err(Failure::Protocol);
    }
    let digits = |part: &[u8]| -> Result<u32, Failure> {
        if !part.iter().all(u8::is_ascii_digit) {
            return Err(Failure::Protocol);
        }
        std::str::from_utf8(part)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or(Failure::Protocol)
    };
    if digits(&bytes[6..12])? as usize != bytes.len() - header {
        return Err(Failure::Protocol);
    }
    let id = if header == 50 {
        let s = std::str::from_utf8(&bytes[14..50]).map_err(|_| Failure::Protocol)?;
        uuid::Uuid::parse_str(s).map_err(|_| Failure::Protocol)?;
        (s != ZERO_ID).then_some(s)
    } else {
        None
    };
    let body = std::str::from_utf8(&bytes[header..]).map_err(|_| Failure::Protocol)?;
    if !body.starts_with('\x0c') || !body.ends_with('\x0c') {
        return Err(Failure::Protocol);
    }
    Ok(Frame {
        service: digits(&bytes[2..6])? as u16,
        result: digits(&bytes[12..14])? as u16,
        id,
        body,
    })
}
fn event(started: Instant, kind: &'static str, text: &str) -> ChatEvent {
    ChatEvent {
        received_monotonic: Instant::now(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        received_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        message_id: None,
        kind,
        nickname: String::new(),
        text: text.into(),
        flags: String::new(),
        extra: None,
        gift_asset: None,
        mission_asset: None,
        emoticon_assets: Vec::new(),
        subscription_assets: Vec::new(),
        personalcon_asset: None,
        avatar_asset: None,
        presentation_partial: false,
    }
}
fn chat_event(frame: &Frame<'_>, started: Instant) -> Result<Option<ChatEvent>, Failure> {
    chat_event_bound(frame, started, None)
}
fn chat_event_bound(frame: &Frame<'_>, started: Instant, channel: Option<&str>) -> Result<Option<ChatEvent>, Failure> {
    chat_event_context(frame, started, channel, None)
}
fn chat_event_context(frame: &Frame<'_>, started: Instant, channel: Option<&str>, labels: Option<(&[Option<String>;2],bool)>) -> Result<Option<ChatEvent>, Failure> {
    // Never ingest whispers (9), user lists, tickets or complete raw payloads.
    // Public gifts use only their independently verified display-field schema.
    if !matches!(frame.service, 5 | 18 | 33 | 91 | 93 | 105 | 109 | 121) {
        return Ok(None);
    }
    if frame.body.len() < 2 {
        return Err(Failure::Protocol);
    }
    let fields: Vec<_> = frame.body[1..frame.body.len() - 1].split('\x0c').collect();
    if frame.service == 121 {
        let public=missions::parse(&fields).map_err(|_|Failure::Protocol)?;
        let mut row=event(started,public.metadata.row_kind(),&public.metadata.text());
        row.message_id=frame.id.map(str::to_owned);
        row.nickname=public.nickname;
        row.presentation_partial=public.partial;
        row.extra=Some(serde_json::json!({"service":121,"soopMission":public.metadata}));
        return Ok(Some(row));
    }
    if matches!(frame.service,91|93) {
        let public=subscriptions::parse(frame.service,&fields,labels.map(|value|value.0)).map_err(|_|Failure::Protocol)?;
        let mut row=event(started,"membership","");
        row.message_id=frame.id.map(str::to_owned);
        row.nickname=public.nickname.into();
        row.presentation_partial=public.partial || labels.is_some_and(|value|value.1);
        row.subscription_assets=public.requests;
        row.extra=Some(serde_json::json!({"service":frame.service,"soopSubscription":public.metadata}));
        return Ok(Some(row));
    }
    if frame.service == 109 {
        let Some(emote)=ogq::parse(&fields).map_err(|_|Failure::Protocol)? else{return Ok(None);};
        let mut row=event(started,"chat",emote.text);
        row.message_id=frame.id.map(str::to_owned);row.nickname=emote.nickname.into();row.flags=emote.flags.into();
        row.presentation_partial=emote.partial;row.emoticon_assets=emote.requests;
        let bounded=|index:usize|fields.get(index).copied().filter(|s|s.len()<=128).unwrap_or("");
        row.extra=Some(serde_json::json!({"service":109,"soopEmoticon":emote.metadata,
            "subscriptionMonth":bounded(12),"accSubscriptionMonth":bounded(15),"color":bounded(8),
            "nicknameColorKey":ogq::nickname_key(&Value::String(bounded(13).into())),"repPersonalconMonth":bounded(16)}));
        return Ok(Some(row));
    }
    if frame.service != 5 {
        let gift = gifts::parse(frame.service, &fields).map_err(|_| Failure::Protocol)?;
        let Some(gift) = gift else { return Ok(None); };
        let mut row = event(started, "donation", "");
        row.message_id = frame.id.map(str::to_owned);
        row.nickname = gift.nickname.into();
        row.gift_asset = channel.and_then(|channel| gifts::assets::Request::from_fields(frame.service, &fields, channel));
        row.extra = Some(serde_json::json!({
            "service": frame.service, "soopGift": gift.metadata, "fileName": gift.file_name
        }));
        if matches!(frame.service,18|33) {
            let (language,modified)=if frame.service==18 {(12,13)} else {(13,14)};
            let extra=row.extra.as_mut().unwrap().as_object_mut().unwrap();
            if let Some(value)=fields.get(language).filter(|value|value.len()<=5 && value.bytes().all(|b|b.is_ascii_alphabetic() || b==b'_')) {
                extra.insert("senderLanguage".into(),Value::String((*value).into()));
            }
            if let Some(value)=fields.get(modified).filter(|value|value.len()<=20 && value.bytes().all(|b|b.is_ascii_digit())) {
                extra.insert("urlModify".into(),Value::String((*value).into()));
            }
        }
        return Ok(Some(row));
    }
    if !(7..=32).contains(&fields.len())
        || fields[0].len() > 16 * 1024
        || fields[5].len() > 512
        || fields[6].len() > 128
        || [2, 7, 10]
            .iter()
            .any(|index| fields.get(*index).is_some_and(|value| value.len() > 128))
    {
        return Err(Failure::Protocol);
    }
    let mut row = event(started, "chat", fields[0]);
    row.message_id = frame.id.map(str::to_owned);
    row.nickname = fields[5].into();
    row.flags = fields[6].into();
    // Ordinary chat uses the same exact palette as OGQ. Packet[9] is not
    // a separate replay-theme color; never retain an arbitrary class or ID.
    let nickname_color_key = fields
        .get(8)
        .filter(|key| key.len() == 6)
        .and_then(|key| ogq::nickname_key(&Value::String((*key).into())));
    row.extra = Some(
        serde_json::json!({"service":5,"subscriptionMonth":fields.get(7).copied().unwrap_or(""),"accSubscriptionMonth":fields.get(10).copied().unwrap_or(""),"color":fields.get(2).copied().unwrap_or(""),"nicknameColorKey":nickname_color_key,"repPersonalconMonth":fields.get(11).filter(|s|s.len()<=128).copied().unwrap_or(""),"messageType":match fields[3].parse::<f64>().ok(){Some(1.0)=>Some("staff"),Some(2.0)=>Some("police"),_=>None}}),
    );
    Ok(Some(row))
}
struct ConnectionGuard(Arc<AtomicBool>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
#[derive(Default)]
struct Seen {
    ids: HashSet<String>,
    order: VecDeque<String>,
    rich_warning_emitted: bool,
}
impl Seen {
    fn accept(&mut self, id: Option<&str>) -> bool {
        let Some(id) = id else {
            return true;
        };
        if self.ids.contains(id) {
            return false;
        }
        while self.order.len() >= 4096 {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
        self.ids.insert(id.into());
        self.order.push_back(id.into());
        true
    }
}
fn receive_public(
    frame: &Frame<'_>,
    started: Instant,
    sink: &mut impl FnMut(ChatEvent) -> Result<(), String>,
    summary: &mut ChatSummary,
    seen: &mut Seen,
) -> Result<(), Failure> {
    receive_public_bound(frame, started, sink, summary, seen, None)
}
fn receive_public_bound(
    frame: &Frame<'_>, started: Instant, sink: &mut impl FnMut(ChatEvent)->Result<(),String>,
    summary: &mut ChatSummary, seen: &mut Seen, channel: Option<&str>,
) -> Result<(), Failure> {
    receive_public_context(frame,started,sink,summary,seen,channel,None)
}
fn receive_public_context(
    frame: &Frame<'_>, started: Instant, sink: &mut impl FnMut(ChatEvent)->Result<(),String>,
    summary: &mut ChatSummary, seen: &mut Seen, channel: Option<&str>, labels:Option<(&[Option<String>;2],bool)>,
) -> Result<(), Failure> {
    receive_public_appearance(frame,started,sink,summary,seen,channel,labels,None)
}
fn receive_public_appearance(
    frame:&Frame<'_>, started:Instant, sink:&mut impl FnMut(ChatEvent)->Result<(),String>,
    summary:&mut ChatSummary, seen:&mut Seen, channel:Option<&str>, labels:Option<(&[Option<String>;2],bool)>, appearance:Option<&personalcon::Context>,
)->Result<(),Failure>{
    receive_public_assets(frame, started, sink, summary, seen, channel, labels, appearance, None)
}
fn receive_public_assets(
    frame:&Frame<'_>, started:Instant, sink:&mut impl FnMut(ChatEvent)->Result<(),String>,
    summary:&mut ChatSummary, seen:&mut Seen, channel:Option<&str>, labels:Option<(&[Option<String>;2],bool)>,
    appearance:Option<&personalcon::Context>, gift_assets:Option<&gifts::assets::Context>,
)->Result<(),Failure>{
    let unsupported = gifts::known_unsupported(frame.service);
    let mut parsed = if unsupported { Ok(None) } else { chat_event_context(frame, started, channel, labels) };
    if let (Ok(Some(row)), Some(context), Some(channel)) = (&mut parsed, gift_assets, channel) {
        if frame.service == 121 {
            let fields:Vec<_>=frame.body[1..frame.body.len()-1].split('\x0c').collect();
            if let Some(request)=context.mission(&fields) {
                let extra=row.extra.as_mut().ok_or(Failure::Protocol)?;
                let mut meta=missions::from_stored(&extra["soopMission"]).ok_or(Failure::Protocol)?;
                if request.apply(&mut meta) {
                    extra["soopMission"]=serde_json::to_value(meta).map_err(|_|Failure::Protocol)?;
                    row.mission_asset=Some(request);
                }
            }
        }
        if row.kind == "donation" && frame.service != 121 {
            let fields: Vec<_> = frame.body[1..frame.body.len()-1].split('\x0c').collect();
            if let Some(request) = context.from_fields(frame.service, &fields, channel) {
                let extra = row.extra.as_mut().ok_or(Failure::Protocol)?;
                let mut gift = gifts::from_stored(&extra["soopGift"]).ok_or(Failure::Protocol)?;
                if request.apply(&mut gift) {
                    // Save the expected opaque ID before raw serialization; the
                    // capability itself never crosses the serialized boundary.
                    extra["soopGift"] = serde_json::to_value(gift).map_err(|_| Failure::Protocol)?;
                    row.gift_asset = Some(request);
                }
            }
        }
    }
    if let (Ok(Some(row)),Some(context))=(&mut parsed,appearance) {
        if row.kind=="chat" && row.extra.as_ref().is_some_and(|v|v["messageType"].is_null()) {
            let extra=row.extra.as_mut().unwrap();
            let (request,partial)=context.select(&row.flags,extra["accSubscriptionMonth"].as_str().unwrap_or(""),extra["repPersonalconMonth"].as_str().unwrap_or(""));
            if let Some(request)=channel.and_then(|c|context.portrait(c,frame.body[1..frame.body.len()-1].split('\x0c').nth(if frame.service==109{5}else{1}).unwrap_or(""),&row.flags,extra["repPersonalconMonth"].as_str().unwrap_or(""))){
                extra["avatar"]=serde_json::to_value(request.metadata()).map_err(|_|Failure::Protocol)?;row.avatar_asset=Some(request);
            }
            row.presentation_partial |= partial;
            if let Some(request)=request {extra["personalcon"]=serde_json::to_value(request.metadata()).map_err(|_|Failure::Protocol)?;row.personalcon_asset=Some(request);}
        }
    }
    let malformed_gift = matches!(frame.service, 18 | 33 | 91 | 93 | 105 | 109 | 121) && parsed.is_err();
    let partial_emote = matches!(frame.service,5|91|93|109|121) && parsed.as_ref().is_ok_and(|row|row.as_ref().is_none_or(|row|row.presentation_partial));
    if unsupported || malformed_gift || partial_emote {
        if unsupported || partial_emote { summary.unsupported_rich_events += 1; }
        else { summary.malformed_rich_events += 1; }
        // One bounded marker for the capture, not a row for every unsupported
        // event. It marks archive coverage without pretending the socket died.
        if !seen.rich_warning_emitted {
            seen.rich_warning_emitted = true;
            let mut row = event(started, "gap", gifts::COVERAGE_WARNING);
            row.extra = Some(serde_json::json!({
                "service": frame.service, "coverageWarning": "unsupportedSoopRichEvent",
                "reason": if unsupported || partial_emote { "unsupported" } else { "malformed" }
            }));
            sink(row).map_err(|_| Failure::Sink)?;
        }
        if unsupported || malformed_gift || parsed.as_ref().is_ok_and(Option::is_none) {return Ok(());}
    }
    if let Some(row) = parsed? {
        if seen.accept(row.message_id.as_deref()) {
            sink(row).map_err(|_| Failure::Sink)?;
            summary.messages += 1;
        }
    }
    Ok(())
}
fn connect_read(
    channel: &str,
    endpoint: &Endpoint,
    started: Instant,
    stop: Arc<AtomicBool>,
    sink: &mut impl FnMut(ChatEvent) -> Result<(), String>,
    summary: &mut ChatSummary,
    seen: &mut Seen,
) -> Result<(), Failure> {
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let stream = TcpStream::connect_timeout(&endpoint.address, Duration::from_secs(2))
        .map_err(|_| Failure::Network)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| Failure::Network)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| Failure::Network)?;
    let watchdog_socket = stream.try_clone().map_err(|_| Failure::Network)?;
    let done = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let _guard = ConnectionGuard(done.clone());
    let watch_stop = stop.clone();
    let watch_ready = ready.clone();
    thread::spawn(move || {
        let at = Instant::now();
        while !done.load(Ordering::Acquire) {
            if watch_stop.load(Ordering::Acquire)
                || (!watch_ready.load(Ordering::Acquire) && at.elapsed() > Duration::from_secs(4))
            {
                let _ = watchdog_socket.shutdown(Shutdown::Both);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
    let mut request = format!(
        "wss://{}:{}/Websocket/{channel}",
        endpoint.host,
        endpoint.address.port()
    )
    .into_client_request()
    .map_err(|_| Failure::Protocol)?;
    request.headers_mut().insert(
        "Origin",
        "https://play.sooplive.com"
            .parse()
            .map_err(|_| Failure::Protocol)?,
    );
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "chat".parse().map_err(|_| Failure::Protocol)?,
    );
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_FRAME);
    config.max_frame_size = Some(MAX_FRAME);
    config.write_buffer_size = 0;
    config.max_write_buffer_size = 16384;
    let (mut socket, response) = client_tls_with_config(request, stream, Some(config), None)
        .map_err(|_| Failure::Network)?;
    if response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        != Some("chat")
    {
        return Err(Failure::Protocol);
    }
    ready.store(true, Ordering::Release);
    socket
        .send(Message::Binary(packet(1, &["", "", "16"])?.into()))
        .map_err(|_| Failure::Network)?;
    let joining = Instant::now();
    let mut joined = false;
    let mut login_seen = false;
    let mut last_ping = Instant::now();
    while !stop.load(Ordering::Acquire) {
        if !joined && joining.elapsed() > Duration::from_secs(8) {
            return Err(Failure::Network);
        }
        if last_ping.elapsed() > Duration::from_secs(30) {
            socket
                .send(Message::Binary(packet(0, &[])?.into()))
                .map_err(|_| Failure::Network)?;
            last_ping = Instant::now();
        }
        let message = match socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue
            }
            Err(_) => return Err(Failure::Network),
        };
        match message {
            Message::Binary(bytes) => {
                let frame = parse(&bytes)?;
                #[cfg(test)]
                if !gift_probe::observe(&frame, bytes.len()) {
                    stop.store(true, Ordering::Release);
                    break;
                }
                if frame.result != 0 {
                    return Err(Failure::Restricted);
                }
                match frame.service {
                    1 => {
                        if login_seen {
                            return Err(Failure::Protocol);
                        }
                        login_seen = true;
                        socket
                            .send(Message::Binary(
                                packet(
                                    2,
                                    &[
                                        &endpoint.room,
                                        &endpoint.ticket,
                                        "0",
                                        "",
                                        "pver\x112\x12access_system\x11html5\x12",
                                    ],
                                )?
                                .into(),
                            ))
                            .map_err(|_| Failure::Network)?;
                    }
                    2 => {
                        if !login_seen {
                            return Err(Failure::Protocol);
                        }
                        if !joined {
                            joined = true;
                            summary.connections += 1;
                            sink(event(started, "connected", "채팅 연결됨"))
                                .map_err(|_| Failure::Sink)?;
                        }
                    }
                    3 | 88 => return Err(Failure::Ended),
                    service if joined && (service == 5 || gifts::is_public_rich(service)) => {
                        receive_public_assets(&frame, started, sink, summary, seen, Some(channel),Some((&endpoint.subscription_labels,endpoint.subscription_labels_partial)),Some(&endpoint.personalcon),Some(&endpoint.gift_assets))?;
                    }
                    _ => {}
                }
            }
            Message::Ping(_) => socket.flush().map_err(|_| Failure::Network)?,
            Message::Close(_) => return Err(Failure::Network),
            _ => {}
        }
    }
    let _ = socket.close(None);
    Ok(())
}

#[cfg(test)]
#[path = "chat_gift_probe.rs"]
mod gift_probe;
/// Dedicated blocking worker. `sink` must be bounded and write only the caller's
/// owned archive; its failure stops capture immediately without retrying disk I/O.
/// elapsedMs is relative to this call, before metadata/handshake; receivedAtMs
/// permits the common recorder to align this separate metadata stream to video.
pub(crate) fn capture(
    channel: &str,
    sink: impl FnMut(ChatEvent) -> Result<(), String>,
    stop: Arc<AtomicBool>,
) -> Result<ChatSummary, String> {
    capture_bound(channel, None, sink, stop)
}
fn valid_broadcast(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 20
        && id.bytes().all(|b| b.is_ascii_digit())
        && id
            .parse::<u64>()
            .is_ok_and(|n| n > 0 && n.to_string() == id)
}
fn bind_broadcast(current: &mut Option<String>, actual: &str, pinned: bool) -> Result<(), Failure> {
    if current.as_ref().is_some_and(|value| value != actual) {
        return Err(if pinned {
            Failure::Identity
        } else {
            Failure::Ended
        });
    }
    *current = Some(actual.to_owned());
    Ok(())
}
/// The official player already knows its broadcast. Never attach that archive
/// to a newer broadcast returned by a channel-only metadata race.
pub(crate) fn capture_bound(
    channel: &str,
    expected_broadcast: Option<&str>,
    mut sink: impl FnMut(ChatEvent) -> Result<(), String>,
    stop: Arc<AtomicBool>,
) -> Result<ChatSummary, String> {
    if !super::valid_id(channel) {
        return Err("SOOP 채널 ID를 확인해 주세요.".into());
    }
    if expected_broadcast.is_some_and(|value| !valid_broadcast(value)) {
        return Err(Failure::Identity.message().into());
    }
    let started = Instant::now();
    let mut summary = ChatSummary::default();
    let mut seen = Seen::default();
    let mut broadcast = expected_broadcast.map(str::to_owned);
    let mut failures = 0;
    while !stop.load(Ordering::Acquire) {
        let attempt_started = Instant::now();
        let prior_connections = summary.connections;
        let result = endpoint(channel).and_then(|remote| {
            bind_broadcast(
                &mut broadcast,
                &remote.broadcast,
                expected_broadcast.is_some(),
            )?;
            connect_read(
                channel,
                &remote,
                started,
                stop.clone(),
                &mut sink,
                &mut summary,
                &mut seen,
            )
        });
        if stop.load(Ordering::Acquire) {
            break;
        }
        match result {
            Ok(()) => break,
            Err(Failure::Ended) => {
                summary.broadcast_ended = true;
                sink(event(started, "ended", "방송 종료"))
                    .map_err(|_| Failure::Sink.message().to_string())?;
                break;
            }
            Err(Failure::Network) => {
                if summary.connections > prior_connections
                    && attempt_started.elapsed() > Duration::from_secs(30)
                {
                    failures = 0;
                }
                failures += 1;
                summary.gaps += 1;
                sink(event(
                    started,
                    "gap",
                    "연결이 끊긴 구간의 채팅은 누락될 수 있습니다.",
                ))
                .map_err(|_| Failure::Sink.message().to_string())?;
                tracing::warn!(
                    diag_stage = "soop_chat_gap",
                    reason = "network",
                    attempt = failures as u64
                );
                if failures >= 8 {
                    return Err(Failure::Network.message().into());
                }
                let delay = Duration::from_secs((1u64 << failures.min(4)).min(16));
                let waiting = Instant::now();
                while !stop.load(Ordering::Acquire) && waiting.elapsed() < delay {
                    thread::sleep(Duration::from_millis(50));
                }
            }
            Err(error) => {
                tracing::warn!(
                    diag_stage = "soop_chat_stopped",
                    reason = error.diagnostic_reason(),
                    success = false
                );
                return Err(error.message().into());
            }
        }
    }
    summary.stopped = stop.load(Ordering::Acquire);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_broadcast_binding_rejects_first_and_later_other_broadcast() {
        let mut bound = Some("123".to_string());
        assert!(bind_broadcast(&mut bound, "123", true).is_ok());
        assert!(matches!(
            bind_broadcast(&mut bound, "124", true),
            Err(Failure::Identity)
        ));
        assert_eq!(bound.as_deref(), Some("123"));
        let mut legacy = None;
        assert!(bind_broadcast(&mut legacy, "124", false).is_ok());
        assert!(matches!(
            bind_broadcast(&mut legacy, "125", false),
            Err(Failure::Ended)
        ));
        for invalid in ["", "0", "0123", "123a", "18446744073709551616"] {
            assert!(capture_bound(
                "fixture",
                Some(invalid),
                |_| panic!("must not connect"),
                Arc::new(AtomicBool::new(true))
            )
            .is_err());
        }
        let stopped = capture_bound(
            "fixture",
            Some("123"),
            |_| panic!("must not connect"),
            Arc::new(AtomicBool::new(true)),
        )
        .unwrap();
        assert!(stopped.stopped);
        assert_eq!(stopped.connections, 0);
    }
    fn metadata() -> Value {
        serde_json::json!({"CHANNEL":{"RESULT":1,"BPWD":"N","GRADE":0,"P_MIN_TIER":0,"BSTATUS":"BROADING","BJID":"fixture","CHDOMAIN":"chat-76DBFCC2.sooplive.com","CHPT":9000,"CHATNO":8709,"FTK":"anonymous-fixture-ticket","BNO":123}})
    }
    fn frame(service: u16, fields: &[&str]) -> Vec<u8> {
        let body = format!("\x0c{}\x0c", fields.join("\x0c"));
        format!("\x1d\x09{service:04}{:06}00{ZERO_ID}{body}", body.len()).into_bytes()
    }
    #[test]
    fn server_issued_public_chat_port_matches_official_https_plus_one_rule() {
        for clear in [8040, 9000, 65534] {
            for value in [
                serde_json::json!(clear),
                serde_json::json!(clear.to_string()),
            ] {
                let mut row = metadata();
                row["CHANNEL"]["CHPT"] = value;
                assert_eq!(
                    endpoint_from_value("fixture", &row)
                        .ok()
                        .unwrap()
                        .address
                        .port(),
                    clear as u16 + 1
                );
            }
        }
        for invalid in [
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!(443),
            serde_json::json!(1023),
            serde_json::json!(65535),
            serde_json::json!(8040.5),
            serde_json::json!("8040x"),
            serde_json::json!("18446744073709551615"),
            serde_json::Value::Null,
        ] {
            assert!(secure_chat_port(&invalid).is_err());
        }
    }
    #[test]
    fn restrict_metadata_and_transport() {
        let value = metadata();
        let endpoint = endpoint_from_value("fixture", &value).ok().unwrap();
        assert_eq!(endpoint.address.ip().to_string(), "118.219.252.194");
        for (key, bad) in [
            ("GRADE", serde_json::json!(19)),
            ("BPWD", serde_json::json!("Y")),
            ("P_MIN_TIER", serde_json::json!(1)),
            ("CHDOMAIN", serde_json::json!("chat-7f000001.sooplive.com")),
            (
                "CHDOMAIN",
                serde_json::json!("chat-76dbfcc2.sooplive.com.evil.test"),
            ),
            ("CHPT", serde_json::json!(80)),
            ("BJID", serde_json::json!("someone")),
        ] {
            let mut changed = value.clone();
            changed["CHANNEL"][key] = bad;
            assert!(endpoint_from_value("fixture", &changed).is_err());
        }
    }
    #[test]
    fn packets_are_control_only_and_utf8_lengths_exact() {
        let bytes = packet(1, &["", "", "16"]).ok().unwrap();
        let decoded = parse(&bytes).ok().unwrap();
        assert_eq!(decoded.service, 1);
        assert_eq!(decoded.body, "\x0c\x0c\x0c16\x0c");
        assert!(packet(5, &["must never send chat"]).is_err());
        assert!(packet(2, &["bad\x0cfield"]).is_err());
        let keep = packet(0, &[]).ok().unwrap();
        assert_eq!(keep.len(), 51);
    }
    #[test]
    fn korean_chat_is_text_and_private_packets_are_ignored() {
        let bytes = frame(
            5,
            &[
                "안녕하세요 <script>",
                "unused-account-id",
                "0",
                "0",
                "0",
                "시험 닉네임",
                "16",
                "3",
                "",
                "",
                "7",
            ],
        );
        let raw = parse(&bytes).ok().unwrap();
        let row = chat_event(&raw, Instant::now()).ok().flatten().unwrap();
        assert_eq!(row.text, "안녕하세요 <script>");
        assert_eq!(row.nickname, "시험 닉네임");
        let encoded = serde_json::to_string(&row).unwrap();
        assert!(!encoded.contains("unused-account-id"));
        let direct = frame(9, &["private fixture"]);
        assert!(chat_event(&parse(&direct).ok().unwrap(), Instant::now())
            .ok()
            .flatten()
            .is_none());
    }
    fn service5_palette_fixture(key: Option<&str>, message_type: &str) -> ChatEvent {
        let mut fields = vec!["한글 <script>", "sender-private", "255", message_type, "0", "Public viewer", "32|262144", "7", key.unwrap_or(""), "C40F70", "9"];
        if key.is_none() { fields.truncate(8); }
        let bytes = frame(5, &fields);
        chat_event(&parse(&bytes).ok().unwrap(), Instant::now()).ok().flatten().unwrap()
    }
    #[test]
    fn ordinary_chat_captures_only_exact_official_nickname_palette() {
        for key in ["E12E2E", "A90A0A", "EB500D", "C25111", "E07E0A", "B26C03", "648E0B", "119937", "158304", "047143", "0799B9", "0B6D82", "0C80D3", "0C61E1", "1915BF", "6518C8", "AD19E1", "DD32E0", "DC3585", "C40F70"] {
            let row = service5_palette_fixture(Some(key), "0");
            let value = serde_json::to_value(&row).unwrap();
            assert_eq!(value["extra"]["nicknameColorKey"], key);
            assert_eq!(row.text, "한글 <script>");
            assert_eq!(row.nickname, "Public viewer");
            assert_eq!(row.flags, "32|262144");
            assert_eq!(value["extra"]["color"], "255");
            assert_eq!(value["extra"]["subscriptionMonth"], "7");
            assert_eq!(value["extra"]["accSubscriptionMonth"], "9");
            assert!(!value.to_string().contains("sender-private"));
            assert!(row.gift_asset.is_none() && row.emoticon_assets.is_empty());
        }
    }
    #[test]
    fn invalid_or_absent_nickname_palette_keeps_public_text_without_using_dark_field() {
        let oversized = "E".repeat(1024);
        for key in [None, Some(""), Some("e12e2e"), Some("#E12E2E"), Some("FFFFFF"), Some("random-color1 injected"), Some(oversized.as_str())] {
            let row = service5_palette_fixture(key, "0");
            let value = serde_json::to_value(&row).unwrap();
            assert!(value["extra"]["nicknameColorKey"].is_null());
            assert_eq!(row.kind, "chat");
            assert_eq!(row.text, "한글 <script>");
            assert_eq!(row.nickname, "Public viewer");
            let serialized = value.to_string();
            for excluded in ["sender-private", "C40F70", "randomNicknameColorDarkmode", "random-color1 injected"] { assert!(!serialized.contains(excluded)); }
        }
    }
    #[test]
    fn nickname_palette_capture_does_not_filter_existing_service5_message_types() {
        for message_type in ["0", "1", "2", "3", "4", "", "unrecognized"] {
            let row = service5_palette_fixture(Some("E12E2E"), message_type);
            assert_eq!(row.kind, "chat");
            assert_eq!(row.text, "한글 <script>");
            assert_eq!(row.extra.as_ref().unwrap()["nicknameColorKey"], "E12E2E");
        }
    }
    #[test]
    fn public_gift_roundtrip_keeps_display_data_and_deduplicates() {
        let bytes = frame(18, &["receiver-secret", "sender-secret", "Public viewer", "100", "0", "", "", "", "0", "0"]);
        let mut decoded = parse(&bytes).ok().unwrap();
        decoded.id = Some("fixture-event");
        let mut rows = Vec::new();
        let mut summary = ChatSummary::default();
        let mut seen = Seen::default();
        for _ in 0..2 {
            receive_public(&decoded, Instant::now(), &mut |row| { rows.push(row); Ok(()) }, &mut summary, &mut seen).ok().unwrap();
        }
        assert_eq!(rows.len(), 1);
        assert_eq!(summary.messages, 1);
        assert_eq!(rows[0].kind, "donation");
        assert!(rows[0].text.is_empty() && rows[0].flags.is_empty());
        let encoded = serde_json::to_string(&rows[0]).unwrap();
        assert!(!encoded.contains("receiver-secret") && !encoded.contains("sender-secret"));
        assert!(gifts::from_raw(&serde_json::to_value(&rows[0]).unwrap()).is_some());
    }
    #[test]
    fn custom_gift_request_is_bound_transient_and_malformed_selectors_preserve_donation() {
        for file in ["50","../invalid"] {
            let bytes=frame(18,&["fixture(1)","sender-secret","Public","50","0","","",file,"0","0","","","ja_JP","123"]);
            let decoded=parse(&bytes).ok().unwrap();
            let row=chat_event_bound(&decoded,Instant::now(),Some("fixture")).ok().unwrap().unwrap();
            assert_eq!(row.kind,"donation");assert_eq!(row.extra.as_ref().unwrap()["soopGift"]["count"],50);
            assert_eq!(row.gift_asset.is_some(),file=="50");
            let encoded=serde_json::to_string(&row).unwrap();
            for private in ["sender-secret","fixture(1)","https://","giftAsset","imageAssetId","../invalid"] {assert!(!encoded.contains(private));}
            let wrong=chat_event_bound(&decoded,Instant::now(),Some("another")).ok().unwrap().unwrap();
            assert!(wrong.gift_asset.is_none());assert_eq!(wrong.kind,"donation");
        }
    }
    #[test]
    fn ogq_native_receive_keeps_empty_public_rows_without_serializing_requests_or_accounts() {
        let fields=["room-private","","gem_fixture","0","version-private","sender-private","Public <b>","32|262144","255","0","0","jpg","7","E12E2E","private-dark","9","7","1","-1"];
        let bytes=frame(109,&fields);let mut decoded=parse(&bytes).ok().unwrap();decoded.id=Some("ogq-fixture");
        let mut rows=Vec::new();let mut summary=ChatSummary::default();let mut seen=Seen::default();
        for _ in 0..2 {receive_public(&decoded,Instant::now(),&mut|row|{rows.push(row);Ok(())},&mut summary,&mut seen).ok().unwrap();}
        assert_eq!(summary.messages,1);assert_eq!(rows.len(),1);assert_eq!(rows[0].kind,"chat");assert!(rows[0].text.is_empty());
        assert_eq!(rows[0].emoticon_assets.len(),4);assert_eq!(rows[0].extra.as_ref().unwrap()["nicknameColorKey"],"E12E2E");
        let encoded=serde_json::to_string(&rows[0]).unwrap();
        for private in ["room-private","sender-private","version-private","private-dark","https://","emoticonAssets","imageAssetId"] {assert!(!encoded.contains(private));}
        for kind in ["1","2","4"] {
            let mut changed=fields;changed[1]="readable text";changed[10]=kind;
            let bytes=frame(109,&changed);let decoded=parse(&bytes).ok().unwrap();
            receive_public(&decoded,Instant::now(),&mut|row|{rows.push(row);Ok(())},&mut summary,&mut seen).ok().unwrap();
        }
        assert_eq!(rows.iter().filter(|row|row.kind=="gap").count(),1);
        assert_eq!(rows.iter().filter(|row|row.kind=="chat").count(),2);
        assert_eq!(rows.last().unwrap().text,"readable text");assert!(rows.last().unwrap().emoticon_assets.is_empty());
        assert_eq!(summary.gaps,0);
    }
    #[test]
    fn unsupported_rich_and_malformed_gifts_warn_once_without_stopping_chat() {
        let started = Instant::now();
        let mut rows = Vec::new();
        let mut summary = ChatSummary::default();
        let mut seen = Seen::default();
        let mut frames: Vec<_> = [20, 34, 45, 70, 71, 86, 87, 91, 92, 93, 102, 103, 107, 108, 109, 118, 120, 121, 125, 142, 143]
            .into_iter().map(|service| frame(service, &["private-id", "do-not-store"])).collect();
        frames.extend([frame(18, &["malformed"]), frame(9, &["private-whisper"]), frame(144, &["private receipt"]),
            frame(5, &["public body", "private-account", "0", "0", "0", "Public viewer", "16"])]);
        for bytes in frames {
            let parsed = parse(&bytes).ok().unwrap();
            receive_public(&parsed, started, &mut |row| { rows.push(row); Ok(()) }, &mut summary, &mut seen).ok().unwrap();
        }
        assert_eq!(summary.unsupported_rich_events, 17);
        assert_eq!(summary.malformed_rich_events, 5);
        assert_eq!(summary.gaps, 0, "unsupported rich data is not a socket disconnect");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "gap");
        assert_eq!(rows[1].kind, "chat");
        let encoded = serde_json::to_string(&rows).unwrap();
        for private in ["private-id", "do-not-store", "private-whisper", "private-account"] { assert!(!encoded.contains(private)); }
    }
    #[test]
    fn subscriptions_keep_public_fields_with_partial_assets_and_no_private_packet_data() {
        let started=Instant::now();let mut summary=ChatSummary::default();let mut seen=Seen::default();let mut rows=Vec::new();
        for (service,fields) in [(91,vec!["room-private","receiver-private","sender-private","Public","103","unused","unused","2","unused","ko_KR","1"]),
            (93,vec!["channel-private","sender-private","Public","17","room-private","unused","999","2","unused","../bad","1"])] {
            let bytes=frame(service,&fields);let decoded=parse(&bytes).ok().unwrap();
            receive_public(&decoded,started,&mut |row| {rows.push(row);Ok(())},&mut summary,&mut seen).ok().unwrap();
        }
        assert_eq!(summary.messages,2);assert_eq!(summary.unsupported_rich_events,1);
        let memberships=rows.iter().filter(|row|row.kind=="membership").collect::<Vec<_>>();assert_eq!(memberships.len(),2);
        assert_eq!(memberships[0].subscription_assets.len(),2);assert_eq!(memberships[1].subscription_assets.len(),1);
        assert!(memberships.iter().all(|row|row.text.is_empty() && row.gift_asset.is_none() && row.emoticon_assets.is_empty()));
        let encoded=serde_json::to_string(&rows).unwrap();
        for secret in ["room-private","receiver-private","sender-private","channel-private","subscriptionAssets","https://","../bad"] {assert!(!encoded.contains(secret));}
        assert!(!crate::soop::chat::gifts::known_unsupported(91));
        assert!(!crate::soop::chat::gifts::known_unsupported(121));
    }
    #[test]
    fn malformed_frames_are_bounded() {
        assert!(parse(&[29, 9]).is_err());
        let mut bytes = frame(5, &["body"]);
        bytes[11] = b'9';
        assert!(parse(&bytes).is_err());
        let mut invalid = frame(5, &["body"]);
        *invalid.last_mut().unwrap() = 255;
        assert!(parse(&invalid).is_err());
        assert!(parse(&vec![29; MAX_FRAME + 1]).is_err());
        let tiny = Frame {
            service: 5,
            result: 0,
            id: None,
            body: "\x0c",
        };
        assert!(chat_event(&tiny, Instant::now()).is_err());
        let long = frame(5, &[&"a".repeat(16385), "id", "0", "0", "0", "name", "16"]);
        assert!(chat_event(&parse(&long).ok().unwrap(), Instant::now()).is_err());
    }
    #[test]
    fn duplicates_are_bounded() {
        let mut seen = Seen::default();
        assert!(seen.accept(Some("one")));
        assert!(!seen.accept(Some("one")));
        for i in 0..5000 {
            assert!(seen.accept(Some(&format!("id{i}"))));
        }
        assert_eq!(seen.order.len(), 4096);
        assert_eq!(seen.ids.len(), 4096);
        assert!(seen.accept(None));
    }
    #[test]
    fn already_cancelled_does_not_connect() {
        let stop = Arc::new(AtomicBool::new(true));
        let summary = capture("fixture", |_| panic!("no callback expected"), stop).unwrap();
        assert!(summary.stopped);
        assert_eq!(summary.connections, 0);
    }
    #[test]
    #[ignore = "explicit anonymous network chat probe; outputs counts only"]
    fn public_guest_chat_smoke() {
        let channel =
            std::env::var("ATSUMI_SOOP_CHAT_TEST_CHANNEL").unwrap_or_else(|_| "phonics1".into());
        let stop = Arc::new(AtomicBool::new(false));
        let deadline = stop.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(25));
            deadline.store(true, Ordering::Release);
        });
        let request_stop = stop.clone();
        let mut count = 0;
        let summary = capture(
            &channel,
            |row| {
                if row.kind == "chat" {
                    count += 1;
                    if count >= 3 {
                        request_stop.store(true, Ordering::Release);
                    }
                }
                Ok(())
            },
            stop,
        )
        .unwrap();
        assert!(summary.connections > 0);
        assert!(
            summary.messages > 0,
            "public channel had no chat in the bounded observation window"
        );
        println!("SOOP anonymous chat: connections={}, messages={}, gaps={}, video_requests=0, chat_messages_sent=0",summary.connections,summary.messages,summary.gaps);
    }
}

#[cfg(test)]
#[path="chat_personalcon_capture_tests.rs"]
mod personalcon_capture_tests;

#[cfg(test)]
#[path = "chat_standard_gift_capture_tests.rs"]
mod standard_gift_capture_tests;

#[cfg(test)]
#[path="chat_mission_capture_tests.rs"]
mod mission_capture_tests;
