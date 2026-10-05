//! Bounded, local-only capture evidence. No media, chat text, URLs or credentials.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn record(_: &OfficialBrowser, event: &str, fields: Value) {
    // Old per-session capture files are left intact. New evidence uses the
    // shared ring instead of adding another permanent 32 MiB file each run.
    static PACKETS: AtomicU64 = AtomicU64::new(0);
    let count = PACKETS.fetch_add(1, Ordering::Relaxed) + 1;
    let queued = fields["queuedMs"].as_u64().unwrap_or(0);
    let processing = fields["processMs"].as_u64().unwrap_or(0);
    let packet = fields.get("packet").unwrap_or(&fields);
    let kind = packet["kind"].as_str().unwrap_or("unknown");
    // Routine packets are sampled, not written once per video/chat chunk.
    if event == "bridge_processed"
        && fields["errorCode"].is_null()
        && queued < 1000
        && processing < 1000
        && !matches!(kind, "encoded_begin" | "encoded_finish")
        && !count.is_multiple_of(128)
    {
        return;
    }
    let stage = match event {
        "bridge_processed" => "capture_bridge_processed",
        "bridge_state_busy" => "capture_state_busy",
        "bridge_busy" => "capture_bridge_busy",
        "capture_transport" => "capture_transport",
        _ => return,
    };
    let recording = packet["recordingId"].as_str().and_then(safe_id);
    let _diagnostic = crate::diagnostics::operation("capture_bridge", recording).entered();
    tracing::info!(
        diag_stage = stage,
        count,
        wait_ms = queued,
        elapsed_ms = processing,
        bytes = packet["encodedBytes"].as_u64().unwrap_or(0),
        success = fields["errorCode"].is_null()
    );
    if let Some(code) = fields["errorCode"].as_str() {
        tracing::warn!(error_code = code, diag_stage = "capture_failed");
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
