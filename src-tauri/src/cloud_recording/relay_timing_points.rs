//! Bounded SQL candidates inside the already verified, pinned ACK ledger.
//! This is not a proof that every time within a segment envelope has a frame.
use super::*;

// Native source timescale/origin and stream identity remain in the held summary.
// SQL stores native ordinals explicitly; only at most three candidates are read.
pub(super) struct VideoAckRange {
    pub(super) source_min: u64,
    pub(super) source_end: u64,
    pub(super) transport_min: u64,
    pub(super) transport_end: u64,
    pub(super) sha256: [u8; 32],
}

#[derive(Clone, Debug)]
pub(crate) struct AcknowledgedSourcePoint {
    pub ordinal: u64,
    pub segment_sha256: String,
    pub transport_ticks90k: u64,
}

impl RelayTimingLease {
    /// One tick of input quantization must fit inside an acknowledged envelope.
    /// Presentation uncertainty remains separate from this numeric admission.
    pub(crate) fn map_video_observations(&self, points90k:&[u64], cancel:&AtomicBool)->Result<Vec<Option<AcknowledgedSourcePoint>>> {
        cancelled(cancel)?;
        require(points90k.len()<=120,TimingReadError::Limit)?;
        require(points90k.iter().all(|p|*p<=90_000_000_000_000),TimingReadError::Clock)?;
        if points90k.is_empty(){return Ok(Vec::new());}
        if self.summary.prefix_segments==0{return Ok(vec![None;points90k.len()]);}
        self.ensure_index(cancel)?.points(&self.summary,points90k,cancel)
    }
}

pub(super) fn decode_sha(value: &str) -> Result<[u8; 32]> {
    require(hex(value), TimingReadError::Hash)?;
    let mut bytes = [0;32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index*2..index*2+2],16).map_err(|_| TimingReadError::Hash)?;
    }
    Ok(bytes)
}
fn sha_text(bytes: &[u8;32]) -> String {
    const HEX: &[u8;16] = b"0123456789abcdef";
    let mut text=String::with_capacity(64);
    for byte in bytes { text.push(HEX[usize::from(byte>>4)] as char);text.push(HEX[usize::from(byte&15)] as char); }
    text
}
pub(super) fn point_in_range(range: &VideoAckRange, scale: u32, ordinal: u64, source90k: u64, transport: u64) -> Option<AcknowledgedSourcePoint> {
    let Some(lower) = source90k.checked_sub(1) else { return None; };
    let upper = source90k.checked_add(1)?;
    let scale = u128::from(scale);
    if u128::from(lower) * scale < u128::from(range.source_min) * 90_000
        || u128::from(upper) * scale >= u128::from(range.source_end) * 90_000
        || transport < range.transport_min || transport >= range.transport_end {
        return None;
    }
    Some(AcknowledgedSourcePoint { ordinal, segment_sha256: sha_text(&range.sha256), transport_ticks90k: transport })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn range(start: u64, end: u64) -> VideoAckRange {
        VideoAckRange {source_min:start,source_end:end,transport_min:start-90_000,transport_end:end-90_000,sha256:[0xab;32]}
    }
    #[test]
    fn point_uses_acknowledged_source_origin_and_preserves_provenance() {
        let p = point_in_range(&range(90_000,270_000),90_000,4,180_000,90_000).unwrap();
        assert_eq!(p.ordinal,4); assert_eq!(p.transport_ticks90k,90_000);
        assert_eq!(p.segment_sha256,"ab".repeat(32));
    }
    #[test]
    fn segment_envelopes_do_not_bridge_a_gap_or_round_across_boundaries() {
        let a=range(90_000,180_000); let b=range(270_000,360_000);
        for p in [180_000,225_000,270_000] {
            assert!(point_in_range(&a,90_000,0,p,p-90_000).is_none());
            assert!(point_in_range(&b,90_000,1,p,p-90_000).is_none());
        }
        assert!(point_in_range(&a,90_000,0,90_000,0).is_none());
        assert!(point_in_range(&a,90_000,0,90_001,1).is_some());
        assert!(point_in_range(&a,90_000,0,179_999,89_999).is_none());
        assert!(point_in_range(&a,90_000,0,179_998,89_998).is_some());
    }
}
