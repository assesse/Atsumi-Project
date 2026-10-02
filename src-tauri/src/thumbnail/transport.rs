//! Bounded, one-read capabilities. Large image bodies never enter an event JSON
//! or an unbounded completion queue. The existing coordinator remains the owner
//! of resolution/caching; this store only leases already prepared bytes.
use super::{ThumbnailCompletionEventDto, ThumbnailCompletionOutcomeDto, ThumbnailKey};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

const MAX_LEASES: usize = 128;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const LEASE_TTL: Duration = Duration::from_secs(45);

struct Lease {
    request_id: String,
    key: ThumbnailKey,
    body: Option<Vec<u8>>,
    byte_len: usize,
    created: Instant,
}

struct State {
    epoch: String,
    leases: HashMap<String, Lease>,
    bytes: usize,
    offered: u64,
    read: u64,
    refused: u64,
}

pub struct ThumbnailTransport(Mutex<State>);

impl Default for ThumbnailTransport {
    fn default() -> Self {
        Self(Mutex::new(State {
            epoch: uuid::Uuid::new_v4().to_string(),
            leases: HashMap::new(),
            bytes: 0,
            offered: 0,
            read: 0,
            refused: 0,
        }))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportStats {
    pub leases: usize,
    pub bytes: usize,
    pub offered: u64,
    pub read: u64,
    pub refused: u64,
}

impl State {
    fn remove(&mut self, token: &str) -> bool {
        if let Some(lease) = self.leases.remove(token) {
            self.bytes = self.bytes.saturating_sub(lease.byte_len);
            true
        } else {
            false
        }
    }
    fn expire(&mut self, now: Instant) {
        let expired: Vec<_> = self
            .leases
            .iter()
            .filter(|(_, lease)| now.saturating_duration_since(lease.created) >= LEASE_TTL)
            .map(|(token, _)| token.clone())
            .collect();
        for token in expired {
            self.remove(&token);
        }
    }
}

impl ThumbnailTransport {
    pub fn epoch(&self) -> String {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .epoch
            .clone()
    }

    pub fn reset_document(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.epoch = uuid::Uuid::new_v4().to_string();
        state.leases.clear();
        state.bytes = 0;
    }

    /// None means the originating document was replaced. Never publish its
    /// result into the new document (nor let it allocate a new lease there).
    pub fn publish(
        &self,
        epoch: &str,
        event: ThumbnailCompletionEventDto,
    ) -> Option<serde_json::Value> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.epoch != epoch {
            return None;
        }
        state.expire(Instant::now());
        let request_id = event.request_id;
        let key = event.key;
        let outcome = match event.outcome {
            ThumbnailCompletionOutcomeDto::Failed { failure } => {
                serde_json::json!({"status":"failed", "failure":failure})
            }
            ThumbnailCompletionOutcomeDto::Ready { delivery } => {
                let image = delivery.thumbnail;
                let byte_len = image.bytes.len();
                if byte_len == 0
                    || state.leases.len() >= MAX_LEASES
                    || byte_len > MAX_BYTES.saturating_sub(state.bytes)
                {
                    state.refused += 1;
                    serde_json::json!({"status":"failed", "failure": {
                        "key":key, "code":"temporarilyUnavailable", "message":"Image delivery capacity is busy",
                        "retryable":true, "negativeCacheHit":false
                    }})
                } else {
                    let token = uuid::Uuid::new_v4().to_string();
                    state.bytes += byte_len;
                    state.offered += 1;
                    state.leases.insert(
                        token.clone(),
                        Lease {
                            request_id: request_id.clone(),
                            key: key.clone(),
                            body: Some(image.bytes),
                            byte_len,
                            created: Instant::now(),
                        },
                    );
                    serde_json::json!({"status":"ready", "delivery":{
                        "key":delivery.key, "cacheStatus":delivery.cache_status,
                        "thumbnail": {"resourceToken":token, "byteLength":byte_len,
                            "contentType":image.content_type, "width":image.width, "height":image.height,
                            "sourceRevision":image.source_revision}
                    }})
                }
            }
        };
        Some(serde_json::json!({"requestId":request_id, "key":key, "outcome":outcome}))
    }

    /// Keep the byte charge until the frontend acknowledges creating its Blob.
    /// A read cannot be repeated to amplify copies of the same capability.
    pub fn read(&self, token: &str) -> Option<Vec<u8>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.expire(Instant::now());
        let body = state.leases.get_mut(token)?.body.take()?;
        state.read += 1;
        Some(body)
    }
    pub fn release(&self, token: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(token)
    }
    pub fn cancel_request(&self, request_id: &str) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let tokens: Vec<_> = state
            .leases
            .iter()
            .filter(|(_, v)| v.request_id == request_id)
            .map(|(k, _)| k.clone())
            .collect();
        for token in tokens {
            state.remove(&token);
        }
    }
    pub fn invalidate(&self, key: &ThumbnailKey) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let tokens: Vec<_> = state
            .leases
            .iter()
            .filter(|(_, v)| &v.key == key)
            .map(|(k, _)| k.clone())
            .collect();
        for token in tokens {
            state.remove(&token);
        }
    }
    pub fn stats(&self) -> TransportStats {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.expire(Instant::now());
        TransportStats {
            leases: state.leases.len(),
            bytes: state.bytes,
            offered: state.offered,
            read: state.read,
            refused: state.refused,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thumbnail::{
        ResolvedThumbnail, ThumbnailCacheStatus, ThumbnailDeliveryDto, ThumbnailRequestTokenDto,
    };
    fn event(id: &str, len: usize) -> ThumbnailCompletionEventDto {
        let key = ThumbnailKey::gallery_cover(1).unwrap();
        ThumbnailCompletionEventDto::from_result(
            ThumbnailRequestTokenDto {
                request_id: id.into(),
                key: key.clone(),
            },
            Ok(ThumbnailDeliveryDto {
                key,
                cache_status: ThumbnailCacheStatus::Resolved,
                thumbnail: ResolvedThumbnail {
                    content_type: "image/webp".into(),
                    bytes: vec![7; len],
                    width: 2,
                    height: 2,
                    source_revision: None,
                },
            }),
        )
    }
    #[test]
    fn event_is_small_and_body_is_one_read_until_acknowledgement() {
        let store = ThumbnailTransport::default();
        let value = store
            .publish(&store.epoch(), event("one", 1_000_000))
            .unwrap();
        assert!(serde_json::to_vec(&value).unwrap().len() < 700);
        assert!(value["outcome"]["delivery"]["thumbnail"]
            .get("bytes")
            .is_none());
        let token = value["outcome"]["delivery"]["thumbnail"]["resourceToken"]
            .as_str()
            .unwrap();
        assert_eq!(store.read(token).unwrap().len(), 1_000_000);
        assert!(store.read(token).is_none());
        assert_eq!(store.stats().bytes, 1_000_000);
        assert!(store.release(token));
        assert_eq!(store.stats().bytes, 0);
    }
    #[test]
    fn old_documents_cancelled_requests_and_changed_evidence_cannot_be_read() {
        let store = ThumbnailTransport::default();
        let epoch = store.epoch();
        let value = store.publish(&epoch, event("one", 10)).unwrap();
        let token = value["outcome"]["delivery"]["thumbnail"]["resourceToken"]
            .as_str()
            .unwrap();
        store.cancel_request("one");
        assert!(store.read(token).is_none());
        store.publish(&epoch, event("two", 10));
        store.invalidate(&ThumbnailKey::gallery_cover(1).unwrap());
        assert_eq!(store.stats().leases, 0);
        store.reset_document();
        assert!(store.publish(&epoch, event("late", 10)).is_none());
    }
    #[test]
    fn delivery_is_byte_and_count_bounded_and_expiry_reclaims_it() {
        let store = ThumbnailTransport::default();
        let epoch = store.epoch();
        for i in 0..MAX_LEASES {
            store.publish(&epoch, event(&i.to_string(), 1));
        }
        let refused = store.publish(&epoch, event("excess", 1)).unwrap();
        assert_eq!(refused["outcome"]["status"], "failed");
        assert_eq!(store.stats().leases, MAX_LEASES);
        store.0.lock().unwrap().expire(Instant::now() + LEASE_TTL);
        assert_eq!(store.stats().bytes, 0);
        store.0.lock().unwrap().bytes = MAX_BYTES;
        let refused = store.publish(&epoch, event("too-large", 1)).unwrap();
        assert_eq!(refused["outcome"]["status"], "failed");
    }
}
