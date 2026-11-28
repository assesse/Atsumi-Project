//! Public service121 only. Source: soop-mission-source-proof.json (six variants).
//! Settlement is not a donation. No account ID, raw packet or image URL is saved.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all="camelCase")]
pub enum Kind { Battle, Challenge }
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all="camelCase")]
pub enum Event { Gift, Settle, Notice }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag="result",rename_all="camelCase",deny_unknown_fields)]
pub enum Notice { Draw, Winner { winner:String }, Rank { rank:u32, team:String }, Challenge { title:String, successful:bool } }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Metadata {
    pub kind:Kind, pub event:Event,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub count:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub relay:Option<bool>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub fan_order:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub top_fan:Option<bool>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub notice:Option<Notice>,
    #[serde(default,skip_serializing_if="Option::is_none",deserialize_with="asset_id")]
    pub image_asset_id:Option<String>,
}
fn asset_id<'de,D:serde::Deserializer<'de>>(d:D)->Result<Option<String>,D::Error>{
    let value=Value::deserialize(d)?; Ok(value.as_str().filter(|id|valid_id(id)).map(str::to_owned))
}
fn text(value:&Value,limit:usize)->Result<String,()>{
    let value=value.as_str().ok_or(())?;
    if value.is_empty()||value.chars().count()>limit||value.chars().any(char::is_control){return Err(());}
    Ok(value.into())
}
fn integer(value:&Value)->Result<u32,()>{
    let n=value.as_u64().or_else(||value.as_str().filter(|s|!s.is_empty()&&s.len()<=10&&s.bytes().all(|b|b.is_ascii_digit()))?.parse().ok()).ok_or(())?;
    u32::try_from(n).map_err(|_|())
}
fn optional_integer(value:&Value)->Result<Option<u32>,()>{if value.is_null(){Ok(None)}else{integer(value).map(Some)}}
fn flag(value:&Value)->Result<bool,()>{
    if let Some(v)=value.as_bool(){return Ok(v);}
    match integer(value){Ok(0)=>Ok(false),Ok(1)=>Ok(true),_=>Err(())}
}
fn valid_text(value:&str,limit:usize)->bool{!value.is_empty()&&value.chars().count()<=limit&&!value.chars().any(char::is_control)}
fn valid_id(value:&str)->bool{value.len()==64&&value.bytes().all(|b|b.is_ascii_hexdigit()&&!b.is_ascii_uppercase())}
fn selector(value:&Value)->Option<String>{
    let value=match value{Value::String(v)=>v.clone(),Value::Number(_)=>integer(value).ok()?.to_string(),_=>return None};
    (!value.is_empty()&&value.len()<=64&&value.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'))).then_some(value)
}
impl Metadata {
    pub(crate) fn kind_label(&self)->&'static str{match self.kind{Kind::Battle=>"대결미션",Kind::Challenge=>"도전미션"}}
    pub(crate) fn row_kind(&self)->&'static str{if self.event==Event::Gift{"donation"}else{"system"}}
    pub(crate) fn has_image(&self)->bool{matches!(self.event,Event::Gift|Event::Settle)}
    pub(crate) fn valid(&self)->bool{
        let no_fan=self.fan_order.is_none()&&self.top_fan.is_none();
        match self.event{
            Event::Gift=>self.count.is_some_and(|v|v>0)&&self.relay.is_some()&&self.notice.is_none()&&(self.kind==Kind::Battle||no_fan),
            Event::Settle=>self.count.is_some()&&self.relay.is_none()&&no_fan&&self.notice.is_none(),
            Event::Notice=>self.count.is_none()&&self.relay.is_none()&&no_fan&&self.image_asset_id.is_none()&&match(&self.kind,&self.notice){
                (Kind::Battle,Some(Notice::Draw))=>true,
                (Kind::Battle,Some(Notice::Winner{winner}))=>valid_text(winner,160),
                (Kind::Battle,Some(Notice::Rank{rank,team}))=>*rank>0&&valid_text(team,160),
                (Kind::Challenge,Some(Notice::Challenge{title,..}))=>valid_text(title,512),_=>false,
            }
        }
    }
    /// Public fallback text preserves meaning even if optional image data is lost.
    pub(crate) fn text(&self)->String{
        let name=self.kind_label();
        match self.event{
            Event::Gift=>format!("{}{name} 별풍선 {}개 후원{}",if self.relay==Some(true){"중계방 "}else{""},self.count.unwrap_or(0),if self.kind==Kind::Battle{"!"}else{""}),
            Event::Settle=>format!("{name} 별풍선 {}개 획득!",self.count.unwrap_or(0)),
            Event::Notice=>match &self.notice{
                Some(Notice::Draw)=>"대결미션 결과 무승부입니다! 참여한 스트리머에게 수고의 별풍선을 선물해 보세요.".into(),
                Some(Notice::Winner{winner})=>format!("대결미션 승리 팀은 {winner}입니다. 승리 팀의 스트리머에게 축하의 별풍선을 선물해 보세요."),
                Some(Notice::Rank{rank,team})=>format!("대결미션 {rank}등은 {team}입니다. {rank}등 팀의 스트리머에게 수고의 별풍선을 선물해 보세요."),
                Some(Notice::Challenge{title,successful:true})=>format!("[{title}] 미션을 성공하였습니다! 스트리머에게 축하의 별풍선을 선물해보세요!"),
                Some(Notice::Challenge{title,successful:false})=>format!("[{title}] 미션을 실패하였습니다... 스트리머에게 위로의 별풍선을 선물해보세요!"),_=>String::new(),
            }
        }
    }
}
pub(crate) fn from_stored(value:&Value)->Option<Metadata>{let v:Metadata=serde_json::from_value(value.clone()).ok()?;v.valid().then_some(v)}
pub(crate) fn invalid_stored(value:&Value)->bool{
    !value.is_null()&&(from_stored(value).is_none()||value.get("imageAssetId").is_some_and(|v|!v.is_null()&&!v.as_str().is_some_and(valid_id)))
}
pub(super) struct PublicMission {pub nickname:String,pub metadata:Metadata,pub partial:bool, image:Option<String>}
pub(super) fn parse(fields:&[&str])->Result<PublicMission,()>{
    // Exactly the one public JSON field used by the original service121 handler.
    if fields.is_empty()||fields.len()>8||fields[0].len()>16*1024{return Err(());}
    let v:Value=serde_json::from_str(fields[0]).map_err(|_|())?;if !v.is_object(){return Err(());}
    let(kind,event)=match v["type"].as_str(){
        Some("GIFT")=>(Kind::Battle,Event::Gift),Some("SETTLE")=>(Kind::Battle,Event::Settle),Some("NOTICE")=>(Kind::Battle,Event::Notice),
        Some("CHALLENGE_GIFT")=>(Kind::Challenge,Event::Gift),Some("CHALLENGE_SETTLE")=>(Kind::Challenge,Event::Settle),Some("CHALLENGE_NOTICE")=>(Kind::Challenge,Event::Notice),_=>return Err(()),
    };
    let mut meta=Metadata{kind,event,count:None,relay:None,fan_order:None,top_fan:None,notice:None,image_asset_id:None};
    let mut nickname=String::new();
    match event{
        Event::Gift=>{
            nickname=text(&v["user_nick"],160)?;meta.count=Some(integer(&v["gift_count"])?);meta.relay=Some(flag(&v["is_relay"])?);
            if kind==Kind::Battle{meta.fan_order=optional_integer(&v["fan_order"])?;meta.top_fan=optional_integer(&v["top_fan"])?.map(|v|v>0);}
        },
        Event::Settle=>meta.count=Some(integer(&v["settle_count"])?),
        Event::Notice=>meta.notice=Some(if kind==Kind::Challenge{
            let status=text(&v["mission_status"],64)?;Notice::Challenge{title:text(&v["title"],512)?,successful:status=="SUCCESS"}
        }else if !v["draw"].is_null()&&flag(&v["draw"])?{Notice::Draw}
        else if optional_integer(&v["rank"])?.is_some_and(|v|v>0){Notice::Rank{rank:integer(&v["rank"])?,team:text(&v["my_team_name"],160)?}}
        else{Notice::Winner{winner:text(&v["winner"],160)?}}),
    }
    if !meta.valid(){return Err(());}
    let image=if meta.has_image(){selector(&v["image"])}else{None};
    let partial=meta.has_image()&&image.is_none();
    Ok(PublicMission{nickname,metadata:meta,partial,image})
}
/// Capture-owned capability; serde never constructs this from saved JSON.
#[derive(Clone,Debug)]
pub(crate) struct Request {url:String,id:String,kind:Kind}
impl Request{
    pub(crate) fn from_fields(namespace:uuid::Uuid,fields:&[&str])->Option<Self>{
        let parsed=parse(fields).ok()?;let image=parsed.image?;
        let path=match parsed.metadata.kind{Kind::Battle=>"images/new_app/chat/mission",Kind::Challenge=>"images/chat/mission"};
        let url=format!("https://res.sooplive.com/{path}/m_{image}.png");
        let id=format!("{:x}",Sha256::digest(format!("soop-mission-v1:{namespace}:{url}")));
        Some(Self{url,id,kind:parsed.metadata.kind})
    }
    pub(crate) fn valid(&self)->bool{valid_id(&self.id)&&canonical_url(&self.url)==Some(self.url.as_str())}
    pub(crate) fn matches(&self,meta:&Metadata)->bool{self.valid()&&meta.valid()&&meta.has_image()&&meta.kind==self.kind&&meta.image_asset_id.as_deref().is_none_or(|id|id==self.id)}
    pub(crate) fn apply(&self,meta:&mut Metadata)->bool{if !self.matches(meta){return false;}meta.image_asset_id=Some(self.id.clone());true}
    pub(crate) fn url(&self)->&str{&self.url}
    pub(crate) fn id(&self)->&str{&self.id}
}
pub(crate) fn canonical_url(value:&str)->Option<&str>{
    let selector=value.strip_prefix("https://res.sooplive.com/images/new_app/chat/mission/m_")
        .or_else(||value.strip_prefix("https://res.sooplive.com/images/chat/mission/m_"))?.strip_suffix(".png")?;
    (!selector.is_empty()&&selector.len()<=64&&selector.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'))).then_some(value)
}

#[cfg(test)]
#[path="chat_mission_tests.rs"]
mod tests;
