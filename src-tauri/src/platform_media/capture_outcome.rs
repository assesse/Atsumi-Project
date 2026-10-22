//! Truthful recording outcome, independent of a successfully published file.
//! This bounded DTO carries no source, publication, or cleanup authority.
use super::youtube_spool::{PublicationSourceFacts, PublicationState, SealCause};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;

const ERROR: &str = "녹화 종료 정보를 확인하지 못했습니다. 원본은 보존했습니다.";
const MAX_UNITS: u64 = 100_000;
const MAX_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const MAX_PACKETS: u64 = 20_000_000;
const MAX_REASONS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Complete,
    Partial,
    Interrupted,
}
impl<'de> Deserialize<'de> for State {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "complete" => Ok(Self::Complete),
            "partial" => Ok(Self::Partial),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(de::Error::custom(ERROR)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    SourceInterrupted,
    Cancelled,
    Limit,
    FormatBoundary,
    TornJournal,
    UncommittedBytes,
    IncompleteAppend,
    IncompleteStructure,
    StartupCoverage,
    AudioCoverageTail,
}
impl Reason {
    fn interrupted(self) -> bool {
        matches!(self, Self::SourceInterrupted | Self::Cancelled | Self::Limit)
    }
}
impl<'de> Deserialize<'de> for Reason {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "source_interrupted" => Ok(Self::SourceInterrupted),
            "cancelled" => Ok(Self::Cancelled),
            "limit" => Ok(Self::Limit),
            "format_boundary" => Ok(Self::FormatBoundary),
            "torn_journal" => Ok(Self::TornJournal),
            "uncommitted_bytes" => Ok(Self::UncommittedBytes),
            "incomplete_append" => Ok(Self::IncompleteAppend),
            "incomplete_structure" => Ok(Self::IncompleteStructure),
            "startup_coverage" => Ok(Self::StartupCoverage),
            "audio_coverage_tail" => Ok(Self::AudioCoverageTail),
            _ => Err(de::Error::custom(ERROR)),
        }
    }
}

