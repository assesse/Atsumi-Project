//! Non-destructive upload derivatives; no merge-job, source cleanup or publication.
use super::*;

fn local_path(path: &Path) -> Result<String, StreamError> {
    let raw = path.to_str().ok_or_else(|| failure("영상 경로를 읽지 못했습니다."))?;
    // Only native catalog-owned held paths reach this adapter. Network protocols,
    // line breaks and device paths must never acquire meaning in an ffconcat file.
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(raw);
    if raw.starts_with(r"UNC\") || raw.starts_with(r"\\") || raw.contains(['\n','\r','\0']) {
        return Err(failure("클라우드 준비에는 로컬 영상 저장소가 필요합니다."));
    }
    Ok(raw.replace('\\', "/"))
}
fn run_bytes(command: Command, cancel: &AtomicBool) -> Result<Vec<u8>,StreamError> {
    match run_tool(command,cancel,Duration::from_secs(30),false,None)? { ToolOutput::Bytes(data)=>Ok(data), _=>unreachable!() }
}
/// Uploads may contain no audio. Keep the original recording merger's stricter
/// two-track policy unchanged rather than weakening unrelated cleanup proofs.
fn upload_info(value:&Value)->Result<(Value,Option<f64>),StreamError> {
    let streams=value["streams"].as_array().ok_or_else(||failure("업로드 영상 트랙 정보가 없습니다."))?;
    if streams.is_empty()||streams.len()>2||streams.iter().filter(|s|s["codec_type"]=="video").count()!=1
        ||streams.iter().filter(|s|s["codec_type"]=="audio").count()>1
        ||streams.iter().any(|s|!matches!(s["codec_type"].as_str(),Some("video"|"audio"))) {
        return Err(failure("영상 한 개와 선택적인 음성 한 개로 된 녹화만 업로드 준비할 수 있습니다."));
    }
    let mut result=Vec::new();
    for kind in ["video","audio"] {
        let Some(stream)=streams.iter().find(|s|s["codec_type"]==kind) else {continue};
        let codec=stream["codec_name"].as_str().unwrap_or("");
        let supported=if kind=="video" {matches!(codec,"h264"|"hevc"|"av1"|"vp8"|"vp9"|"mpeg4")} else {matches!(codec,"aac"|"opus"|"mp3"|"ac3"|"eac3"|"alac")};
        let clock=stream["time_base"].as_str().and_then(|v|v.split_once('/')).and_then(|(a,b)|Some((a.parse::<u64>().ok()?,b.parse::<u64>().ok()?)));
        if !supported||clock.is_none_or(|(a,b)|a==0||b==0) {return Err(failure("업로드 준비를 지원하지 않는 코덱이나 시간축입니다."));}
        result.push(json!({"index":stream["index"],"type":stream["codec_type"],"codec":stream["codec_name"],"profile":stream["profile"],"timeBase":stream["time_base"],"width":stream["width"],"height":stream["height"],"sampleRate":stream["sample_rate"],"channels":stream["channels"],"channelLayout":stream["channel_layout"],"extra":stream["extradata_hash"]}));
    }
    Ok((Value::Array(result),numeric(&value["format"]["duration"]).filter(|v|*v>0.0)))
}
fn upload_probe(tools:&MediaTools,path:&Path,maximum:f64,cancel:&AtomicBool)->Result<MediaInfo,StreamError> {
    let mut command=Command::new(&tools.ffprobe);
    command.args(["-v","error","-max_alloc",&media_allocation_limit(maximum).to_string(),"-protocol_whitelist","file","-show_data_hash","sha256","-show_entries","stream=index,codec_type,codec_name,profile,time_base,width,height,sample_rate,channels,channel_layout,extradata_hash:format=duration","-of","json"]).arg(path);
    let value:Value=serde_json::from_slice(&run_bytes(command,cancel)?).map_err(|_|failure("업로드 영상 검사 응답 오류"))?;
    let (signature,duration)=upload_info(&value)?;
    let duration=if let Some(duration)=duration {duration}else {
        let mut command=Command::new(&tools.ffprobe);
        command.args(["-v","error","-max_alloc",&media_allocation_limit(maximum).to_string(),"-protocol_whitelist","file","-show_packets","-show_entries","packet=pts_time,dts_time,duration_time","-of","compact=p=0:nk=0"]).arg(path);
        match run_tool(command,cancel,Duration::from_secs(90),true,None)? {ToolOutput::Packets(clock)=>clock.duration()?,_=>unreachable!()}
    };
    if duration<=0.0||duration>maximum {return Err(failure("업로드 영상의 실제 길이가 올바르지 않습니다."));}
    Ok(MediaInfo{signature,duration})
}
fn packet_at(tools:&MediaTools,path:&Path,at:f64,cancel:&AtomicBool) -> Result<(f64,u64),StreamError> {
    let mut command=Command::new(&tools.ffprobe);
    command.args(["-v","error","-max_alloc","268435456","-protocol_whitelist","file","-select_streams","v:0","-read_intervals"])
        .arg(format!("{at:.9}%+1"))
        .args(["-show_packets","-show_entries","packet=pts_time,pos,flags","-of","json"]).arg(path);
    let value:Value=serde_json::from_slice(&run_bytes(command,cancel)?).map_err(|_|failure("영상 분할 경계를 읽지 못했습니다."))?;
    value["packets"].as_array().ok_or_else(||failure("영상 분할 경계가 없습니다."))?.iter()
        .filter(|p|p["flags"].as_str().is_some_and(|f|f.contains('K')&&!f.contains('D')))
        .filter_map(|p|Some((numeric(&p["pts_time"])? , p["pos"].as_str()?.parse::<u64>().ok()?)))
        .filter(|(time,_)|*time>=0.0&&*time<=at+0.000_001)
        .max_by(|a,b|a.0.total_cmp(&b.0))
        .ok_or_else(||failure("독립적으로 재생 가능한 영상 분할 경계를 찾지 못했습니다."))
}
fn audio_boundary(tools:&MediaTools,path:&Path,at:f64,sample_rate:Option<f64>,cancel:&AtomicBool)->Result<bool,StreamError> {
    let Some(rate)=sample_rate else {return Ok(true)};
    let mut command=Command::new(&tools.ffprobe);
    command.args(["-v","error","-max_alloc","268435456","-protocol_whitelist","file","-select_streams","a:0","-read_intervals"])
        .arg(format!("{at:.9}%+1"))
        .args(["-show_packets","-show_entries","packet=pts_time,duration_time","-of","json"]).arg(path);
    let value:Value=serde_json::from_slice(&run_bytes(command,cancel)?).map_err(|_|failure("음성 분할 경계를 읽지 못했습니다."))?;
    let tolerance=(0.5/rate).max(0.000_001);
    Ok(value["packets"].as_array().ok_or_else(||failure("음성 분할 경계가 없습니다."))?.iter().any(|p| {
        numeric(&p["pts_time"]).is_some_and(|start|(start-at).abs()<=tolerance
            || numeric(&p["duration_time"]).is_some_and(|duration|(start+duration-at).abs()<=tolerance))
    }))
}
fn presentation(tools:&MediaTools,path:&Path,cancel:&AtomicBool)->Result<Value,StreamError> {
    let mut command=Command::new(&tools.ffprobe);
    command.args(["-v","error","-max_alloc","268435456","-protocol_whitelist","file","-show_entries","stream=codec_type,start_time,duration,sample_rate,nb_frames,avg_frame_rate,time_base:format=start_time,duration","-of","json"]).arg(path);
    serde_json::from_slice(&run_bytes(command,cancel)?).map_err(|_|failure("영상·음성 재생 구간을 읽지 못했습니다."))
}
fn validate_presentation(source:&Value,output:&Value,start:f64,end:f64)->Result<(),StreamError> {
    for kind in ["video","audio"] {
        let original=source["streams"].as_array().and_then(|v|v.iter().find(|s|s["codec_type"]==kind));
        let result=output["streams"].as_array().and_then(|v|v.iter().find(|s|s["codec_type"]==kind));
        let (original,result)=match(original,result) {(None,None)=>continue,(Some(a),Some(b))=>(a,b),_=>return Err(failure("분할 후 영상·음성 트랙이 달라졌습니다."))};
        let source_start=numeric(&original["start_time"]).unwrap_or(0.0);
        let source_end=source_start+numeric(&original["duration"]).ok_or_else(||failure("원본 트랙 길이가 없습니다."))?;
        let actual_start=numeric(&result["start_time"]).ok_or_else(||failure("분할 트랙 시작 시각이 없습니다."))?;
        let actual_end=actual_start+numeric(&result["duration"]).ok_or_else(||failure("분할 트랙 길이가 없습니다."))?;
        let expected_start=source_start.max(start)-start;
        let expected_end=source_end.min(end)-start;
        // Audio is checked at sample precision, not a blanket 100ms tolerance.
        // This catches the 5.333ms duplicated AAC tail that a container-only
        // duration check previously accepted.
        let tolerance=if kind=="audio" { 1.5/numeric(&original["sample_rate"]).filter(|v|*v>0.0).ok_or_else(||failure("음성 샘플 시계가 없습니다."))? }else {0.001};
        if expected_end<=expected_start || (actual_start-expected_start).abs()>tolerance || (actual_end-expected_end).abs()>tolerance {
            return Err(failure("분할 영상·음성의 재생 경계에 누락 또는 중복이 있습니다. 원본을 유지했습니다."));
        }
    }
    Ok(())
}
fn audio_packet_guard(tools:&MediaTools,path:&Path,cancel:&AtomicBool,progress:Arc<dyn Fn(f64)+Send+Sync>)->Result<(),StreamError> {
    let mut command=Command::new(&tools.ffprobe);
    command.args(["-v","error","-max_alloc","268435456","-protocol_whitelist","file","-select_streams","a:0","-show_packets","-show_entries","packet=pts_time,dts_time,duration_time,flags","-of","compact=p=0:nk=0"]).arg(path);
    let ToolOutput::Packets(clock)=run_tool_observed(command,cancel,Duration::from_secs(300),true,None,Some(progress))? else{unreachable!()};
    if clock.invalid||clock.presentation_overlap||clock.presentation_gap {return Err(failure("준비한 영상의 음성 구간에 중복 또는 빈 구간이 있습니다. 원본은 유지됩니다."));}
    Ok(())
}
fn grouped_presentation(tools:&MediaTools,inputs:&[(PathBuf,f64)],cancel:&AtomicBool)->Result<Value,StreamError> {
    let mut streams:Vec<Value>=Vec::new();let mut offset=0.0;
    for (index,(path,duration)) in inputs.iter().enumerate() {
        let value=presentation(tools,path,cancel)?;
        let current=value["streams"].as_array().ok_or_else(||failure("원본 트랙 재생 범위를 확인하지 못했습니다."))?;
        if index==0 {streams=current.clone();}
        for target in &mut streams {
            let source=current.iter().find(|s|s["codec_type"]==target["codec_type"]).ok_or_else(||failure("원본 트랙 구성이 다릅니다."))?;
            let original_start=numeric(&source["start_time"]).unwrap_or(0.0);
            let original_end=original_start+numeric(&source["duration"]).ok_or_else(||failure("원본 트랙 길이가 없습니다."))?;
            let first_start=numeric(&target["start_time"]).unwrap_or(0.0);
            target["duration"]=json!(offset+original_end-first_start);
        }
        offset+=duration;
    }
    Ok(json!({"streams":streams}))
}

