//! Session-local disposable chat derivative. No original path or media enters
//! SQLite. Payloads are duplicated temporarily (at most512MiB per admitted
//! session); exhaustion is a preparation error, never a completed history.
use super::{CloudError,live_alignment::MeasuredAlignment,
    live_replay_session::{Row,SourcePoint,RemoteInterval,unavailable,project_interval}};
use rusqlite::{params,Connection,OptionalExtension};
use serde_json::Value;
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};

const MAX_PAGES:u32=131_072;
const PAGE_BYTES:usize=1024*1024;
fn storage()->CloudError{CloudError::new("CLOUD_LIVE_HISTORY_STORAGE","채팅 탐색용 임시 공간을 준비하지 못했습니다. 원본 기록은 보존됩니다.",true)}
fn changed()->CloudError{CloudError::new("CLOUD_LIVE_REPLAY_CHANGED","재생 위치가 바뀌었습니다.",true)}
// Lexicographic BLOB ordering exactly covers negative uncertainty endpoints,
// all u64 transport values and signed offsets, without SQLite REAL conversion.
fn key(value:i128)->[u8;16]{((value as u128)^(1u128<<127)).to_be_bytes()}
#[derive(Clone,PartialEq,Eq)]
struct Projection { lower:i64,upper:i64,end:u64,binding:String }
impl Projection {
    fn from(value:Option<&MeasuredAlignment>,duration:f64)->Result<Option<Self>,CloudError>{
        let Some(m)=value else{return Ok(None)};
        let end=(duration*90_000.).floor();
        if !end.is_finite()||end<0.||end>u64::MAX as f64||m.offset.lower>m.offset.upper{return Err(unavailable());}
        Ok(Some(Self{lower:m.offset.lower,upper:m.offset.upper,end:end as u64,
            binding:format!("{:?}/{:?}/{}",m.binding,m.offset,m.two_window_estimate)}))
    }
    fn low(&self)->[u8;16]{key(i128::from(self.upper))}
    fn high(&self)->[u8;16]{key(i128::from(self.end)+i128::from(self.lower))}
}
#[derive(Clone,Copy)]
struct Cursor { at:i64,backward:bool }
pub(super) struct Stored { pub ordinal:i64,pub row:Row,pub interval:Option<RemoteInterval>,meta:Option<i64> }
pub(super) struct Selection { pub rows:Vec<Stored>,pub next:Option<String>,pub previous:Option<String>,
    pub next_at:Option<u64>,pub total:usize,pub untimed:usize,pub meta:Option<Value> }
