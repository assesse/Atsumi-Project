use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl StreamError {
    pub fn new(code: &str, message: &str, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}
impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for StreamError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LiveStatus {
    Live,
    Offline,
    Restricted,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveInfo {
    pub channel_id: String,
    pub channel_name: String,
    pub title: String,
    pub live_id: Option<String>,
    #[serde(default)]
    pub broadcast_started_at: Option<u64>,
    pub status: LiveStatus,
    pub chat_available: bool,
    /// Existing public metadata only; never serialize a chat routing identifier.
    #[serde(skip)]
    pub(crate) chat_channel_id: Option<String>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatState {
    Disabled,
    Connecting,
    Connected,
    Reconnecting,
    Stopped,
    Failed,
}

#[derive(Debug, Clone)]
pub enum ChatEvent {
    Status(ChatState, Option<String>),
    Message {
        sender: String,
        text: String,
        server_time: Option<u64>,
        rich: Option<ChatRich>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatBadgeKind {
    Subscription,
    Activity,
    Profile,
    Donation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatBadge {
    pub kind: ChatBadgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub image_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatEmoji {
    pub id: String,
    pub image_url: String,
}

/// Only display metadata received with a message. Never persist the raw profile,
/// raw profiles or extras (which may include an extraToken). A canonical public
/// profile link is optional and intentionally identifiable; it is not senderKey.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatRich {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_url: Option<String>,
    #[serde(default)]
    pub badges: Vec<ChatBadge>,
    #[serde(default)]
    pub emojis: Vec<ChatEmoji>,
}

impl ChatRich {
    /// The bounds also keep a maximum-size escaped chat JSONL record below 64 KiB.
    /// Apply again on disk reads so hand-edited archives cannot bypass the checks.
    pub(crate) fn bounded(mut self) -> Option<Self> {
        self.nickname_color = self.nickname_color.filter(|color| {
            color.len() == 7
                && color.starts_with('#')
                && color.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit())
        });
        self.text_color = self.text_color.filter(|color| {
            color.len() == 7
                && color.starts_with('#')
                && color.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit())
        });
        self.profile_url = self.profile_url.and_then(|url| public_profile_url(&url));
        let mut seen_badges = std::collections::HashSet::new();
        self.badges = self
            .badges
            .into_iter()
            .filter_map(|mut badge| {
                badge.image_url = super::chat_assets::sanitize_chat_asset_url(&badge.image_url)?;
                if !seen_badges.insert(badge.image_url.clone()) {
                    return None;
                }
                badge.id = badge.id.filter(|id| {
                    !id.is_empty()
                        && id.len() <= 64
                        && id.bytes().all(|byte| byte.is_ascii_graphic())
                });
                badge.title = badge.title.map(|title| title.chars().take(64).collect());
                Some(badge)
            })
            .take(8)
            .collect();
        self.emojis = self
            .emojis
            .into_iter()
            .filter_map(|mut emoji| {
                if emoji.id.is_empty()
                    || emoji.id.len() > 64
                    || !emoji
                        .id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    return None;
                }
                emoji.image_url = super::chat_assets::sanitize_chat_asset_url(&emoji.image_url)?;
                Some(emoji)
            })
            .take(16)
            .collect();
        if self.nickname_color.is_none()
            && self.text_color.is_none()
            && self.profile_url.is_none()
            && self.badges.is_empty()
            && self.emojis.is_empty()
        {
            None
        } else {
            Some(self)
        }
    }
}

/// Exact public channel/profile page only; never an API, login, query or redirect.
pub(crate) fn public_profile_url(value: &str) -> Option<String> {
    let id = value.strip_prefix("https://chzzk.naver.com/")?;
    (id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("https://chzzk.naver.com/{}", id.to_ascii_lowercase()))
}

fn deserialize_chat_rich<'de, D>(deserializer: D) -> Result<Option<ChatRich>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    // Optional decoration must never make an otherwise valid text record unreadable.
    Ok(serde_json::from_value::<ChatRich>(value)
        .ok()
        .and_then(ChatRich::bounded))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReplayClock {
    pub version: u8,
    pub received_at_ms: u64,
    pub observed_monotonic_ms: f64,
    pub source_generation: u64,
    pub clock: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_time_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_time_seconds: Option<f64>,
}

impl ChatReplayClock {
    pub(crate) fn bounded(self) -> Option<Self> {
        let seconds = |value: f64| value.is_finite() && (0.0..=1_000_000_000.0).contains(&value);
        (self.version == 1
            && self.received_at_ms <= 9_007_199_254_740_991
            && self.observed_monotonic_ms.is_finite()
            && (0.0..=1_000_000_000_000.0).contains(&self.observed_monotonic_ms)
            && (1..=9_007_199_254_740_991).contains(&self.source_generation)
            && matches!(
                self.clock.as_str(),
                "player_observation" | "mse_presentation_v1"
            )
            && self.media_time_seconds.is_none_or(seconds)
            && self.source_time_seconds.is_none_or(seconds)
            && self
                .playback_rate
                .is_none_or(|rate| rate.is_finite() && (0.25..=4.0).contains(&rate))
            && self
                .source_id
                .as_ref()
                .is_none_or(|id| id.len() == 36 && uuid::Uuid::parse_str(id).is_ok())
            && (self.clock != "mse_presentation_v1"
                || (self.source_id.is_some()
                    && self.source_time_seconds.is_some()
                    && self.source_time_seconds == self.media_time_seconds)))
            .then_some(self)
    }

    pub(crate) fn observation_only(&mut self) {
        self.clock = "player_observation".into();
        self.source_id = None;
        self.source_time_seconds = None;
    }
}

fn deserialize_replay_clock<'de, D>(deserializer: D) -> Result<Option<ChatReplayClock>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<ChatReplayClock>(value)
        .ok()
        .and_then(ChatReplayClock::bounded))
}

pub(crate) fn bounded_sender_key(value: &str) -> Option<String> {
    let digest = value.strip_prefix("sha256:")?;
    (digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then(|| value.to_owned())
}

fn deserialize_sender_key<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_str().and_then(bounded_sender_key))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub sequence: u64,
    pub sender: String,
    pub text: String,
    pub server_time: Option<u64>,
    pub received_at: u64,
    /// Approximate receive-time offset, not a promise of media PTS synchronization.
    pub offset_seconds: f64,
    /// Broadcast uptime, separate from the recording-relative receive timestamp.
    #[serde(default)]
    pub broadcast_offset_seconds: Option<f64>,
    /// Socket receipt observation; absent in older archives. Only the explicitly
    /// validated MSE clock maps original presentation time into encoded segments.
    #[serde(
        default,
        deserialize_with = "deserialize_replay_clock",
        skip_serializing_if = "Option::is_none"
    )]
    pub replay_clock: Option<ChatReplayClock>,
    /// Per-recording salted digest. Neither raw account IDs nor salts are saved.
    #[serde(
        default,
        deserialize_with = "deserialize_sender_key",
        skip_serializing_if = "Option::is_none"
    )]
    pub sender_key: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_chat_rich",
        skip_serializing_if = "Option::is_none"
    )]
    pub rich: Option<ChatRich>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatPage {
    pub items: Vec<ChatMessage>,
    /// Byte offset into this session's append-only chat log. Never a filesystem path.
    pub previous_cursor: Option<u64>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
