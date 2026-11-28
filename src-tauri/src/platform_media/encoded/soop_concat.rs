//! Source-clock concat for one verified SOOP encoder epoch. No media rewrite.
use super::*;
use std::io::Read;

const MAX_INIT_BYTES: usize = 2 * 64 * 1024 + 1024;
const CLOCK_ERROR: &str = "녹화 조각의 원본 시각이나 영상 설정이 일치하지 않습니다. 원본은 보존했습니다.";

pub(in crate::platform_media) struct Plan {
    pub durations_us: Vec<u64>,
    pub expected_seconds: f64,
    // Windows permits the decoder to read, but denies writes/deletes for the
    // entire preflight+merge lifetime. This is not a cross-platform lock claim.
    _pins: Vec<fs::File>,
}

fn pin(path: &Path) -> Result<fs::File, String> {
    storage::regular(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    options.open(path).map_err(|_| CLOCK_ERROR.into())
}

fn init(file: &mut impl Read) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    for kind in [b"ftyp", b"moov"] {
        let mut header = [0_u8; 8];
        file.read_exact(&mut header).map_err(|_| CLOCK_ERROR)?;
        let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        if &header[4..] != kind || size < 8 || size > MAX_INIT_BYTES
            || result.len().checked_add(size).is_none_or(|n| n > MAX_INIT_BYTES)
        { return Err(CLOCK_ERROR.into()); }
        let start = result.len();
        result.extend(header);
        result.resize(start + size, 0);
        file.read_exact(&mut result[start + 8..]).map_err(|_| CLOCK_ERROR)?;
    }
    Ok(result)
}

fn timeline(parts: &[SegmentProof]) -> Result<(Vec<u64>, f64), String> {
    let first = parts.first().ok_or(CLOCK_ERROR)?.source_start;
    if !first.is_finite() || first < 0.0 { return Err(CLOCK_ERROR.into()); }
    let mut offsets = Vec::with_capacity(parts.len() + 1);
    for (index, part) in parts.iter().enumerate() {
        if segment_sequence(&part.name) != Some(index as u64)
            || !part.source_start.is_finite() || !part.source_end.is_finite()
            || part.source_start < first || part.source_end <= part.source_start
            || !part.duration.is_finite() || !(0.0..=120.0).contains(&part.duration)
            || part.duration == 0.0
            // The concat clock itself has microsecond resolution. Check the
            // native duration in that same grid, without per-piece accumulation.
            || ((part.source_end - part.source_start) * 1_000_000.0).round()
                != (part.duration * 1_000_000.0).round()
        { return Err(CLOCK_ERROR.into()); }
        let offset = ((part.source_start - first) * 1_000_000.0).round();
        if !offset.is_finite() || offset < 0.0 || offset >= u64::MAX as f64 {
            return Err(CLOCK_ERROR.into());
        }
        offsets.push(offset as u64);
    }
    let end = ((parts.last().unwrap().source_end - first) * 1_000_000.0).round();
    if !end.is_finite() || end < 0.0 || end >= u64::MAX as f64 { return Err(CLOCK_ERROR.into()); }
    offsets.push(end as u64);
    let durations: Vec<_> = offsets.windows(2).map(|pair| pair[1].checked_sub(pair[0]))
        .collect::<Option<_>>().ok_or(CLOCK_ERROR)?;
    if durations.iter().any(|n| *n == 0 || *n > 120_000_000) { return Err(CLOCK_ERROR.into()); }
    Ok((durations, end / 1_000_000.0))
}

