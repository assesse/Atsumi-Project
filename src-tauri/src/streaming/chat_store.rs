//! Durable append-only official chat log, independent of retired HLS storage.
//! Existing chat.jsonl files remain readable; this module never migrates media.
use super::model::{ChatMessage, StreamError};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    time::{Duration, Instant},
};

const MAX_JOURNAL_LINE: usize = 64 * 1024;

fn storage_error() -> StreamError {
    StreamError::new(
        "STREAM_STORAGE_FAILED",
        "채팅 파일을 저장하거나 읽을 수 없습니다. 저장 공간과 폴더 권한을 확인하세요.",
        false,
    )
}

fn append_json<T: Serialize>(file: &mut File, value: &T, sync: bool) -> Result<(), StreamError> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| storage_error())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_JOURNAL_LINE {
        return Err(storage_error());
    }
    file.write_all(&bytes).map_err(|_| storage_error())?;
    if sync {
        file.sync_all().map_err(|_| storage_error())?;
    }
    Ok(())
}

/// Read one history page backwards. Work and memory are proportional to the
/// displayed page, not the full (potentially hours-long) append-only chat log.
pub(crate) fn read_chat_page(
    root: &Path,
    before: Option<u64>,
) -> Result<super::model::ChatPage, StreamError> {
    let path = root.join("chat.jsonl");
    let metadata = fs::symlink_metadata(&path).map_err(|_| storage_error())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(storage_error());
    }
    let mut file = File::open(path).map_err(|_| storage_error())?;
    let length = file.metadata().map_err(|_| storage_error())?.len();
    let mut position = before.unwrap_or(length);
    if position > length {
        return Err(storage_error());
    }
    if before.is_some() && position > 0 {
        file.seek(SeekFrom::Start(position - 1))
            .map_err(|_| storage_error())?;
        let mut boundary = [0];
        file.read_exact(&mut boundary)
            .map_err(|_| storage_error())?;
        if boundary[0] != b'\n' {
            return Err(storage_error());
        }
    }
    let mut items = Vec::new();
    let mut reversed_line = Vec::new();
    let mut skip_tail = true;
    let mut tail_length = 0;
    let mut buffer = [0_u8; 64 * 1024];
    while position > 0 {
        let size = position.min(buffer.len() as u64) as usize;
        position -= size as u64;
        file.seek(SeekFrom::Start(position))
            .map_err(|_| storage_error())?;
        file.read_exact(&mut buffer[..size])
            .map_err(|_| storage_error())?;
        for index in (0..size).rev() {
            let byte = buffer[index];
            if skip_tail {
                // A concurrently written or crash-truncated last line is not committed.
                if byte == b'\n' {
                    skip_tail = false;
                } else {
                    tail_length += 1;
                    if tail_length > MAX_JOURNAL_LINE {
                        return Err(storage_error());
                    }
                }
                continue;
            }
            if byte == b'\n' {
                parse_chat_history_line(&mut reversed_line, &mut items)?;
                if items.len() == 200 {
                    items.reverse();
                    return Ok(super::model::ChatPage {
                        items,
                        previous_cursor: Some(position + index as u64 + 1),
                    });
                }
            } else {
                reversed_line.push(byte);
                if reversed_line.len() > MAX_JOURNAL_LINE {
                    return Err(storage_error());
                }
            }
        }
    }
    if !reversed_line.is_empty() {
        parse_chat_history_line(&mut reversed_line, &mut items)?;
    }
    items.reverse();
    Ok(super::model::ChatPage {
        items,
        previous_cursor: None,
    })
}

fn parse_chat_history_line(
    line: &mut Vec<u8>,
    items: &mut Vec<ChatMessage>,
) -> Result<(), StreamError> {
    line.reverse();
    let message: ChatMessage = serde_json::from_slice(line).map_err(|_| storage_error())?;
    if message.sequence == 0
        || message.sender.chars().count() > 128
        || message.text.chars().count() > 4096
        || !message.offset_seconds.is_finite()
        || message.offset_seconds < 0.0
        || message
            .broadcast_offset_seconds
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        || items
            .last()
            .is_some_and(|newer| message.sequence >= newer.sequence)
    {
        return Err(storage_error());
    }
    items.push(message);
    line.clear();
    Ok(())
}

pub(crate) struct ChatStore {
    file: File,
    pending: usize,
    last_sync: Instant,
}

