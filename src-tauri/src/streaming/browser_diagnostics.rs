//! Bounded, local-only capture evidence. No media, chat text, URLs or credentials.
use super::*;
use std::io::Write;
use std::sync::{mpsc, OnceLock};

static WRITER: OnceLock<Option<mpsc::SyncSender<Value>>> = OnceLock::new();

pub(super) fn record(host: &OfficialBrowser, event: &str, fields: Value) {
    if cfg!(test) {
        return;
    }
    let sender = WRITER.get_or_init(|| {
        let root = host
            .inner
            .data_dir
            .join("streaming")
            .join("browser")
            .join("diagnostics");
        std::fs::create_dir_all(&root).ok()?;
        let path = root.join(format!("capture-{}-{}.jsonl", now_ms(), std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .ok()?;
        let (sender, receiver) = mpsc::sync_channel::<Value>(256);
        thread::Builder::new()
            .name("chzzk-capture-diagnostics".into())
            .spawn(move || {
                let mut written = 0usize;
                while let Ok(value) = receiver.recv() {
                    let Ok(mut bytes) = serde_json::to_vec(&value) else {
                        continue;
                    };
                    bytes.push(b'\n');
                    // Keep existing sessions forever; stop this session's diagnostics
                    // at 32 MiB instead of deleting evidence or blocking recording.
                    if written + bytes.len() > 32 * 1024 * 1024 {
                        break;
                    }
                    if file.write_all(&bytes).and_then(|_| file.flush()).is_err() {
                        break;
                    }
                    written += bytes.len();
                }
            })
            .ok()?;
        Some(sender)
    });
    if let Some(sender) = sender {
        let _ = sender
            .try_send(json!({"at":now_ms(),"webview":host.label(),"event":event,"fields":fields}));
    }
}

pub(super) fn packet(message: &BrowserMessage) -> Value {
    match message {
        BrowserMessage::EncodedAppend {
            recording_id,
            track_index,
            append_index,
            chunk_index,
            final_chunk,
            data,
        } => {
            json!({"kind":"encoded_append","recordingId":safe_id(recording_id),"track":track_index,"append":append_index,"chunk":chunk_index,"final":final_chunk,"encodedBytes":data.len()})
        }
        BrowserMessage::EncodedBegin { .. } => json!({"kind":"encoded_begin"}),
        BrowserMessage::EncodedFinish {
            recording_id,
            interrupted,
            ..
        } => {
            json!({"kind":"encoded_finish","recordingId":safe_id(recording_id),"interrupted":interrupted})
        }
        BrowserMessage::ChatBatch {
            recording_id,
            batch_id,
            events,
        } => {
            json!({"kind":"chat_batch","recordingId":safe_id(recording_id),"batch":batch_id,"events":events.len()})
        }
        BrowserMessage::Status { .. } => json!({"kind":"status"}),
        _ => json!({"kind":"control"}),
    }
}

fn safe_id(id: &str) -> Option<&str> {
    ((id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        || (id.len() == 36 && uuid::Uuid::parse_str(id).is_ok()))
    .then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packet_evidence_drops_untrusted_text_and_payloads() {
        let packet = packet(&BrowserMessage::ChatBatch {
            recording_id: "private text".into(),
            batch_id: Some(1),
            events: vec![json!({"msg":"private chat"})],
        });
        assert!(packet["recordingId"].is_null());
        assert!(!packet.to_string().contains("private"));
        assert_eq!(packet["events"], 1);
    }
}