pub(in crate::platform_media) fn prepare(
    service: &PlatformMediaService, job: &MediaJob, directory: &Path,
    cancel: &Arc<AtomicBool>, segments: &[(String, f64)], reserve: u64,
) -> Result<Plan, String> {
    owner(service, job, cancel)?;
    if directory != storage::job_dir(&job.download_root, "soop", &job.job_id)?
        || directory.join("media.mp4").exists() { return Err(CLOCK_ERROR.into()); }
    let mut manifest = pin(&directory.join(MANIFEST))?;
    if manifest.metadata().map_err(|_| CLOCK_ERROR)?.len() > 32 * 1024 * 1024 {
        return Err(CLOCK_ERROR.into());
    }
    let mut text = String::new();
    Read::by_ref(&mut manifest).take(32 * 1024 * 1024 + 1).read_to_string(&mut text).map_err(|_| CLOCK_ERROR)?;
    if text.len() > 32 * 1024 * 1024 { return Err(CLOCK_ERROR.into()); }
    let mut parts = Vec::new();
    for line in text.lines() {
        if parts.len() >= MAX_SEGMENTS || cancel.load(Ordering::Acquire) { return Err(CLOCK_ERROR.into()); }
        parts.push(serde_json::from_str::<SegmentProof>(line).map_err(|_| CLOCK_ERROR)?);
    }
    if !same_segments(&parts.iter().map(|p| (p.name.clone(), p.duration)).collect::<Vec<_>>(), segments) {
        return Err(CLOCK_ERROR.into());
    }
    let (durations_us, expected_seconds) = timeline(&parts)?;
    let mut pins = vec![manifest];
    let mut common_init = None;
    for part in &parts {
        if cancel.load(Ordering::Acquire) { return Err("cancelled".into()); }
        let mut file = pin(&directory.join(&part.name))?;
        if file.metadata().map_err(|_| CLOCK_ERROR)?.len() != part.bytes { return Err(CLOCK_ERROR.into()); }
        let header = init(&mut file)?;
        if common_init.as_ref().is_some_and(|known| known != &header) { return Err(CLOCK_ERROR.into()); }
        common_init.get_or_insert(header);
        pins.push(file);
    }
    // Hash/proof/outcome/ownership checks happen while exact input names are
    // pinned. Auto conversion is disabled only after this succeeds.
    preflight(service, job, directory, cancel, segments, reserve)?;
    Ok(Plan { durations_us, expected_seconds, _pins: pins })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn part(index: usize, start: f64, end: f64) -> SegmentProof {
        SegmentProof { name: format!("e{index:08}.mp4"), duration:end-start,
            source_start:start, source_end:end, bytes:1, sha256:"unused".into() }
    }
    #[test]
    fn boundary_offsets_do_not_sum_container_padding() {
        let parts = [part(0,12.041,24.047),part(1,24.047,36.05766666666667),part(2,36.053,46.01033333333333)];
        let (durations, expected) = timeline(&parts).unwrap();
        assert_eq!(durations, [12_006_000,12_006_000,9_957_333]);
        assert_eq!(expected,33.969333);
    }
    #[test]
    fn absolute_microsecond_rounding_does_not_accumulate() {
        let parts:Vec<_>=(0..1000).map(|i|part(i,i as f64/3.0,(i+1) as f64/3.0)).collect();
        let (durations,expected)=timeline(&parts).unwrap();
        assert_eq!(durations.iter().sum::<u64>(),333_333_333);
        assert_eq!(expected,333.333333);
        assert!(durations.contains(&333_334));
    }
    #[test]
    fn gaps_remain_source_gaps_and_invalid_clocks_fail() {
        assert_eq!(timeline(&[part(0,1.0,2.0),part(1,3.0,4.0)]).unwrap().0,[2_000_000,1_000_000]);
        for value in [f64::NAN,f64::INFINITY,-1.0,1.0,1000.0] {
            assert!(timeline(&[part(0,1.0,2.0),part(1,value,value+1.0)]).is_err());
        }
        assert!(timeline(&[]).is_err());
        let mut bad = part(0,0.0,1.0);bad.duration=1.01;
        assert!(timeline(&[bad]).is_err());
    }
    #[test]
    fn init_reader_is_bounded_and_keeps_codec_bytes() {
        let bytes=[0,0,0,12,b'f',b't',b'y',b'p',1,2,3,4,0,0,0,9,b'm',b'o',b'o',b'v',9];
        assert_eq!(init(&mut &bytes[..]).unwrap(),bytes);
        assert!(init(&mut &bytes[..20]).is_err());
        let mut invalid=bytes;invalid[4]=b'x';assert!(init(&mut &invalid[..]).is_err());
        invalid=bytes;invalid[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(init(&mut &invalid[..]).is_err());
    }
}

#[cfg(test)]
#[path = "soop_concat_tests.rs"]
mod native_tests;