pub(super) struct DiskIndex {
    db:Connection,cancel:Arc<AtomicBool>,scope:String,count:usize,mapped:usize,
    projection:Option<Projection>,generation:u64,last_meta:Option<i64>,
}
impl DiskIndex {
    pub(super) fn new(cancel:Arc<AtomicBool>,scope:String)->Result<Self,CloudError>{Self::with_pages(cancel,scope,MAX_PAGES)}
    fn with_pages(cancel:Arc<AtomicBool>,scope:String,pages:u32)->Result<Self,CloudError>{
        if uuid::Uuid::parse_str(&scope).is_err(){return Err(unavailable());}
        let db=Connection::open("").map_err(|_|storage())?;
        db.execute_batch(&format!("PRAGMA trusted_schema=OFF; PRAGMA page_size=4096; PRAGMA max_page_count={pages}; PRAGMA cache_size=-2048; PRAGMA temp_store=FILE; PRAGMA mmap_size=0; PRAGMA journal_mode=DELETE; PRAGMA synchronous=OFF;
            CREATE TABLE rows(ord INTEGER PRIMARY KEY,id TEXT NOT NULL,payload TEXT NOT NULL,t BLOB,n INTEGER,lo BLOB,hi BLOB,reason TEXT,meta INTEGER);
            CREATE INDEX by_hi ON rows(hi DESC,ord DESC); CREATE INDEX by_lo ON rows(lo,ord);
            CREATE TABLE metas(id INTEGER PRIMARY KEY,body TEXT NOT NULL UNIQUE);
            CREATE TABLE cursors(value TEXT PRIMARY KEY) WITHOUT ROWID;")).map_err(|_|storage())?;
        let flag=cancel.clone();db.progress_handler(1000,Some(move||flag.load(Ordering::Acquire)));
        let value=Self{db,cancel,scope,count:0,mapped:0,projection:None,generation:1,last_meta:None};value.check()?;Ok(value)
    }
    fn check(&self)->Result<(),CloudError>{if self.cancel.load(Ordering::Acquire){Err(CloudError::cancelled())}else{Ok(())}}
    fn sql_error(&self)->CloudError{if self.cancel.load(Ordering::Acquire){CloudError::cancelled()}else{storage()}}
    pub(super) fn count(&self)->usize{self.count}
    pub(super) fn counts(&mut self,m:Option<&MeasuredAlignment>,duration:f64)->Result<(usize,usize),CloudError>{
        self.check()?;let projection=Projection::from(m,duration)?;
        if projection!=self.projection{
            let mapped=if let Some(p)=&projection{self.db.query_row("SELECT COUNT(*) FROM rows WHERE lo>=?1 AND hi<=?2",params![p.low().as_slice(),p.high().as_slice()],|r|r.get::<_,i64>(0)).map_err(|_|self.sql_error())?}else{0};
            self.check()?;self.mapped=usize::try_from(mapped).map_err(|_|unavailable())?;
            self.generation=self.generation.checked_add(1).ok_or_else(unavailable)?;self.projection=projection;
        }
        Ok((self.count,self.count.checked_sub(self.mapped).ok_or_else(unavailable)?))
    }
    /// One source page is atomic, including its cursor. The worker validates
    /// native source ownership before and after calling this method.
    pub(super) fn append(&mut self,rows:Vec<Row>,meta:Option<Value>,next:Option<&str>)->Result<(),CloudError>{
        self.check()?;if rows.len()>120||next.is_some_and(|v|v.is_empty()||v.len()>128){return Err(unavailable());}
        let old_count=self.count;let mut mapped=self.mapped;let mut meta_id=self.last_meta;
        let projection=self.projection.clone();
        let cancelled=self.cancel.clone();
        let result=(||->Result<(),CloudError>{
            let tx=self.db.transaction().map_err(|_|storage())?;
            if let Some(next)=next{tx.execute("INSERT INTO cursors(value) VALUES (?1)",[next]).map_err(|error|
                if error.sqlite_error_code()==Some(rusqlite::ErrorCode::ConstraintViolation){
                    CloudError::new("CLOUD_LIVE_HISTORY_CURSOR","채팅 기록의 다음 페이지가 반복됐습니다. 원본 기록은 보존됩니다.",false)
                }else{storage()})?;}
            if let Some(mut meta)=meta{
                if let Some(f)=meta.as_object_mut(){for k in ["cursor","previousCursor","nextAtMs","total","preparation","state"]{f.remove(k);}}
                let body=serde_json::to_string(&meta).map_err(|_|unavailable())?;
                if body.len()>PAGE_BYTES{return Err(storage());}
                tx.execute("INSERT OR IGNORE INTO metas(body) VALUES (?1)",[&body]).map_err(|_|storage())?;
                meta_id=Some(tx.query_row("SELECT id FROM metas WHERE body=?1",[body],|r|r.get(0)).map_err(|_|storage())?);
            }
            let mut insert=tx.prepare_cached("INSERT INTO rows(ord,id,payload,t,n,lo,hi,reason,meta) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)").map_err(|_|storage())?;
            for (i,row) in rows.iter().enumerate(){
                if cancelled.load(Ordering::Acquire){return Err(CloudError::cancelled());}
                let ord=i64::try_from(old_count.checked_add(i).ok_or_else(storage)?).map_err(|_|storage())?;
                let payload=serde_json::to_string(&row.value).map_err(|_|unavailable())?;
                if payload.len()>PAGE_BYTES{return Err(CloudError::new("CLOUD_LIVE_HISTORY_ROW_LIMIT","한 채팅 항목이 표시 크기를 넘었습니다. 원본 기록은 보존됩니다.",false));}
                let (t,n,lo,hi)=row.source.as_ref().map(|s|{
                    let lo=i128::from(s.transport90k)-i128::from(s.numeric_uncertainty);let hi=i128::from(s.transport90k)+i128::from(s.numeric_uncertainty);
                    if projection.as_ref().is_some_and(|p|lo>=i128::from(p.upper)&&hi<=i128::from(p.end)+i128::from(p.lower)){mapped+=1;}
                    (Some(s.transport90k.to_be_bytes().to_vec()),Some(i64::from(s.numeric_uncertainty)),Some(key(lo).to_vec()),Some(key(hi).to_vec()))
                }).unwrap_or((None,None,None,None));
                insert.execute(params![ord,row.id,payload,t,n,lo,hi,row.reason,meta_id]).map_err(|_|storage())?;
            }
            drop(insert);if cancelled.load(Ordering::Acquire){return Err(CloudError::cancelled());}
            tx.commit().map_err(|_|storage())?;Ok(())
        })();
        if let Err(error)=result{return Err(if cancelled.load(Ordering::Acquire){CloudError::cancelled()}else{error});}
        self.count=old_count.checked_add(rows.len()).ok_or_else(storage)?;self.mapped=mapped;self.last_meta=meta_id;self.check()
    }
    fn cursor(&self,mode:&str,value:Option<&str>)->Result<Cursor,CloudError>{
        let Some(v)=value else{return Ok(Cursor{at:0,backward:false})};
        if v.len()>128{return Err(unavailable());}
        let parts:Vec<_>=v.split(':').collect();
        if parts.len()!=6||parts[0]!="r"||parts[1]!=self.scope||parts[2]!=mode{return Err(changed());}
        let generation=parts[3].parse::<u64>().map_err(|_|unavailable())?;
        if generation!=if mode=="untimed"{self.generation}else{0}{return Err(changed());}
        if !matches!(parts[4],"n"|"p")||parts[5].is_empty()||parts[5].len()>20||!parts[5].bytes().all(|b|b.is_ascii_digit()){return Err(unavailable());}
        let n=parts[5].parse::<u64>().map_err(|_|unavailable())?;
        let at=i64::try_from(n).map_err(|_|unavailable())?;
        if n>self.count as u64{return Err(unavailable());}
        Ok(Cursor{at,backward:parts[4]=="p"})
    }
    fn token(&self,mode:&str,at:i64,backward:bool)->String{format!("r:{}:{}:{}:{}:{}",self.scope,mode,if mode=="untimed"{self.generation}else{0},if backward{"p"}else{"n"},at)}
    pub(super) fn select(&mut self,mode:&str,cursor:Option<&str>,limit:usize,now:Option<u64>,m:Option<&MeasuredAlignment>,duration:f64)->Result<Selection,CloudError>{
        let (total,untimed)=self.counts(m,duration)?;
        if !matches!(mode,"history"|"untimed"|"current"){return Err(unavailable());}
        let limit=limit.clamp(1,120);let c=self.cursor(mode,cursor)?;
        if mode=="current"&&cursor.is_some(){return Err(unavailable());}
        let projected=self.projection.as_ref();
        let low=projected.map_or(key(0),Projection::low);let high=projected.map_or(key(-1),Projection::high);
        let cap=projected.zip(now).map(|(p,t)|key(i128::from(t)+i128::from(p.lower))).unwrap_or(key(-1));
        let filter=if mode=="current"{"lo>=?2 AND hi<=?3 AND hi<=?4"}else if mode=="untimed"&&projected.is_some(){"(lo IS NULL OR lo<?2 OR hi>?3)"}else{"1"};
        let order=if mode=="current"{"hi DESC,ord DESC"}else if c.backward{"ord DESC"}else{"ord ASC"};
        let direction=if c.backward{"<="}else{">="};
        // Current uses the endpoint index and a bounded top-k reply, not a
        // materialized all-row eligibility vector. History is ordinal keyset.
        let sql=format!("SELECT ord,id,payload,t,n,reason,meta FROM rows WHERE ord{direction}?1 AND {filter} AND ?2 IS NOT NULL AND ?3 IS NOT NULL AND ?4 IS NOT NULL ORDER BY {order} LIMIT ?5");
        let mut selected=Vec::new();let mut body_bytes=0;
        if mode!="current"||projected.is_some()&&now.is_some(){
            let mut statement=self.db.prepare_cached(&sql).map_err(|_|self.sql_error())?;
            let mut query=statement.query(params![if mode=="current"{0}else{c.at},low.as_slice(),high.as_slice(),cap.as_slice(),(limit+1) as i64]).map_err(|_|self.sql_error())?;
            while let Some(r)=query.next().map_err(|_|self.sql_error())?{
                self.check()?;
                let ord:i64=r.get(0).map_err(|_|unavailable())?;let payload:String=r.get(2).map_err(|_|unavailable())?;
                if selected.len()>=limit||body_bytes+payload.len()>PAGE_BYTES{break;}body_bytes+=payload.len();
                let transport:Option<Vec<u8>>=r.get(3).map_err(|_|unavailable())?;
                let source=match transport{Some(t)=>Some(SourcePoint{transport90k:u64::from_be_bytes(t.try_into().map_err(|_|unavailable())?),numeric_uncertainty:r.get::<_,u32>(4).map_err(|_|unavailable())?}),None=>None};
                let interval=match (&source,m){(Some(p),Some(m))=>project_interval(p.transport90k,p.numeric_uncertainty,m,duration)?,_=>None};
                if mode=="current"&&!interval.as_ref().is_some_and(|p|now.is_some_and(|t|p.upper_ticks90k<=t)){return Err(unavailable());}
                if mode=="untimed"&&interval.is_some(){return Err(unavailable());}
                selected.push(Stored{ordinal:ord,row:Row{id:r.get(1).map_err(|_|unavailable())?,value:serde_json::from_str(&payload).map_err(|_|unavailable())?,source,reason:r.get(5).map_err(|_|unavailable())?},interval,meta:r.get(6).map_err(|_|unavailable())?});
            }
        }
        selected.sort_by_key(|r|r.ordinal);
        let first=selected.first().map(|r|r.ordinal);let last=selected.last().map(|r|r.ordinal);
        let exists=|at:i64,backward:bool|->Result<bool,CloudError>{
            let op=if backward{"<"}else{">"};
            let q=format!("SELECT EXISTS(SELECT 1 FROM rows WHERE ord{op}?1 AND {filter} AND ?2 IS NOT NULL AND ?3 IS NOT NULL LIMIT 1)");
            self.db.query_row(&q,params![at,low.as_slice(),high.as_slice()],|r|r.get(0)).map_err(|_|self.sql_error())
        };
        let next=if mode!="current"&&last.is_some_and(|last|last<i64::MAX)&&exists(last.unwrap_or(-1),false)?{Some(self.token(mode,last.map_or(c.at,|n|n+1),false))}else{None};
        let previous=if mode!="current"&&first.is_some_and(|first|first>0)&&exists(first.unwrap_or(0),true)?{Some(self.token(mode,first.unwrap()-1,true))}else{None};
        let next_at=if let (Some(p),Some(now))=(projected,now){
            let after=key(i128::from(now)+i128::from(p.upper));
            let lo:Option<Vec<u8>>=self.db.query_row("SELECT lo FROM rows WHERE lo>?1 AND lo>=?2 AND hi<=?3 ORDER BY lo,ord LIMIT 1",params![after.as_slice(),low.as_slice(),high.as_slice()],|r|r.get(0)).optional().map_err(|_|self.sql_error())?;
            lo.map(|b|{let raw=u128::from_be_bytes(b.try_into().map_err(|_|unavailable())?)^(1u128<<127);u64::try_from((raw as i128)-i128::from(p.upper)).map_err(|_|unavailable())}).transpose()?
        }else{None};
        let meta_id=selected.last().and_then(|r|r.meta).or(self.last_meta);
        let meta=meta_id.map(|id|{let text:String=self.db.query_row("SELECT body FROM metas WHERE id=?1",[id],|r|r.get(0)).map_err(|_|self.sql_error())?;serde_json::from_str(&text).map_err(|_|unavailable())}).transpose()?;
        self.check()?;Ok(Selection{rows:selected,next,previous,next_at,total,untimed,meta})
    }
}
#[cfg(test)]
#[path="live_replay_disk_index_tests.rs"]
mod tests;
