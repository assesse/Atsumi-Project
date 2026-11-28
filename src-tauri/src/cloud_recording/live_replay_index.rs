//! Session-owned disk seek index. The original sidecars and assets are read only.
//! Untimed records remain individually reachable; receive times never become remote times.
use super::live_replay_session::{LiveReplaySession,Row,SourcePoint,RemoteInterval,unavailable};
use super::CloudError;
use serde::Serialize;
use serde_json::{Value,json};
use super::live_replay_disk_index::DiskIndex;

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Position {pub id:String,pub remote_interval:Option<RemoteInterval>,pub unmapped_reason:Option<String>}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Page {pub provider:String,pub page:Value,pub positions:Vec<Position>,
    pub native_clock:Option<crate::youtube::original::VodClock>,pub alignment_state:&'static str,
    pub cursor_generation:Option<CursorGeneration>,pub untimed_count:usize,pub source_count:usize,pub incomplete:bool}
#[derive(Serialize,PartialEq)]
#[serde(rename_all="camelCase")]
pub struct CursorGeneration {pub document_epoch:u64,pub source_id:String,pub source_generation:u64,pub seek_generation:u64}
impl LiveReplaySession {
    /// One registry-owned joinable worker invokes this. No tasks are detached.
    pub(super) fn build_index(&self){
        let result=self.scan();
        if let Ok(mut index)=self.index.lock(){
            index.ready=result.is_ok();
            if let Err(error)=result{index.partial=true;index.error=Some(error);}
        }
    }
    fn scan(&self)->Result<(),CloudError>{
        let mut cursor:Option<String>=None;
        {let mut index=self.index.lock().map_err(|_|unavailable())?;
            index.store=Some(DiskIndex::new(self.cancel.clone(),self.opened.token.clone())?);}
        loop {
            self.valid()?;
            let (rows,next,meta)=if self.opened.provider=="chzzk" {
                let page=self.history.source_chat_page(cursor.as_deref(),120)?;
                let mut rows=Vec::with_capacity(page.items.len());
                for entry in page.items {
                    let assets:std::collections::BTreeMap<_,_>=entry.message.rich.as_ref().map(|rich|
                        rich.asset_urls().filter_map(|url|crate::streaming::replay_assets::asset_id(url).map(|id|(url.clone(),id))).collect()).unwrap_or_default();
                    let id=entry.message.sequence.to_string();
                    let mut value=serde_json::to_value(&entry.message).map_err(|_|unavailable())?;
                    let fields=value.as_object_mut().ok_or_else(unavailable)?;
                    fields.insert("assetIds".into(),json!(assets));
                    // Explicitly optional. Never substitute the receive-time offset.
                    fields.insert("mediaTimeSeconds".into(),Value::Null);
                    fields.insert("syncQuality".into(),json!("unknown"));
                    rows.push(Row{id,value,source:entry.position.map(|p|SourcePoint{transport90k:p.transport_ticks90k,numeric_uncertainty:p.numeric_uncertainty_ticks90k}),reason:entry.unmapped_reason.map(str::to_owned)});
                }
                (rows,page.cursor,None)
            } else {
                let page=self.history.soop_source_chat_page(cursor.as_deref(),120)?;
                if page.page.state=="indexing" {
                    std::thread::sleep(std::time::Duration::from_millis(25));continue;
                }
                if page.page.items.len()!=page.positions.len(){return Err(unavailable());}
                let next=page.page.cursor.clone();
                let mut meta=serde_json::to_value(&page.page).map_err(|_|unavailable())?;
                meta.as_object_mut().ok_or_else(unavailable)?.remove("items");
                let mut rows=Vec::with_capacity(page.page.items.len());
                for (item,position) in page.page.items.into_iter().zip(page.positions){
                    if item.id!=position.id{return Err(unavailable());}
                    let mut value=serde_json::to_value(&item).map_err(|_|unavailable())?;
                    value.as_object_mut().ok_or_else(unavailable)?.insert("timeMs".into(),Value::Null);
                    rows.push(Row{id:item.id,value,source:position.position.map(|p|SourcePoint{transport90k:p.transport_ticks90k,numeric_uncertainty:p.numeric_uncertainty_ticks90k}),reason:position.unmapped_reason.map(str::to_owned)});
                }
                (rows,next,Some(meta))
            };
            self.valid()?;
            {
                let mut index=self.index.lock().map_err(|_|unavailable())?;
                index.store.as_mut().ok_or_else(unavailable)?.append(rows,meta,next.as_deref())?;
            }
            self.valid()?;
            let Some(next)=next else{break;};
            cursor=Some(next);
        }
        self.valid()
    }
    /// The only playback position comes from this owned player's native clock.
    /// History/untimed cursors are integer indices scoped by the public token.
    pub(super) fn page(&self,mode:&str,cursor:Option<&str>,limit:usize)->Result<Page,CloudError>{
        self.valid()?;
        if !matches!(mode,"current"|"history"|"untimed"){return Err(unavailable());}
        let limit=limit.clamp(1,120);
        let clock=self.content_clock()?;
        let alignment=self.current_alignment(clock.as_ref())?;
        let generation=clock.as_ref().map(|c|CursorGeneration{document_epoch:c.document_epoch,source_id:c.fields.source_id.clone(),source_generation:c.fields.source_generation,seek_generation:c.fields.seek_generation});
        let now=clock.as_ref().filter(|c|!c.fields.seeking&&!c.fields.buffering).and_then(|c|c.fields.current_time).filter(|t|t.is_finite()&&*t>=0.).map(|t|(t*90000.).floor() as u64);
        let (mut result,next,previous,next_at,untimed,count,incomplete,meta,ready,partial)={
            let mut index=self.index.lock().map_err(|_|unavailable())?;
            let selected=match index.store.as_mut(){Some(store)=>Some(store.select(mode,cursor,limit,now,alignment.as_ref(),self.opened.duration_seconds)?),None=>None};
            let (result,next,previous,next_at,untimed,count,meta)=if let Some(selected)=selected{
                let mut result=Vec::with_capacity(selected.rows.len());
                for stored in selected.rows{
                    let mut value=stored.row.value;let interval=stored.interval;
                    if let Some(p)=&interval{
                        let midpoint=p.lower_ticks90k as f64+(p.upper_ticks90k-p.lower_ticks90k) as f64/2.;
                        if let Some(fields)=value.as_object_mut(){
                            if self.opened.provider=="chzzk"{fields.insert("mediaTimeSeconds".into(),json!(midpoint/90000.));fields.insert("syncQuality".into(),json!("observed_media"));}
                            else{fields.insert("timeMs".into(),json!((midpoint/90.).round() as u64));}
                        }
                    }
                    let reason=if interval.is_some(){None}else{Some(stored.row.reason.unwrap_or_else(||if alignment.is_none(){"remote_alignment_unknown"}else{"outside_remote_duration"}.into()))};
                    result.push((value,Position{id:stored.row.id,remote_interval:interval,unmapped_reason:reason}));
                }
                (result,selected.next,selected.previous,selected.next_at,selected.untimed,selected.total,selected.meta)
            }else{(Vec::new(),None,None,None,0,0,None)};
            (result,next,previous,next_at,untimed,count,index.partial||self.facts().history_incomplete,meta,index.ready,index.partial)
        };
        self.valid()?;
        // Reject responses that crossed an actual document/source/seek boundary.
        let after=self.content_clock()?;
        let after_generation=after.as_ref().map(|c|CursorGeneration{document_epoch:c.document_epoch,source_id:c.fields.source_id.clone(),source_generation:c.fields.source_generation,seek_generation:c.fields.seek_generation});
        let after_alignment=self.current_alignment(after.as_ref())?;
        let before_binding=alignment.as_ref().map(|a|(a.binding.clone(),a.offset));
        let after_binding=after_alignment.as_ref().map(|a|(a.binding.clone(),a.offset));
        if generation!=after_generation||before_binding!=after_binding{return Err(CloudError::new("CLOUD_LIVE_REPLAY_CHANGED","재생 위치가 바뀌었습니다.",true));}
        let positions=result.iter_mut().map(|(_,p)|Position{id:p.id.clone(),remote_interval:p.remote_interval.clone(),unmapped_reason:p.unmapped_reason.clone()}).collect();
        let items:Vec<_>=result.into_iter().map(|(v,_)|v).collect();
        let mut page=if self.opened.provider=="chzzk"{json!({"generation":generation.as_ref().map_or(0,|g|g.seek_generation),"previousCursor":previous,"cursor":next,"nextCursor":next,
            "indexState":if partial{"failed"}else if ready{"ready"}else{"building"},"syncQuality":if alignment.is_some(){"observed_media"}else{"unknown"},"warnings":[]})}
            else{let mut v=meta.unwrap_or_else(||json!({"state":"indexing","timelineQuality":"missing-anchor","total":0,"malformedLines":0,"unsupportedItems":0,"reordered":0,"duplicates":0,"partialFile":false}));
                let f=v.as_object_mut().ok_or_else(unavailable)?;
                f.insert("state".into(),json!(if partial{"error"}else{soop_state(ready,mode,alignment.is_some())}));
                // Display-control receipt time has no verified remote coordinate.
                // Per-message creation appearance remains intact; never apply a later global palette.
                if f.get("soopDisplay").is_some_and(|v|!v.is_null()){f.insert("soopDisplay".into(),Value::Null);f.insert("soopDisplayIncomplete".into(),json!(true));}
                f.insert("cursor".into(),json!(next));f.insert("previousCursor".into(),json!(previous));f.insert("nextAtMs".into(),json!(next_at.map(|t|t/90)));f.insert("total".into(),json!(count));f.insert("timelineQuality".into(),json!(if alignment.is_some(){"observed-media-interval"}else{"missing-anchor"}));v};
        page.as_object_mut().ok_or_else(unavailable)?.insert("items".into(),json!(items));
        Ok(Page{provider:self.opened.provider.clone(),page,positions,native_clock:after,alignment_state:if alignment.is_some(){"measured"}else{"unknown"},cursor_generation:after_generation,untimed_count:untimed,source_count:count,incomplete})
    }
}
fn soop_state(ready:bool,mode:&str,measured:bool)->&'static str{
    if !ready{"indexing"}else if mode=="current"&&measured{"ready"}else{"unmapped"}
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn measured_current_keeps_polling_even_before_first_row(){
        assert_eq!(soop_state(false,"current",true),"indexing");
        assert_eq!(soop_state(true,"current",true),"ready");
        assert_eq!(soop_state(true,"current",false),"unmapped");
        assert_eq!(soop_state(true,"history",true),"unmapped");
        assert_eq!(soop_state(true,"untimed",true),"unmapped");
    }
}