fn reasons<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Reason>, D::Error> {
    struct Reasons;
    impl<'de> de::Visitor<'de> for Reasons {
        type Value = Vec<Reason>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("at most ten distinct recording reason strings")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(reason) = sequence.next_element()? {
                if result.len() >= MAX_REASONS || result.contains(&reason) {
                    return Err(de::Error::custom(ERROR));
                }
                result.push(reason);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_seq(Reasons)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOutcome {
    state: State,
    reasons: Vec<Reason>,
    excluded_units: [u64; 2],
    excluded_bytes: [u64; 2],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stored {
    state: State,
    #[serde(deserialize_with = "reasons")]
    reasons: Vec<Reason>,
    excluded_units: [u64; 2],
    excluded_bytes: [u64; 2],
}
impl<'de> Deserialize<'de> for CaptureOutcome {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = Stored::deserialize(deserializer)?;
        let value = Self {
            state: stored.state,
            reasons: stored.reasons,
            excluded_units: stored.excluded_units,
            excluded_bytes: stored.excluded_bytes,
        };
        value.validate().map_err(de::Error::custom)?;
        Ok(value)
    }
}
impl CaptureOutcome {
    /// The stop proof grants reporting only, never publication/cleanup authority.
    pub(super) fn soop_interrupted() -> Self {
        Self {
            state: State::Interrupted,
            reasons: vec![Reason::SourceInterrupted],
            excluded_units: [0; 2],
            excluded_bytes: [0; 2],
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self.reasons.len() > MAX_REASONS
            || self.excluded_units.iter().any(|n| *n > MAX_UNITS)
            || self.excluded_bytes[0]
                .checked_add(self.excluded_bytes[1])
                .is_none_or(|n| n > MAX_BYTES)
            || self.excluded_units.iter().zip(self.excluded_bytes)
                .any(|(units, bytes)| *units != 0 && bytes == 0)
            || self.reasons.iter().enumerate()
                .any(|(at, reason)| self.reasons[..at].contains(reason))
        {
            return Err(ERROR.into());
        }
        let interrupted = self.reasons.iter().any(|reason| reason.interrupted());
        let consistent = match self.state {
            State::Complete => self.reasons.is_empty()
                && self.excluded_units[0] == 0 && self.excluded_bytes[0] == 0,
            State::Partial => !interrupted && !self.reasons.is_empty(),
            State::Interrupted => interrupted,
        };
        if consistent { Ok(()) } else { Err(ERROR.into()) }
    }
}

/// Only the native publication service uses this reporting projection. Its
/// input is still a snapshot, not a capability to install or delete any file.
pub(super) fn from_source(source: &PublicationSourceFacts) -> Result<CaptureOutcome, String> {
    let mut result = CaptureOutcome {
        state: State::Complete,
        reasons: Vec::new(),
        excluded_units: [0; 2],
        excluded_bytes: [0; 2],
    };
    let mut add = |reason: Reason| {
        if !result.reasons.contains(&reason) { result.reasons.push(reason); }
    };
    if source.state == PublicationState::Interrupted || source.seal_cause.is_none() {
        add(Reason::SourceInterrupted);
    }
    match source.seal_cause {
        Some(SealCause::Cancelled) => add(Reason::Cancelled),
        Some(SealCause::Limit) => add(Reason::Limit),
        Some(SealCause::Interrupted) => add(Reason::SourceInterrupted),
        Some(SealCause::FormatBoundary) => add(Reason::FormatBoundary),
        Some(SealCause::Stopped) | None => (),
    }
    if source.torn_journal { add(Reason::TornJournal); }
    if source.uncommitted_bytes.iter().any(|n| *n != 0) { add(Reason::UncommittedBytes); }
    if source.incomplete_append { add(Reason::IncompleteAppend); }
    if source.partial_tail.iter().any(|partial| *partial) { add(Reason::IncompleteStructure); }
    for at in 0..2 {
        for exclusion in [source.leading[at], source.trailing[at]] {
            result.excluded_units[at] = result.excluded_units[at]
                .checked_add(exclusion.units).ok_or(ERROR)?;
            result.excluded_bytes[at] = result.excluded_bytes[at]
                .checked_add(exclusion.bytes).ok_or(ERROR)?;
            if exclusion.startup_coverage { add(Reason::StartupCoverage); }
            if exclusion.audio_coverage_tail { add(Reason::AudioCoverageTail); }
            if exclusion.incomplete_structure { add(Reason::IncompleteStructure); }
            if exclusion.incomplete_append { add(Reason::IncompleteAppend); }
            // outside_selected_video describes expected complete AAC padding.
            // Packet-boundary trimming and selected=true alone are not losses.
        }
        result.excluded_bytes[at] = result.excluded_bytes[at]
            .checked_add(source.uncommitted_bytes[at]).ok_or(ERROR)?;
    }
    if source.audio_excluded_packets[0].checked_add(source.audio_excluded_packets[1])
        .is_none_or(|n| n > MAX_PACKETS)
    { return Err(ERROR.into()); }
    result.state = if result.reasons.iter().any(|reason| reason.interrupted()) {
        State::Interrupted
    } else if !result.reasons.is_empty() || result.excluded_units[0] != 0 || result.excluded_bytes[0] != 0 {
        State::Partial
    } else {
        State::Complete
    };
    // Unexplained video exclusion cannot fall back to normal or invent a reason.
    result.validate()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_media::youtube_spool::PublicationExclusion;
    use serde_json::{json, Value};

    fn source() -> PublicationSourceFacts {
        PublicationSourceFacts {
            state: PublicationState::Sealed, seal_cause: Some(SealCause::Stopped),
            selected: false, incomplete_append: false, torn_journal: false,
            uncommitted_bytes: [0; 2], partial_tail: [false; 2],
            leading: [PublicationExclusion::default(); 2],
            trailing: [PublicationExclusion::default(); 2], audio_excluded_packets: [0; 2],
        }
    }
    fn roundtrip(value: &CaptureOutcome) {
        let bytes = serde_json::to_vec(value).unwrap();
        assert_eq!(serde_json::from_slice::<CaptureOutcome>(&bytes).unwrap(), *value);
    }
    fn rejected(value: Value) {
        assert!(serde_json::from_value::<CaptureOutcome>(value).is_err());
    }

    #[test]
    fn whole_and_selected_complete_are_reporting_only() {
        for selected in [false, true] {
            let mut facts = source(); facts.selected = selected;
            let value = from_source(&facts).unwrap();
            assert_eq!(value.state, State::Complete);
            assert!(value.reasons.is_empty());
            assert_eq!(serde_json::to_value(&value).unwrap(), json!({
                "state":"complete","reasons":[],"excludedUnits":[0,0],"excludedBytes":[0,0]
            }));
            roundtrip(&value);
        }
    }

    #[test]
    fn audio_outside_video_and_packet_boundary_trim_do_not_invent_loss() {
        let mut facts = source(); facts.selected = true;
        facts.leading[1] = PublicationExclusion { bytes: 12, units: 1, outside_selected_video: true, ..Default::default() };
        facts.trailing[1] = PublicationExclusion { bytes: 20, units: 2, outside_selected_video: true, ..Default::default() };
        facts.audio_excluded_packets = [7, 11];
        let value = from_source(&facts).unwrap();
        assert_eq!(value.state, State::Complete);
        assert_eq!(value.excluded_units, [0, 3]);
        assert_eq!(value.excluded_bytes, [0, 32]);
        roundtrip(&value);
    }

    #[test]
    fn video_exclusions_keep_exact_counts_and_deduplicate_reasons() {
        let mut facts = source(); facts.selected = true;
        facts.leading[0] = PublicationExclusion { bytes: 100, units: 1, startup_coverage: true, incomplete_append: true, ..Default::default() };
        facts.trailing[0] = PublicationExclusion { bytes: 210, units: 2, audio_coverage_tail: true, incomplete_append: true, ..Default::default() };
        facts.incomplete_append = true;
        let value = from_source(&facts).unwrap();
        assert_eq!(value.state, State::Partial);
        assert_eq!(value.excluded_units, [3, 0]);
        assert_eq!(value.excluded_bytes, [310, 0]);
        assert_eq!(value.reasons, [Reason::IncompleteAppend, Reason::StartupCoverage, Reason::AudioCoverageTail]);
        roundtrip(&value);
    }

    #[test]
    fn every_partial_provenance_stays_non_normal() {
        for case in 0..6 {
            let mut facts = source();
            let expected = match case {
                0 => { facts.torn_journal = true; Reason::TornJournal },
                1 => { facts.uncommitted_bytes = [1, 2]; Reason::UncommittedBytes },
                2 => { facts.incomplete_append = true; Reason::IncompleteAppend },
                3 => { facts.partial_tail[1] = true; Reason::IncompleteStructure },
                4 => { facts.seal_cause = Some(SealCause::FormatBoundary); Reason::FormatBoundary },
                _ => { facts.trailing[1].incomplete_structure = true; Reason::IncompleteStructure },
            };
            let value = from_source(&facts).unwrap();
            assert_eq!(value.state, State::Partial);
            assert_eq!(value.reasons, [expected]);
            assert_eq!(value.excluded_bytes, facts.uncommitted_bytes);
            roundtrip(&value);
        }
    }

    #[test]
    fn interruption_precedes_partial_and_preserves_all_reasons() {
        for cause in [None, Some(SealCause::Interrupted), Some(SealCause::Cancelled), Some(SealCause::Limit)] {
            let mut facts = source(); facts.seal_cause = cause;
            facts.incomplete_append = true; facts.partial_tail[0] = true;
            let value = from_source(&facts).unwrap();
            assert_eq!(value.state, State::Interrupted);
            assert!(value.reasons.contains(&Reason::IncompleteAppend));
            assert!(value.reasons.contains(&Reason::IncompleteStructure));
            roundtrip(&value);
        }
        let mut facts = source(); facts.state = PublicationState::Interrupted;
        assert_eq!(from_source(&facts).unwrap().state, State::Interrupted);
    }

    #[test]
    fn counts_are_checked_once_with_fixed_combined_limits() {
        let mut facts = source(); facts.selected = true;
        facts.leading[0] = PublicationExclusion { bytes: 10, units: 1, startup_coverage: true, incomplete_structure: true, ..Default::default() };
        facts.uncommitted_bytes = [3, 4];
        assert_eq!(from_source(&facts).unwrap().excluded_bytes, [13, 4]);
        facts.leading[0].units = MAX_UNITS;
        facts.trailing[0].units = 1; facts.trailing[0].bytes = 1;
        assert!(from_source(&facts).is_err());
        let mut facts = source(); facts.uncommitted_bytes = [MAX_BYTES, 0];
        roundtrip(&from_source(&facts).unwrap());
        facts.uncommitted_bytes[1] = 1;
        assert!(from_source(&facts).is_err());
        facts.uncommitted_bytes = [u64::MAX, 1];
        assert!(from_source(&facts).is_err());
        let mut facts = source(); facts.leading[0].bytes = u64::MAX; facts.trailing[0].bytes = 1;
        assert!(from_source(&facts).is_err());
        let mut facts = source(); facts.audio_excluded_packets = [MAX_PACKETS, 1];
        assert!(from_source(&facts).is_err());
    }

    #[test]
    fn unknown_duplicate_and_non_string_fields_are_not_normal() {
        let base = serde_json::to_value(from_source(&source()).unwrap()).unwrap();
        for (key, replacement) in [
            ("state", json!("new_state")), ("state", json!({"complete":null})),
            ("reasons", json!(["unknown"])), ("reasons", json!([{"limit":null}])),
            ("reasons", json!(["limit","limit"])), ("reasons", json!(null)),
            ("excludedBytes", json!([0])), ("excludedBytes", json!([0,0,0])),
            ("excludedUnits", json!([-1,0])), ("excludedUnits", json!([MAX_UNITS+1,0])),
            ("excludedBytes", json!([MAX_BYTES,1])), ("extra", json!(false)),
        ] {
            let mut value = base.clone();
            value[key] = replacement;
            rejected(value);
        }
        let mut missing = base;
        missing.as_object_mut().unwrap().remove("reasons");
        rejected(missing);
        assert!(serde_json::from_str::<CaptureOutcome>(
            r#"{"state":"complete","state":"complete","reasons":[],"excludedUnits":[0,0],"excludedBytes":[0,0]}"#
        ).is_err());
    }

    #[test]
    fn complete_partial_and_interrupted_schema_cannot_be_interchanged() {
        let base = serde_json::to_value(from_source(&source()).unwrap()).unwrap();
        let mut value = base.clone(); value["reasons"] = json!(["torn_journal"]); rejected(value);
        let mut value = base.clone(); value["excludedBytes"] = json!([1,0]); rejected(value);
        let mut value = base.clone(); value["excludedUnits"] = json!([0,1]); rejected(value);
        for state in ["partial", "interrupted"] {
            let mut value = base.clone(); value["state"] = json!(state); rejected(value);
        }
        let mut value = base.clone(); value["state"] = json!("partial"); value["reasons"] = json!(["limit"]); rejected(value);
        let mut value = base; value["state"] = json!("interrupted"); value["reasons"] = json!(["torn_journal"]); rejected(value);
        let mut facts = source(); facts.leading[0].bytes = 1;
        assert!(from_source(&facts).is_err(), "unknown video exclusion must not invent an explanation");
    }
}
