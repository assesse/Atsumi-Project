//! Exact public packet identity observations from a pinned SOOP timing ledger.
//! This does not infer source timestamps from receive time or remote video time.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct EventRow {
    message_id_sha256:String,service:u16,observation_sequence:u64,received_monotonic_ns:u64,
    sample:SoopSample,live_visible:Option<bool>,pub(super) usable:bool,
    quality:String,presentation_uncertainty:String,archive_mapping:(),
}
impl EventRow {
    fn validate(&self)->Result<()> {
        require(hex(&self.message_id_sha256)&&matches!(self.service,5|18|33|91|93|105|109|121)
            &&self.observation_sequence<=9_007_199_254_740_991&&self.quality=="public-receiver-dispatch"
            &&self.presentation_uncertainty=="unknown",TimingReadError::Schema)?;
        let s=&self.sample;valid_clock(s.source_time_seconds,s.current_time_seconds,s.timestamp_offset_seconds,s.performance_now_ms)?;
        require(s.playback_rate.is_finite()&&(0.0625..=16.).contains(&s.playback_rate)&&s.ready_state<=4
            &&self.usable==(self.live_visible==Some(true)&&!s.paused&&!s.seeking&&s.ready_state>=2),TimingReadError::Clock)
    }
}
#[derive(Default)]
pub(super) struct ReadState {sequence:Option<u64>,received:Option<u64>,page:Option<f64>}
impl ReadState {
    pub(super) fn observe(&mut self,row:&EventRow)->Result<bool> {
        row.validate()?;
        require(self.sequence.is_none_or(|v|row.observation_sequence>v)
            &&self.received.is_none_or(|v|row.received_monotonic_ns>=v)
            &&self.page.is_none_or(|v|row.sample.performance_now_ms>=v),TimingReadError::Order)?;
        let gap=self.sequence.map_or(row.observation_sequence!=0,|v|v.checked_add(1)!=Some(row.observation_sequence));
        self.sequence=Some(row.observation_sequence);self.received=Some(row.received_monotonic_ns);self.page=Some(row.sample.performance_now_ms);
        Ok(gap)
    }
}
#[derive(Clone,Debug,PartialEq)]
pub(crate) enum EventPosition {
    Unknown(&'static str),
    Source {ticks90k:u64,service:u16,observation_sequence:u64},
}
pub(super) fn hash(value:&str)->Result<[u8;32]> {
    require(hex(value),TimingReadError::Hash)?;let mut bytes=[0;32];
    for (i,b) in bytes.iter_mut().enumerate(){*b=u8::from_str_radix(&value[2*i..2*i+2],16).map_err(|_|TimingReadError::Hash)?;}
    Ok(bytes)
}
impl EventRow {
    pub(super) fn index_parts(&self)->Result<([u8;32],Option<u64>,u16,u64)> {
        self.validate()?;
        let ticks=self.sample.source_time_seconds*90_000.;
        require(ticks.is_finite()&&(0.0..=90_000_000_000_000.).contains(&ticks),TimingReadError::Clock)?;
        Ok((hash(&self.message_id_sha256)?,self.usable.then_some(ticks.round() as u64),self.service,self.observation_sequence))
    }
}
impl RelayTimingLease {
    /// Accept only normalized digests from the caller's held SOOP history.
    /// The caller must keep that history lease alive; no public IPC accepts IDs.
    pub(crate) fn soop_event_clocks(&self,ids:&[Option<&str>],cancel:&AtomicBool)->Result<Vec<EventPosition>> {
        cancelled(cancel)?;require(self.summary.dialect==Dialect::Soop,TimingReadError::Identity)?;
        require(ids.len()<=120,TimingReadError::Limit)?;
        if ids.is_empty(){return Ok(Vec::new());}
        self.ensure_index(cancel)?.events(ids,cancel)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn event()->EventRow {serde_json::from_value(serde_json::json!({"messageIdSha256":"ab".repeat(32),"service":5,"observationSequence":0,
        "receivedMonotonicNs":100,"sample":{"sourceTimeSeconds":12.,"currentTimeSeconds":14.,"timestampOffsetSeconds":2.,"performanceNowMs":1000.,
        "playbackRate":1.,"paused":false,"seeking":false,"readyState":4},"liveVisible":true,"usable":true,
        "quality":"public-receiver-dispatch","presentationUncertainty":"unknown","archiveMapping":null})).unwrap()}
    #[test]
    fn strict_reader_rejects_fake_usable_private_service_and_backward_sequence() {
        let mut row=event();let mut state=ReadState::default();assert_eq!(state.observe(&row).unwrap(),false);
        assert!(state.observe(&row).is_err());row.observation_sequence=2;assert_eq!(state.observe(&row).unwrap(),true);
        row.sample.paused=true;assert!(row.validate().is_err());row.usable=false;assert!(row.validate().is_ok());
        row.service=9;assert!(row.validate().is_err());row.service=5;row.live_visible=None;row.usable=true;assert!(row.validate().is_err());
    }
}