/// Metadata seeks use the existing container index. No whole-movie packet dump
/// or frame decode occurs while freezing the manifest. The actual unit is fully
/// decoded later; a keyframe flag alone is not treated as proof of an open GOP.
pub(crate) fn cloud_upload_cuts(
    tools:&MediaTools,path:&Path,duration:f64,bytes:u64,max_seconds:f64,max_bytes:u64,
    cancel:&AtomicBool,progress:Arc<dyn Fn(f64)+Send+Sync>,
) -> Result<Vec<f64>,StreamError> {
    tools_available(Some(tools))?; regular(path,u64::MAX)?;
    if !duration.is_finite()||duration<=0.0||bytes==0||max_seconds<=0.0||max_bytes==0 { return Err(failure("영상 분할 범위가 올바르지 않습니다.")); }
    let mut command=Command::new(&tools.ffprobe);
    command.args(["-v","error","-max_alloc","268435456","-protocol_whitelist","file","-show_entries","format=start_time,duration:stream=codec_type,start_time,sample_rate","-of","json"]).arg(path);
    let value:Value=serde_json::from_slice(&run_bytes(command,cancel)?).map_err(|_|failure("영상의 시작 시간축을 읽지 못했습니다."))?;
    let start=numeric(&value["format"]["start_time"]).unwrap_or(0.0);
    let actual=numeric(&value["format"]["duration"]).ok_or_else(||failure("영상 길이가 없습니다."))?;
    let audio_rate=value["streams"].as_array().and_then(|v|v.iter().find(|s|s["codec_type"]=="audio")).map(|s|numeric(&s["sample_rate"]).filter(|v|*v>0.0).ok_or_else(||failure("음성 샘플 시계를 읽지 못했습니다."))).transpose()?;
    // Inputs with nonzero global origins require a separate source clock map.
    if start.abs()>0.001 || (actual-duration).abs()>0.1 { return Err(failure("시작 시각이나 재생 길이가 다른 영상은 자동 분할하지 않습니다. 원본은 유지됩니다.")); }
    let mut cuts=vec![0.0]; let mut last=0.0; let mut last_pos=0u64;
    while duration-last>max_seconds+0.000_001 || bytes.saturating_sub(last_pos)>max_bytes {
        check_cancel(cancel)?;
        if cuts.len()>=2048 { return Err(failure("업로드 구간 수 제한을 초과했습니다.")); }
        let remaining=bytes.saturating_sub(last_pos);
        let byte_duration=(duration-last)*(max_bytes as f64/remaining.max(1) as f64)*0.95;
        let mut target=last+max_seconds.min(byte_duration);
        let mut boundary=None;
        let initial_target=target;
        for _ in 0..128 {
            check_cancel(cancel)?;
            let (at,pos)=packet_at(tools,path,target,cancel)?;
            if at<=last+0.001||pos<=last_pos { return Err(failure("영상 분할 경계가 너무 멉니다. 원본을 보존했습니다.")); }
            if pos-last_pos<=max_bytes && at-last<=max_seconds+0.000_001 {
                if audio_boundary(tools,path,at,audio_rate,cancel)? {boundary=Some((at,pos));break;}
                target=at-0.001;
                if initial_target-target>120.0 {return Err(failure("2분 범위에서 영상·음성의 공통 분할 경계를 찾지 못했습니다. 원본은 유지됩니다."));}
            } else {target=last+(at-last)*(max_bytes as f64/(pos-last_pos) as f64).min(0.9)*0.95;}
        }
        let (at,pos)=boundary.ok_or_else(||failure("용량 제한 안의 영상 분할 경계를 찾지 못했습니다."))?;
        cuts.push(at);last=at;last_pos=pos;progress(at);
    }
    cuts.push(duration);progress(duration);Ok(cuts)
}

