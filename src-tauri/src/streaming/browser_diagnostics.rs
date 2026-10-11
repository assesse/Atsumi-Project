//! Bounded, local-only capture evidence. No media, chat text, URLs or credentials.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn capability(value: &Value) {
    let reason = value["reason"]
        .as_str()
        .filter(|value| crate::diagnostics::safe_capture_reason(value))
        .unwrap_or("other");
    tracing::info!(
        diag_stage = "capture_capability",
        capture_reason = reason,
        width = value["width"].as_u64().unwrap_or(0).min(16384),
        height = value["height"].as_u64().unwrap_or(0).min(16384)
    );
    for source in value["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["selected"].as_bool() == Some(true))
        .take(1)
    {
        for (index, track) in source["tracks"]
            .as_array()
            .into_iter()
            .flatten()
            .take(2)
            .enumerate()
        {
            let mime = track["mimeType"]
                .as_str()
                .filter(|value| crate::diagnostics::safe_capture_mime(value))
                .unwrap_or("unsupported");
            tracing::info!(
                diag_stage = "capture_track",
                track_index = index as u64,
                capture_mime = mime,
                init_bytes = track["initBytes"].as_u64().unwrap_or(0).min(65536)
            );
        }
    }
    if let Some(reason) = value["lastStop"]["reason"]
        .as_str()
        .filter(|value| crate::diagnostics::safe_capture_reason(value))
    {
        tracing::info!(diag_stage = "capture_last_stop", capture_reason = reason);
    }
}

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
    if let Some(reason) = packet["reason"]
        .as_str()
        .filter(|value| crate::diagnostics::safe_capture_reason(value))
    {
        tracing::info!(diag_stage = "capture_finished", capture_reason = reason);
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
            reason,
        } => {
            json!({"kind":"encoded_finish","recordingId":safe_id(recording_id),"interrupted":interrupted,
                "reason":reason.as_deref().filter(|value| crate::diagnostics::safe_capture_reason(value)).unwrap_or("other")})
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
    #[test]
    fn finish_evidence_keeps_precise_track_change_without_free_form_reason() {
        let finish = |reason: &str| BrowserMessage::EncodedFinish {
            recording_id: "a".repeat(32),
            interrupted: true,
            reason: Some(reason.into()),
        };
        assert_eq!(packet(&finish("track_changed"))["reason"], "track_changed");
        assert_eq!(packet(&finish("secret token"))["reason"], "other");
        assert!(!packet(&finish("secret token"))
            .to_string()
            .contains("secret"));
    }
}
