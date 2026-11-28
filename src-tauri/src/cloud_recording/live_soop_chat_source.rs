//! SOOP-only source position projection using saved normalized public UUIDs.
//! Official appearance/assets remain in the original ReplayChatItem unchanged.
use super::*;
use crate::platform_media::replay_chat::{ReplayChatItem,ReplayChatPage};
use timing::EventPosition;
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct SoopSourceChatPage {
    pub page:ReplayChatPage,
    pub positions:Vec<SoopSourceChatItem>,
    pub remote_anchor:&'static str,
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct SoopSourceChatItem {
    pub id:String,
    pub position:Option<SoopSourceChatPosition>,
    pub unmapped_reason:Option<&'static str>,
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct SoopSourceChatPosition {
    pub source_ticks90k:u64,pub transport_ticks90k:u64,pub service:u16,pub observation_sequence:u64,
    pub segment_ordinal:u64,pub segment_sha256:String,pub numeric_uncertainty_ticks90k:u32,
    pub presentation_uncertainty:&'static str,
}
fn identity(item:&ReplayChatItem)->Option<&str>{item.relay_message_id_sha256.as_deref()}
impl LiveHistoryLease {
    /// No page-provided identity/path and no receive-time fallback. This method
    /// consumes the existing source parser while all history files stay pinned.
    pub(crate) fn soop_source_chat_page(&self,cursor:Option<&str>,limit:usize)->Result<SoopSourceChatPage,CloudError>{
        let _active=self.work.begin()?;self.valid()?;
        if self.provider!="soop"{return Err(unavailable());}
        let LiveHistoryPage::Soop(page)=self.page(cursor,limit)? else{return Err(unavailable());};
        let ids:Vec<_>=page.items.iter().map(identity).collect();
        let clocks=self.timing.soop_event_clocks(&ids,&self.work.cancel).map_err(|_|unavailable())?;
        let mut points=Vec::new();let mut prepared=Vec::with_capacity(clocks.len());
        for clock in clocks {
            let index=match &clock{EventPosition::Source{ticks90k,..}=>{let i=points.len();points.push(*ticks90k);Some(i)},_=>None};
            prepared.push((clock,index));
        }
        let mapped=self.timing.map_video_observations(&points,&self.work.cancel).map_err(|_|unavailable())?;
        let mut positions=Vec::with_capacity(prepared.len());
        for (item,(clock,index)) in page.items.iter().zip(prepared) {
            let (position,reason)=match (clock,index) {
                (EventPosition::Source{ticks90k,service,observation_sequence},Some(i))=>match mapped.get(i).and_then(Option::as_ref){
                    Some(point)=>(Some(SoopSourceChatPosition{source_ticks90k:ticks90k,transport_ticks90k:point.transport_ticks90k,service,observation_sequence,
                        segment_ordinal:point.ordinal,segment_sha256:point.segment_sha256.clone(),numeric_uncertainty_ticks90k:2,presentation_uncertainty:"unknown"}),None),
                    None=>(None,Some("outside_acknowledged_envelope")),
                },
                (EventPosition::Unknown(reason),_)=>(None,Some(reason)),
                _=>return Err(unavailable()),
            };
            positions.push(SoopSourceChatItem{id:item.id.clone(),position,unmapped_reason:reason});
        }
        self.valid()?;Ok(SoopSourceChatPage{page,positions,remote_anchor:"unknown"})
    }
}