fn decode(tools:&MediaTools,path:&Path,duration:f64,cancel:&AtomicBool,progress:Arc<dyn Fn(f64)+Send+Sync>) -> Result<(),StreamError> {
    let threads=verification_threads(thread::available_parallelism().map_or(1,|v|v.get())).to_string();
    let mut command=Command::new(&tools.ffmpeg);
    command.args(["-hide_banner","-loglevel","error","-xerror","-err_detect","explode","-threads",&threads,"-filter_threads","1","-max_alloc",&media_allocation_limit(duration).to_string(),"-protocol_whitelist","file"]);
    if let Some(rate)=crate::storage_io_budget::media_read_rate(path,regular(path,u64::MAX)?.len(),duration) { command.args(["-readrate",&rate]); }
    command.arg("-i").arg(path).args(["-map","0:v:0","-map","0:a:0?","-fps_mode:v","passthrough","-enc_time_base:v","demux","-progress","pipe:1","-stats_period","1","-f","null","-"]);
    let ToolOutput::Bytes(data)=run_tool_observed(command,cancel,Duration::from_secs(7200),false,None,Some(progress))? else {unreachable!()};
    let text=std::str::from_utf8(&data).map_err(|_|failure("영상 전체 검증 결과 오류"))?;
    let end=text.lines().filter_map(|l|l.strip_prefix("out_time_us=")?.parse::<f64>().ok()).next_back().unwrap_or(0.0)/1e6;
    if !text.lines().any(|l|l=="progress=end")||(end-duration).abs()>0.1 { return Err(failure("분할 영상의 전체 재생 길이가 원본 구간과 다릅니다.")); }
    Ok(())
}