impl ChatStore {
    pub fn create(root: &Path) -> Result<Self, StreamError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("chat.jsonl"))
            .map_err(|_| storage_error())?;
        Ok(Self {
            file,
            pending: 0,
            last_sync: Instant::now(),
        })
    }

    pub fn append(&mut self, message: &ChatMessage) -> Result<(), StreamError> {
        append_json(&mut self.file, message, false)?;
        self.pending += 1;
        if self.pending >= 32 || self.last_sync.elapsed() >= Duration::from_secs(1) {
            self.sync()?;
        }
        Ok(())
    }

    pub fn sync(&mut self) -> Result<(), StreamError> {
        self.file.sync_all().map_err(|_| storage_error())?;
        self.pending = 0;
        self.last_sync = Instant::now();
        Ok(())
    }

    pub fn sync_if_due(&mut self) -> Result<(), StreamError> {
        if self.pending > 0 && self.last_sync.elapsed() >= Duration::from_secs(1) {
            self.sync()?;
        }
        Ok(())
    }
}
impl Drop for ChatStore {
    fn drop(&mut self) {
        let _ = self.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(sequence: u64) -> ChatMessage {
        ChatMessage {
            sequence,
            sender: "fixture".into(),
            text: "preserved".into(),
            server_time: None,
            received_at: sequence,
            offset_seconds: sequence as f64,
            broadcast_offset_seconds: None,
            replay_clock: None,
            sender_key: None,
            rich: None,
        }
    }

    #[test]
    fn durable_chat_batches_flush_and_existing_logs_are_never_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = ChatStore::create(directory.path()).unwrap();
        for sequence in 1..=32 {
            log.append(&message(sequence)).unwrap();
        }
        assert_eq!(log.pending, 0);
        log.append(&message(33)).unwrap();
        assert_eq!(log.pending, 1);
        log.last_sync = Instant::now() - Duration::from_secs(2);
        log.sync_if_due().unwrap();
        assert_eq!(log.pending, 0);
        drop(log);
        let path = directory.path().join("chat.jsonl");
        let original = fs::read(&path).unwrap();
        assert!(ChatStore::create(directory.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            read_chat_page(directory.path(), None).unwrap().items.len(),
            33
        );
    }

    #[test]
    fn oversized_chat_record_and_invalid_history_leave_committed_bytes_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = ChatStore::create(directory.path()).unwrap();
        log.append(&message(1)).unwrap();
        log.sync().unwrap();
        let path = directory.path().join("chat.jsonl");
        let original = fs::read(&path).unwrap();
        let mut too_large = message(2);
        too_large.text = "x".repeat(MAX_JOURNAL_LINE);
        assert!(log.append(&too_large).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        drop(log);
        let mut output = OpenOptions::new().append(true).open(&path).unwrap();
        output.write_all(b"{invalid}\n").unwrap();
        output.sync_all().unwrap();
        drop(output);
        let damaged = fs::read(&path).unwrap();
        assert!(read_chat_page(directory.path(), None).is_err());
        assert_eq!(fs::read(&path).unwrap(), damaged);
    }

    #[test]
    fn chat_decorations_survive_bounded_history_with_legacy_and_malformed_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = ChatStore::create(directory.path()).unwrap();
        for sequence in 1..=3 {
            let mut value = serde_json::json!({
                "sequence":sequence,"sender":"viewer","text":"preserved {:wave:}",
                "serverTime":null,"receivedAt":1000,"offsetSeconds":1.0
            });
            if sequence == 2 {
                value["rich"] = serde_json::json!({
                    "nicknameColor":"#123ABC",
                    "badges":[{"kind":"activity","id":"supplied","title":"전달된 배지","imageUrl":"https://ssl.pstatic.net/badge.png"}],
                    "emojis":[{"id":"wave","imageUrl":"https://nng-phinf.pstatic.net/wave.png"}]
                });
            } else if sequence == 3 {
                value["rich"] = serde_json::json!({"badges":"invalid decoration"});
            }
            append_json(&mut log.file, &value, false).unwrap();
        }
        log.sync().unwrap();
        let original = fs::read(directory.path().join("chat.jsonl")).unwrap();
        let page = read_chat_page(directory.path(), None).unwrap();
        assert_eq!(page.items.len(), 3);
        for messages in [&page.items[..]] {
            assert!(messages[0].rich.is_none());
            let rich = messages[1].rich.as_ref().unwrap();
            assert_eq!(rich.nickname_color.as_deref(), Some("#123ABC"));
            assert_eq!(rich.badges[0].title.as_deref(), Some("전달된 배지"));
            assert_eq!(rich.emojis[0].id, "wave");
            assert!(messages[2].rich.is_none());
            assert!(messages
                .iter()
                .all(|message| message.text == "preserved {:wave:}"));
        }
        assert_eq!(
            fs::read(directory.path().join("chat.jsonl")).unwrap(),
            original
        );
    }

