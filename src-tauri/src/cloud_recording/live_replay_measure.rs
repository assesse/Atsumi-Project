//! Opt-in passive measurement: samples only this owned official player while it
//! plays. It never seeks, resumes, unmutes, changes speed or touches another view.
//! Failure/insufficient overlap stays unknown; no timing is inferred from length.
use super::{CloudError,live_replay_session::LiveReplaySession};
use crate::youtube::original::NativeFrameWindow;
use std::{sync::atomic::Ordering,time::{Duration,Instant}};
const MAX_CALLS:usize=24;
const MAX_SECONDS:u64=210;
impl LiveReplaySession {
    pub(super) fn measure_worker(&self){
        let result=self.observe_alignment();
        if result.is_err(){self.clear_alignment();}
        if let Ok(mut error)=self.measure_error.lock(){*error=result.err();}
        self.measuring.store(false,Ordering::Release);
    }
    fn measurement_check(&self,deadline:Instant)->Result<(),CloudError>{
        self.valid()?;
        if self.measure_cancel.load(Ordering::Acquire){return Err(CloudError::cancelled());}
        if Instant::now()>=deadline{return Err(CloudError::new("CLOUD_LIVE_ALIGNMENT_TIMEOUT","현재 재생 구간에서 동기화를 확인하지 못했습니다. 영상과 기록은 보존됩니다.",true));}
        Ok(())
    }
    fn observe_alignment(&self)->Result<(),CloudError>{
        let deadline=Instant::now()+Duration::from_secs(MAX_SECONDS);
        self.measurement_check(deadline)?;
        let reference=self.history.references()?.ok_or_else(||CloudError::new("CLOUD_LIVE_ALIGNMENT_SOURCE","이 전송에는 장면 비교 기록이 없습니다. 저장된 채팅은 열어 볼 수 있습니다.",false))?;
        if !(2..=3).contains(&reference.references().windows.len())||!reference.timing().transport_complete||reference.timing().timing_incomplete{
            return Err(CloudError::new("CLOUD_LIVE_ALIGNMENT_SOURCE_INCOMPLETE","장면 비교 기록이 충분하지 않아 동기화를 확정할 수 없습니다.",false));
        }
        let mut windows:Vec<NativeFrameWindow>=Vec::new();
        let mut calls=0;
        loop{
            self.measurement_check(deadline)?;
            let clock=self.content_clock()?;
            if clock.as_ref().is_some_and(|c|c.fields.ended)||calls>=MAX_CALLS{
                return Err(CloudError::new("CLOUD_LIVE_ALIGNMENT_NO_MATCH","재생된 장면과 저장된 비교 구간의 일치를 확인하지 못했습니다.",true));
            }
            let eligible=clock.is_some_and(|c|!c.fields.paused&&!c.fields.seeking&&!c.fields.buffering);
            if !eligible{std::thread::sleep(Duration::from_millis(50));continue;}
            calls+=1;
            let window=self.sample()?;
            self.measurement_check(deadline)?;
            let current=parts(&window);
            for previous in &windows {
                for old in parts(previous){for new in &current{
                    self.measurement_check(deadline)?;
                    let mut pair=vec![old.clone(),new.clone()];
                    pair.sort_by_key(|w|w.frames.first().map_or(0,|f|f.media_pts90k));
                    if self.measure(pair).is_ok(){self.measurement_check(deadline)?;return Ok(());}
                }}
            }
            windows.push(window);
            // Keep independent windows separated in real content time. The
            // matcher still verifies separation, source uniqueness and dynamics.
            let next=(Instant::now()+Duration::from_secs(2)).min(deadline);
            while Instant::now()<next{self.measurement_check(deadline)?;std::thread::sleep(Duration::from_millis(50));}
        }
    }
}
fn parts(window:&NativeFrameWindow)->Vec<NativeFrameWindow>{
    if window.frames.len()<8{return Vec::new();}
    let width=window.frames.len().min(12);let last=window.frames.len()-width;
    let mut starts:Vec<_>=(0..=last).step_by(4).collect();if starts.last()!=Some(&last){starts.push(last);}
    starts.into_iter().map(|start|{let mut part=window.clone();part.frames=window.frames[start..start+width].to_vec();part}).collect()
}

