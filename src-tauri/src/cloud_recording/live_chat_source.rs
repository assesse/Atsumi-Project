//! Direct per-event CHZZK source observations, read only through a native lease.
//! A projected transport coordinate does not by itself establish remote sync.
use super::*;
use crate::streaming::model::{ChatMessage, ChatReplayClock};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceChatPage {
    pub items: Vec<SourceChatItem>,
    pub cursor: Option<String>,
    pub remote_anchor: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceChatItem {
    /// Original saved text, rich appearance and source metadata are retained.
    pub message: ChatMessage,
    pub position: Option<SourceChatPosition>,
    pub unmapped_reason: Option<&'static str>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceChatPosition {
    pub source_ticks90k: u64,
    pub transport_ticks90k: u64,
    pub source_generation: u64,
    pub segment_ordinal: u64,
    pub segment_sha256: String,
    /// Only f64 -> 90k and rational-origin rounding, NOT total sync precision.
    pub numeric_uncertainty_ticks90k: u32,
    /// Saved currentTime precision and event/display sampling lag are unknown.
    pub presentation_uncertainty: &'static str,
}
struct Observed { ticks90k: u64, source: uuid::Uuid, generation: u64 }
fn direct(clock: Option<&ChatReplayClock>) -> std::result::Result<Observed, &'static str> {
    let clock = clock.ok_or("missing_source_clock")?;
    if clock.clone().bounded().is_none() || clock.clock != "mse_presentation_v1" {
        return Err("unknown_source_clock");
    }
    let source = clock.source_id.as_deref().and_then(|s| uuid::Uuid::parse_str(s).ok()
        .filter(|id| !id.is_nil() && id.to_string() == s)).ok_or("invalid_source_clock")?;
    let seconds = clock.source_time_seconds.ok_or("missing_source_clock")?;
    // Bounded source seconds keep the product below 2^53. One 90k tick covers
    // binary multiplication and rounding; origin subtraction adds one more.
    let ticks = seconds * 90_000.;
    if !ticks.is_finite() || !(0.0..=90_000_000_000_000.0).contains(&ticks) {
        return Err("invalid_source_clock");
    }
    Ok(Observed { ticks90k: ticks.round() as u64, source, generation: clock.source_generation })
}

impl LiveHistoryLease {
    /// Selector is an existing bounded byte cursor, never event JSON or a path.
    /// No fallback to receive-time/server-time or a fabricated zero is allowed.
    pub(crate) fn source_chat_page(&self, cursor: Option<&str>, limit: usize) -> Result<SourceChatPage, CloudError> {
        let _active = self.work.begin()?;
        self.valid()?;
        if self.provider != "chzzk" { return Err(unavailable()); }
        let run = uuid::Uuid::parse_str(&self.sidecar_id).map_err(|_| unavailable())?;
        let LiveHistoryPage::Chzzk { items, cursor } = self.page(cursor, limit)? else { return Err(unavailable()); };
        let mut prepared = Vec::with_capacity(items.len());
        let mut points = Vec::new();
        for message in items {
            let observed = direct(message.replay_clock.as_ref()).and_then(|point| {
                if self.capture.owns_reference(run,point.source) { Ok(point) } else { Err("foreign_source_clock") }
            });
            let query = observed.as_ref().ok().map(|point| { let index=points.len(); points.push(point.ticks90k); index });
            prepared.push((message,observed,query));
        }
        let positions = self.timing.map_video_observations(&points,&self.work.cancel).map_err(|_| unavailable())?;
        let mut projected = Vec::with_capacity(prepared.len());
        for (message,observed,query) in prepared {
            let (position,unmapped_reason) = match (observed,query) {
                (Ok(point),Some(index)) => match positions[index].as_ref() {
                    Some(mapped) => (Some(SourceChatPosition { source_ticks90k:point.ticks90k,
                        transport_ticks90k:mapped.transport_ticks90k,source_generation:point.generation,
                        segment_ordinal:mapped.ordinal,segment_sha256:mapped.segment_sha256.clone(),
                        numeric_uncertainty_ticks90k:2,presentation_uncertainty:"unknown" }),None),
                    None => (None,Some("outside_acknowledged_envelope")),
                },
                (Err(reason),_) => (None,Some(reason)),
                _ => return Err(unavailable()),
            };
            projected.push(SourceChatItem {message,position,unmapped_reason});
        }
        self.valid()?;
        Ok(SourceChatPage {items:projected,cursor,remote_anchor:"unknown"})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn clock() -> ChatReplayClock {
        serde_json::from_value(json!({"version":1,"receivedAtMs":1000,"observedMonotonicMs":20.,
            "sourceGeneration":3,"clock":"mse_presentation_v1","mediaTimeSeconds":2.,
            "sourceTimeSeconds":2.,"playbackRate":1.,"sourceId":"44444444-4444-4444-8444-444444444444"})).unwrap()
    }
    #[test]
    fn direct_projection_keeps_source_generation_and_never_accepts_unknown_or_bad_clocks() {
        let mut c=clock();let p=direct(Some(&c)).unwrap();assert_eq!(p.ticks90k,180000);assert_eq!(p.generation,3);
        assert!(direct(None).is_err());
        c.clock="player_observation".into();assert!(direct(Some(&c)).is_err());
        c=clock();c.source_time_seconds=Some(f64::NAN);assert!(direct(Some(&c)).is_err());
        c=clock();c.media_time_seconds=Some(3.);assert!(direct(Some(&c)).is_err());
        c=clock();c.source_id=Some(uuid::Uuid::nil().to_string());assert!(direct(Some(&c)).is_err());
        c=clock();c.source_generation=0;assert!(direct(Some(&c)).is_err());
    }
}
