//! Exact same decoded-packet hash ties RAM pixels to demux PTS; never wall time.
use super::{feature, process, FrameReference, ReferenceWindow, Segment, WIDTH, HEIGHT};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::atomic::AtomicBool};

const MAX_FRAMES: usize = 600;
const FRAME_BYTES: usize = WIDTH*HEIGHT;
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct Timing { origin_ticks:u64, origin_timescale:u32, video:Track }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct Track { timescale:u32, min_pts90k:u64, max_pts_end90k:u64 }
#[derive(Debug)]
struct Row { pts90k:u64, hash:String }
fn hex(bytes:&[u8])->String { Sha256::digest(bytes).iter().map(|b|format!("{b:02x}")).collect() }

fn args(raw:bool)->Vec<&'static str> {
    let mut args=vec!["-hide_banner","-nostdin","-loglevel","error","-xerror","-err_detect","explode",
        "-protocol_whitelist","pipe","-f","mpegts","-i","pipe:0","-map","0:v:0","-an",
        "-vf","scale=32:18:flags=area,format=gray","-c:v","rawvideo","-copyts",
        "-fps_mode:v","passthrough","-enc_time_base:v","demux","-threads","1","-filter_threads","1"];
    if raw { args.extend(["-f","rawvideo","pipe:1"]); }
    else { args.extend(["-f","framehash","-hash","sha256","pipe:1"]); }
    args
}
pub(super) fn window(exe:&Path,segment:&Segment,cancel:&AtomicBool)->Result<ReferenceWindow,String> {
    let cwd=exe.parent().ok_or("frame_reference_tools")?;
    let timing:Timing=serde_json::from_value(serde_json::to_value(segment.timing).map_err(|_|"frame_reference_timing")?).map_err(|_|"frame_reference_timing")?;
    if timing.origin_timescale==0 || timing.video.timescale==0 || timing.video.min_pts90k>=timing.video.max_pts_end90k { return Err("frame_reference_timing".into()); }
    let hashed=process::run(exe,cwd,&args(false),Some(segment.bytes.clone()),128*1024,cancel)?;
    if hashed.facts["success"]!=true || hashed.facts["stderrRetainedBytes"]!=0 { return Err("frame_reference_decode".into()); }
    let rows=parse(&hashed.bytes,&timing)?;
    let pixels=process::run(exe,cwd,&args(true),Some(segment.bytes.clone()),MAX_FRAMES*FRAME_BYTES,cancel)?;
    if pixels.facts["success"]!=true || pixels.facts["stderrRetainedBytes"]!=0 { return Err("frame_reference_decode".into()); }
    let frames=bind(&rows,&pixels.bytes)?;
    Ok(ReferenceWindow {ordinal:segment.ordinal,segment_sha256:hex(&segment.bytes),source_origin_ticks:timing.origin_ticks,
        source_origin_timescale:timing.origin_timescale,source_video_timescale:timing.video.timescale,frames})
}
fn parse(bytes:&[u8],timing:&Timing)->Result<Vec<Row>,String> {
    let text=std::str::from_utf8(bytes).map_err(|_|"frame_reference_decode_text")?;
    if !text.ends_with('\n') { return Err("frame_reference_partial".into()); }
    let mut timebase=None; let mut rows:Vec<Row>=Vec::new();
    for line in text.lines() {
        if let Some(rest)=line.strip_prefix("#tb ") {
            if timebase.is_some() { return Err("frame_reference_timebase".into()); }
            let (index,ratio)=rest.split_once(':').ok_or("frame_reference_timebase")?;
            let (n,d)=ratio.trim().split_once('/').ok_or("frame_reference_timebase")?;
            let (n,d)=(n.parse::<u64>().map_err(|_|"frame_reference_timebase")?,d.parse::<u64>().map_err(|_|"frame_reference_timebase")?);
            if index.trim()!="0" || n==0 || d==0 || n>1_000_000 || d>1_000_000_000 { return Err("frame_reference_timebase".into()); }
            timebase=Some((n,d));
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            let f:Vec<_>=line.split(',').map(str::trim).collect();
            if f.len()!=6 || f[0]!="0" || rows.len()>=MAX_FRAMES || f[4].parse::<usize>().ok()!=Some(FRAME_BYTES)
                || f[5].len()!=64 || !f[5].bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) { return Err("frame_reference_row".into()); }
            let p=f[2].parse::<u64>().map_err(|_|"frame_reference_pts")?;
            let duration=f[3].parse::<u64>().map_err(|_|"frame_reference_duration")?;
            if duration==0 { return Err("frame_reference_duration".into()); }
            let (n,d)=timebase.ok_or("frame_reference_timebase")?;
            let scaled=u128::from(p)*u128::from(n)*90_000;
            // Native TS PTS is an integer90k coordinate. No invented fractional
            // timebase, start_at_zero, or wall-clock correction is admitted.
            if scaled%u128::from(d)!=0 { return Err("frame_reference_pts".into()); }
            let pts90k=u64::try_from(scaled/u128::from(d)).map_err(|_|"frame_reference_pts")?;
            if pts90k<timing.video.min_pts90k || pts90k>=timing.video.max_pts_end90k
                || rows.last().is_some_and(|r|r.pts90k>=pts90k) { return Err("frame_reference_pts".into()); }
            rows.push(Row {pts90k,hash:f[5].into()});
        }
    }
    if rows.len()<8 {return Err("frame_reference_empty".into());} Ok(rows)
}
fn bind(rows:&[Row],pixels:&[u8])->Result<Vec<FrameReference>,String> {
    if rows.len()>MAX_FRAMES || pixels.len()!=rows.len()*FRAME_BYTES { return Err("frame_reference_pixel_count".into()); }
    let mut result:Vec<FrameReference>=Vec::new();
    for (row,pixel) in rows.iter().zip(pixels.chunks_exact(FRAME_BYTES)) {
        if hex(pixel)!=row.hash { return Err("frame_reference_pixel_hash".into()); }
        //12samples/sec retains dynamic sequences without retaining images.
        if result.last().is_some_and(|old|row.pts90k-old.pts90k<7500) { continue; }
        result.push(FrameReference {pts90k:row.pts90k,feature:feature(pixel)?});
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn exact_raw_hash_binds_clock_and_rejects_crossed_passes() {
        let mut pixels=vec![0u8;FRAME_BYTES*8]; for (i,p) in pixels.iter_mut().enumerate(){*p=(i/FRAME_BYTES*17)as u8;}
        let rows:Vec<_>=pixels.chunks_exact(FRAME_BYTES).enumerate().map(|(i,p)|Row{pts90k:i as u64*9000,hash:hex(p)}).collect();
        assert_eq!(bind(&rows,&pixels).unwrap().len(),8);
        pixels[FRAME_BYTES+1]^=1;assert!(bind(&rows,&pixels).is_err());assert!(bind(&rows,&pixels[..FRAME_BYTES]).is_err());
    }
    #[test] fn timestamps_cannot_be_reset_or_inferred_outside_native_range() {
        let timing=Timing{origin_ticks:0,origin_timescale:90000,video:Track{timescale:90000,min_pts90k:90_000,max_pts_end90k:180_000}};
        let mut text="#tb 0: 1/90000\n".to_owned();
        for i in 0..8{text.push_str(&format!("0, {}, {}, 9000, 576, {}\n",90_000+i*9000,90_000+i*9000,"a".repeat(64)));}
        assert_eq!(parse(text.as_bytes(),&timing).unwrap().len(),8);
        assert!(parse(text.replace("90000, 90000","0, 0").as_bytes(),&timing).is_err());
        assert!(parse(text.trim_end().as_bytes(),&timing).is_err());
    }
}