/// Produces only the caller-owned fresh partial file. Caller owns publication,
/// fingerprint proof and retention. Input handles and directory pins stay held.
pub(crate) fn cloud_upload_remux(
    tools:&MediaTools,inputs:&[(PathBuf,f64)],output:&Path,cut:Option<(f64,f64)>,duration:f64,max_bytes:u64,
    cancel:&AtomicBool,progress:Arc<dyn Fn(&str,f64)+Send+Sync>,
) -> Result<(),StreamError> {
    tools_available(Some(tools))?;check_cancel(cancel)?;
    if inputs.is_empty()||inputs.len()>4096||!duration.is_finite()||duration<=0.0||duration>21600.1||cut.is_some()&&inputs.len()!=1 { return Err(failure("업로드 영상 구간이 올바르지 않습니다.")); }
    let root=output.parent().ok_or_else(||failure("업로드 준비 경로 오류"))?;
    space(root,0)?;
    let inputs = inputs.iter().map(|(path,seconds)| {
        fs::canonicalize(path).map(|path|(path,*seconds)).map_err(|_|failure("원본 영상 경로를 확인하지 못했습니다."))
    }).collect::<Result<Vec<_>,_>>()?;
    // Existing parser and strict input signatures keep concat incompatible codec
    // changes from becoming a deceptively playable truncated upload.
    let mut signature=None;let mut pins=Vec::new();
    for (path,seconds) in &inputs {
        pins.push(browser_store::cleanup::verification_guard(path)?);
        pins.extend(browser_store::cleanup::pin_directories(path.parent().ok_or_else(||failure("원본 경로 오류"))?)?);
        let info=upload_probe(tools,path,*seconds+2.0,cancel)?;
        if signature.as_ref().is_some_and(|s|s!=&info.signature) { return Err(failure("영상·음성 형식이 다른 원본 구간은 합치지 않습니다.")); }
        if (info.duration-seconds).abs()>0.1 { return Err(failure("원본 구간의 실제 길이가 저장 시간축과 다릅니다.")); }
        signature=Some(info.signature);
    }
    let output_file=new_file(output)?;drop(output_file);
    let mut command=Command::new(&tools.ffmpeg);
    command.args(["-hide_banner","-loglevel","error","-threads","1","-max_alloc",&media_allocation_limit(duration).to_string(),"-protocol_whitelist","file"]);
    if let Some((start,end))=cut {
        if start<0.0||end<=start||(end-start-duration).abs()>0.001 { return Err(failure("분할 영상 시간 범위 오류")); }
        // Preserve decoder priming as negative timestamps and use the MP4 edit
        // list to exclude it from presentation. Seeking to exactly zero also
        // discards an original AAC priming packet, so omit -ss for the first unit.
        let preroll=start.min(2.0);
        if start>0.0 {
            let (prior,_) = packet_at(tools,&inputs[0].0,start-preroll,cancel)?;
            if start-prior>120.0 {return Err(failure("분할 앞부분의 디코더 준비 구간이 너무 깁니다. 원본을 유지했습니다."));}
            if start-preroll>0.0 {command.args(["-ss",&format!("{:.9}",start-preroll)]);}
        }
        command.arg("-i").arg(&inputs[0].0).args(["-t",&format!("{:.9}",duration+preroll)]);
        if preroll>0.0 {command.args(["-output_ts_offset",&format!("-{preroll:.9}")]);}
        command.args(["-use_editlist","1"]);
    } else {
        let list=output.with_extension("ffconcat");let mut file=new_file(&list)?;
        file.write_all(b"ffconcat version 1.0\n").map_err(|_|failure("업로드 구간 목록 저장 오류"))?;
        for (index,(path,seconds)) in inputs.iter().enumerate() {
            let escaped=local_path(path)?.replace('\'',"'\\''");
            writeln!(file,"file '{escaped}'").map_err(|_|failure("업로드 구간 목록 저장 오류"))?;
            // Retain the very first encoder priming packet. Seeking later files
            // to their presentation origin drops their negative preroll; copying
            // it into the middle duplicates audio already present in prior input.
            if index>0 {writeln!(file,"inpoint 0").map_err(|_|failure("업로드 구간 목록 저장 오류"))?;}
            writeln!(file,"duration {seconds:.9}").map_err(|_|failure("업로드 구간 목록 저장 오류"))?;
        }
        file.sync_all().map_err(|_|failure("업로드 구간 목록 확정 오류"))?;
        command.args(["-copyts","-f","concat","-safe","0","-format_whitelist","concat,mov,matroska,webm"]).arg("-i").arg(&list);
    }
    command.args(["-map","0:v:0","-map","0:a:0?","-map_metadata","-1","-map_chapters","-1","-c","copy","-avoid_negative_ts","disabled","-f","mp4","-progress","pipe:1","-stats_period","1","-y"]).arg(output);
    progress("preparing",0.0);let callback=progress.clone();
    run_tool_observed(command,cancel,Duration::from_secs(7200),false,Some((output,max_bytes)),Some(Arc::new(move |at|callback("preparing",at))))?;
    let result=upload_probe(tools,output,duration+2.0,cancel)?;
    if (result.duration-duration).abs()>0.1 { return Err(failure("분할 영상의 경계가 원본 시간축과 일치하지 않습니다. 원본을 유지했습니다.")); }
    let original=signature.ok_or_else(||failure("원본 트랙 정보 없음"))?;
    let count=original.as_array().map_or(0,Vec::len);
    if result.signature.as_array().map_or(0,Vec::len)!=count {return Err(failure("업로드 준비 후 트랙 수가 달라졌습니다."));}
    for index in 0..count { for key in ["codec","width","height","sampleRate","channels"] { if result.signature[index][key]!=original[index][key] {return Err(failure("업로드 준비 후 영상·음성 형식이 달라졌습니다."));} } }
    progress("verifying",0.0);
    if let Some((start,end))=cut {validate_presentation(&presentation(tools,&inputs[0].0,cancel)?,&presentation(tools,output,cancel)?,start,end)?;}
    else {validate_presentation(&grouped_presentation(tools,&inputs,cancel)?,&presentation(tools,output,cancel)?,0.0,duration)?;}
    if count>1 {let callback=progress.clone();audio_packet_guard(tools,output,cancel,Arc::new(move|at|callback("verifying",at)))?;}
    OpenOptions::new().write(true).open(output).and_then(|file|file.sync_all()).map_err(|_|failure("업로드 준비 영상을 확정하지 못했습니다."))?;
    progress("verifying",0.0);let callback=progress.clone();
    decode(tools,output,duration,cancel,Arc::new(move|at|callback("verifying",at)))?;
    check_cancel(cancel)?;Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn concat_paths_reject_injected_lines_and_network() {
        assert!(local_path(Path::new("C:/owned/media.mp4")).is_ok());
        assert!(local_path(Path::new("C:/owned/a\nfile evil.mp4")).is_err());
        assert!(local_path(Path::new(r"\\?\UNC\server\media.mp4")).is_err());
    }
    #[test] fn upload_probe_accepts_video_only_and_optional_audio_without_relaxing_track_shape() {
        let video=json!({"index":0,"codec_type":"video","codec_name":"h264","time_base":"1/90000","width":64,"height":48});
        let audio=json!({"index":1,"codec_type":"audio","codec_name":"aac","time_base":"1/48000","sample_rate":"48000","channels":2});
        assert_eq!(upload_info(&json!({"streams":[video.clone()],"format":{"duration":"12.0"}})).unwrap().0.as_array().unwrap().len(),1);
        assert_eq!(upload_info(&json!({"streams":[video.clone(),audio.clone()],"format":{"duration":"12.0"}})).unwrap().0.as_array().unwrap().len(),2);
        assert!(upload_info(&json!({"streams":[audio]})).is_err());
        assert!(upload_info(&json!({"streams":[video.clone(),video.clone()]})).is_err());
        assert!(upload_info(&json!({"streams":[video,{"codec_type":"subtitle"}]})).is_err());
    }
    #[test] fn presentation_guard_rejects_sub_100ms_audio_duplication() {
        let source=json!({"streams":[{"codec_type":"video","start_time":"0","duration":"32"},{"codec_type":"audio","start_time":"0","duration":"32","sample_rate":"48000"}]});
        let correct=json!({"streams":[{"codec_type":"video","start_time":"0","duration":"8"},{"codec_type":"audio","start_time":"0","duration":"8","sample_rate":"48000"}]});
        assert!(validate_presentation(&source,&correct,8.0,16.0).is_ok());
        let mut duplicate=correct.clone();duplicate["streams"][1]["duration"]=json!("8.005333");
        assert!(validate_presentation(&source,&duplicate,8.0,16.0).is_err());
        let mut late=correct;late["streams"][1]["start_time"]=json!("0.021333");assert!(validate_presentation(&source,&late,8.0,16.0).is_err());
    }
    #[test] fn packet_guard_ignores_initial_priming_and_rejects_middle_overlap() {
        let mut good=PacketClock::default();
        good.line("pts_time=-0.021333|duration_time=0.021333|flags=KD");
        good.line("pts_time=0.000000|duration_time=0.021333|flags=K_");
        good.line("pts_time=0.021333|duration_time=0.021333|flags=K_");
        assert!(!good.presentation_overlap&&!good.presentation_gap);
        good.line("pts_time=0.030000|duration_time=0.021333|flags=K_");assert!(good.presentation_overlap);
        let mut gap=PacketClock::default();gap.line("pts_time=0.0|duration_time=0.02|flags=K_");gap.line("pts_time=0.04|duration_time=0.02|flags=K_");assert!(gap.presentation_gap);
    }
    #[test] #[ignore = "explicit local 13h synthetic fixture; no user media or network"]
    fn cloud_upload_long_fixture_has_three_continuous_units() {
        let fixture=std::env::var_os("ATSUMI_CLOUD_SPLIT_FIXTURE").map(PathBuf::from).expect("explicit fixture path");
        let bin=std::env::var_os("ATSUMI_CLOUD_TOOLS").map(PathBuf::from).expect("explicit verified tools root");
        let tools=MediaTools{ffmpeg:bin.join("ffmpeg.exe"),ffprobe:bin.join("ffprobe.exe")};
        let cancel=AtomicBool::new(false);let bytes=fs::metadata(&fixture).unwrap().len();
        let cuts=cloud_upload_cuts(&tools,&fixture,46800.0,bytes,21600.0,8*1024*1024*1024,&cancel,Arc::new(|_|{})).unwrap();
        assert_eq!(cuts,vec![0.0,21600.0,43200.0,46800.0]);
        let root=fixture.parent().unwrap().join(format!("verified-cloud-units-{}",uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for (index,window) in cuts.windows(2).enumerate() {
            cloud_upload_remux(&tools,&[(fixture.clone(),46800.0)],&root.join(format!("unit-{index}.mp4")),Some((window[0],window[1])),window[1]-window[0],8*1024*1024*1024,&cancel,Arc::new(|_,_|{})).unwrap();
        }
    }
    #[test] #[ignore = "explicit synthetic fixture; creates retained video-only derivatives"]
    fn cloud_upload_video_only_fixture_splits_without_audio() {
        let fixture=std::env::var_os("ATSUMI_CLOUD_SPLIT_FIXTURE").map(PathBuf::from).expect("explicit fixture path");
        let bin=std::env::var_os("ATSUMI_CLOUD_TOOLS").map(PathBuf::from).expect("explicit verified tools root");
        let tools=MediaTools{ffmpeg:bin.join("ffmpeg.exe"),ffprobe:bin.join("ffprobe.exe")};let cancel=AtomicBool::new(false);
        let root=fixture.parent().unwrap().join(format!("verified-video-only-{}",uuid::Uuid::new_v4()));fs::create_dir(&root).unwrap();
        let source=root.join("video-only.mp4");let mut command=Command::new(&tools.ffmpeg);
        command.args(["-nostdin","-hide_banner","-loglevel","error","-protocol_whitelist","file"]).arg("-i").arg(&fixture).args(["-t","120","-map","0:v:0","-an","-c:v","copy","-n"]).arg(&source);
        run_bytes(command,&cancel).unwrap();
        assert_eq!(upload_probe(&tools,&source,120.1,&cancel).unwrap().signature.as_array().unwrap().len(),1);
        let cuts=cloud_upload_cuts(&tools,&source,120.0,fs::metadata(&source).unwrap().len(),60.0,8*1024*1024*1024,&cancel,Arc::new(|_|{})).unwrap();assert_eq!(cuts,vec![0.0,60.0,120.0]);
        for (index,window) in cuts.windows(2).enumerate() {
            let out=root.join(format!("unit-{index}.mp4"));
            cloud_upload_remux(&tools,&[(source.clone(),120.0)],&out,Some((window[0],window[1])),window[1]-window[0],8*1024*1024*1024,&cancel,Arc::new(|_,_|{})).unwrap();
            assert_eq!(upload_probe(&tools,&out,60.1,&cancel).unwrap().signature.as_array().unwrap().len(),1);
        }
    }
    #[test] #[ignore = "explicit moving/chirp synthetic fixture; retained split and grouped output"]
    fn cloud_upload_boundary_fixture_splits_and_groups() {
        let fixture=std::env::var_os("ATSUMI_CLOUD_BOUNDARY_FIXTURE").map(PathBuf::from).expect("explicit 32s moving/chirp fixture");
        let bin=std::env::var_os("ATSUMI_CLOUD_TOOLS").map(PathBuf::from).expect("explicit verified tools root");
        let tools=MediaTools{ffmpeg:bin.join("ffmpeg.exe"),ffprobe:bin.join("ffprobe.exe")};let cancel=AtomicBool::new(false);
        let root=fixture.parent().unwrap().join(format!("verified-production-boundaries-{}",uuid::Uuid::new_v4()));fs::create_dir(&root).unwrap();
        let cuts=cloud_upload_cuts(&tools,&fixture,32.0,fs::metadata(&fixture).unwrap().len(),10.0,8*1024*1024*1024,&cancel,Arc::new(|_|{})).unwrap();
        assert_eq!(cuts,vec![0.0,8.0,16.0,24.0,32.0]);
        let mut parts=Vec::new();
        for (index,window) in cuts.windows(2).enumerate() {
            let output=root.join(format!("unit-{index}.mp4"));let duration=window[1]-window[0];
            cloud_upload_remux(&tools,&[(fixture.clone(),32.0)],&output,Some((window[0],window[1])),duration,8*1024*1024*1024,&cancel,Arc::new(|_,_|{})).unwrap();parts.push((output,duration));
        }
        cloud_upload_remux(&tools,&parts,&root.join("grouped.mp4"),None,32.0,8*1024*1024*1024,&cancel,Arc::new(|_,_|{})).unwrap();
        fs::write(root.join("cuts.json"),serde_json::to_vec(&cuts).unwrap()).unwrap();
    }
}