    #[test]
    fn maximum_chat_decoration_bounds_fit_the_jsonl_record_budget() {
        use super::super::model::{ChatBadge, ChatBadgeKind, ChatEmoji, ChatRich};
        let long_url = |index: usize| {
            let prefix = format!("https://ssl.pstatic.net/{index}/");
            let length = 1024 - prefix.len();
            prefix + &"a".repeat(length)
        };
        let rich = ChatRich {
            nickname_color: Some("#FFFFFF".into()),
            badges: (0..12)
                .map(|index| ChatBadge {
                    kind: ChatBadgeKind::Subscription,
                    id: Some("x".repeat(64)),
                    title: Some("\0".repeat(100)),
                    image_url: long_url(index),
                })
                .collect(),
            emojis: (0..20)
                .map(|index| ChatEmoji {
                    id: format!("{index:064}"),
                    image_url: long_url(index),
                })
                .collect(),
        }
        .bounded()
        .unwrap();
        assert_eq!(rich.badges.len(), 8);
        assert_eq!(rich.emojis.len(), 16);
        assert_eq!(rich.badges[0].title.as_ref().unwrap().chars().count(), 64);
        for character in ['\0', '🟢'] {
            let message = ChatMessage {
                sequence: u64::MAX,
                sender: character.to_string().repeat(128),
                text: character.to_string().repeat(4096),
                server_time: Some(u64::MAX),
                received_at: u64::MAX,
                offset_seconds: 1.0,
                broadcast_offset_seconds: Some(1.0),
                replay_clock: None,
                sender_key: None,
                rich: Some(rich.clone()),
            };
            let serialized = serde_json::to_vec(&message).unwrap();
            assert!(serialized.len() + 1 < MAX_JOURNAL_LINE);
            let restored: ChatMessage = serde_json::from_slice(&serialized).unwrap();
            assert_eq!(restored.rich, message.rich);
            assert_eq!(restored.text, message.text);
        }
    }

    #[test]
    fn chat_history_preserves_every_message_and_pages_backwards_without_duplicates() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = ChatStore::create(directory.path()).unwrap();
        for sequence in 1..=1003 {
            log.append(&ChatMessage {
                sequence,
                sender: "시청자".into(),
                text: "전체 기록 🟢".repeat(20),
                server_time: None,
                received_at: sequence * 1000,
                offset_seconds: sequence as f64,
                broadcast_offset_seconds: Some(sequence as f64 + 3600.0),
                replay_clock: None,
                sender_key: None,
                rich: None,
            })
            .unwrap();
        }
        log.sync().unwrap();
        let original = fs::read(directory.path().join("chat.jsonl")).unwrap();
        assert_eq!(original.iter().filter(|&&byte| byte == b'\n').count(), 1003);
        let mut cursor = None;
        let mut sequences = Vec::new();
        loop {
            let page = read_chat_page(directory.path(), cursor).unwrap();
            assert!(page.items.len() <= 200);
            sequences.extend(page.items.iter().map(|entry| entry.sequence));
            cursor = page.previous_cursor;
            if cursor.is_none() {
                break;
            }
        }
        sequences.sort_unstable();
        assert_eq!(sequences, (1..=1003).collect::<Vec<_>>());
        assert_eq!(
            fs::read(directory.path().join("chat.jsonl")).unwrap(),
            original
        );
        assert!(read_chat_page(directory.path(), Some(7)).is_err());
        assert!(read_chat_page(directory.path(), Some(original.len() as u64 + 1)).is_err());
    }

    #[test]
    fn history_preserves_clock_and_digest_and_downgrades_malformed_optional_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chat.jsonl");
        let mut log = ChatStore::create(directory.path()).unwrap();
        let clock = serde_json::json!({"version":1,"receivedAtMs":1000,"observedMonotonicMs":10.0,
            "sourceGeneration":1,"clock":"mse_presentation_v1","mediaTimeSeconds":4002.5,
            "sourceTimeSeconds":4002.5,"sourceId":"40000000-0000-4000-8000-000000000001","playbackRate":2.0});
        for sequence in 1..=3 {
            let mut value = serde_json::to_value(message(sequence)).unwrap();
            if sequence == 2 {
                value["replayClock"] = clock.clone();
                value["senderKey"] = serde_json::json!(format!("sha256:{}", "a".repeat(64)));
            } else if sequence == 3 {
                value["replayClock"] = serde_json::json!({"version":1,"sourceGeneration":0});
                value["senderKey"] = serde_json::json!("raw_private_id");
            }
            append_json(&mut log.file, &value, false).unwrap();
        }
        log.sync().unwrap();
        let original = fs::read(&path).unwrap();
        let page = read_chat_page(directory.path(), None).unwrap();
        assert!(page.items[0].replay_clock.is_none());
        assert_eq!(
            page.items[1]
                .replay_clock
                .as_ref()
                .unwrap()
                .source_time_seconds,
            Some(4002.5)
        );
        assert_eq!(
            page.items[1].sender_key,
            Some(format!("sha256:{}", "a".repeat(64)))
        );
        assert!(page.items[2].replay_clock.is_none());
        assert!(page.items[2].sender_key.is_none());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn chat_history_reads_legacy_records_and_leaves_incomplete_tail_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chat.jsonl");
        let original = b"{\"sequence\":1,\"sender\":\"old\",\"text\":\"message\",\"serverTime\":null,\"receivedAt\":100,\"offsetSeconds\":0.1}\n{\"sequence\":2";
        fs::write(&path, original).unwrap();
        let page = read_chat_page(directory.path(), None).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].broadcast_offset_seconds, None);
        assert_eq!(page.previous_cursor, None);
        assert_eq!(fs::read(path).unwrap(), original);
    }
}
