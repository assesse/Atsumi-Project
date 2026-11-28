use super::*;
use super::super::live_alignment::{AlignmentBinding,OffsetInterval90k};
use serde_json::json;
fn index()->DiskIndex{DiskIndex::new(Arc::new(AtomicBool::new(false)),uuid::Uuid::new_v4().to_string()).unwrap()}
fn row(i:u64,size:usize,t:Option<(u64,u32)>)->Row{Row{id:i.to_string(),value:json!({"id":i.to_string(),"text":format!("row-{i}:{}","가".repeat(size)),"appearance":{"color":"#18395a","assetId":"saved-original-asset"}}),source:t.map(|(transport90k,numeric_uncertainty)|SourcePoint{transport90k,numeric_uncertainty}),reason:t.is_none().then(||"missing_event_source_clock".into())}}
fn measured(lower:i64,upper:i64)->MeasuredAlignment{MeasuredAlignment{binding:AlignmentBinding{run_id:uuid::Uuid::new_v4(),source_id:uuid::Uuid::new_v4(),stream_id:uuid::Uuid::new_v4(),video_id:"testvideo01".into(),remote_source_id:uuid::Uuid::new_v4(),document_epoch:1,source_generation:1},offset:OffsetInterval90k{lower,upper},windows:Vec::new(),two_window_estimate:true,history_incomplete:false}}

#[test]
fn long_history_keeps_more_than_65536_rows_and_32mib_with_bounded_cache(){
    let mut db=index();let total=65_537u64;
    for start in (0..total).step_by(120){
        let rows=(start..(start+120).min(total)).map(|i|row(i,190,None)).collect();
        db.append(rows,Some(json!({"state":"ready","cursor":start.to_string(),"total":start,"timelineQuality":"receive-time-approximate","soopDisplayIncomplete":true})),Some(&format!("source:{start}"))).unwrap();
    }
    assert_eq!(db.count(),total as usize);
    let page_size:i64=db.db.query_row("PRAGMA page_size",[],|r|r.get(0)).unwrap();
    let pages:i64=db.db.query_row("PRAGMA page_count",[],|r|r.get(0)).unwrap();assert!(pages*page_size>32*1024*1024);
    assert_eq!(db.db.query_row("PRAGMA cache_size",[],|r|r.get::<_,i64>(0)).unwrap(),-2048);
    assert_eq!(db.db.query_row("SELECT COUNT(*) FROM metas",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    for start in [0,32_768,65_416]{
        let cursor=(start!=0).then(||db.token("history",start,false));
        let page=db.select("history",cursor.as_deref(),120,None,None,100.).unwrap();
        assert_eq!(page.rows[0].row.id,start.to_string());
        for r in &page.rows{assert_eq!(r.row.value,row(r.ordinal as u64,190,None).value);}
    }
    let mut cursor=None;let mut seen=0;
    loop{let p=db.select("untimed",cursor.as_deref(),120,None,None,100.).unwrap();for r in p.rows{assert_eq!(r.ordinal,seen);seen+=1;}cursor=p.next;if cursor.is_none(){break;}}
    assert_eq!(seen,total as i64);
    let tail=db.select("history",Some(&db.token("history",65_536,false)),120,None,None,100.).unwrap();
    assert_eq!(tail.rows.len(),1);assert!(tail.next.is_none());
    let prev=db.select("history",tail.previous.as_deref(),120,None,None,100.).unwrap();
    assert_eq!(prev.rows.first().unwrap().ordinal,65_416);assert_eq!(prev.rows.last().unwrap().ordinal,65_535);
}

#[test]
fn indexed_intervals_match_rust_for_unsigned_extremes_and_signed_offsets(){
    let mut db=index();let values=[None,Some((0,2)),Some((100,2)),Some((90_000,2)),Some((180_000,2)),Some((u64::MAX,32))];
    db.append(values.iter().enumerate().map(|(i,t)|row(i as u64,2,*t)).collect(),None,None).unwrap();
    for (lower,upper) in [(-9000,-4500),(0,0),(4500,9000)]{
        let m=measured(lower,upper);
        let expected:Vec<_>=values.iter().enumerate().filter_map(|(i,t)|t.and_then(|(t,n)|project_interval(t,n,&m,3.).unwrap()).map(|p|(i as i64,p))).collect();
        let (_,untimed)=db.counts(Some(&m),3.).unwrap();assert_eq!(untimed,values.len()-expected.len());
        let page=db.select("current",None,120,Some(270_000),Some(&m),3.).unwrap();
        assert_eq!(page.rows.len(),expected.len());
        for (r,(ord,p)) in page.rows.iter().zip(expected){assert_eq!(r.ordinal,ord);let got=r.interval.as_ref().unwrap();assert_eq!((got.lower_ticks90k,got.upper_ticks90k),(p.lower_ticks90k,p.upper_ticks90k));}
        let untimed=db.select("untimed",None,120,None,Some(&m),3.).unwrap();assert!(untimed.rows.iter().all(|r|r.interval.is_none()));
    }
    assert!(key(-1)<key(0));assert!(key(i128::from(u64::MAX))<key(i128::from(u64::MAX)+1));
}

#[test]
fn token_cursor_is_session_and_projection_bound_and_has_no_six_digit_cap(){
    let mut db=index();db.count=1_500_001;
    let token=db.token("history",1_200_000,false);assert_eq!(db.cursor("history",Some(&token)).unwrap().at,1_200_000);
    let other=index();assert!(other.cursor("history",Some(&token)).is_err());
    assert!(db.cursor("untimed",Some(&token)).is_err());
    let token=db.token("untimed",0,false);let m=measured(0,0);db.counts(Some(&m),20.).unwrap();
    assert!(db.cursor("untimed",Some(&token)).is_err());
    assert!(db.cursor("history",Some(&format!("r:{}:history:0:n:18446744073709551615",db.scope))).is_err());
    for i in 0..4_100{db.append(Vec::new(),None,Some(&format!("source:{i}"))).unwrap();}
    let before=db.count();assert!(db.append(vec![row(999,1,None)],None,Some("source:0")).is_err());assert_eq!(db.count(),before);
}

#[test]
fn cancellation_and_storage_failure_do_not_publish_partial_batch(){
    let cancel=Arc::new(AtomicBool::new(false));let mut db=DiskIndex::with_pages(cancel.clone(),uuid::Uuid::new_v4().to_string(),16).unwrap();
    db.append(vec![row(0,1,None)],None,Some("first")).unwrap();
    assert!(db.append(vec![row(1,40_000,None),row(2,40_000,None)],None,Some("failed")).is_err());
    assert_eq!(db.count(),1);assert_eq!(db.db.query_row("SELECT COUNT(*) FROM rows",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.db.query_row("SELECT COUNT(*) FROM cursors WHERE value='failed'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    cancel.store(true,Ordering::Release);assert!(db.append(vec![row(3,1,None)],None,None).is_err());assert!(db.select("history",None,10,None,None,10.).is_err());
    // SQL VM cancellation also interrupts one long-running query, independently
    // of the per-page checks. No sleep-based timing or detached worker needed.
    let result=db.db.query_row("WITH RECURSIVE seq(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM seq WHERE x<10000000) SELECT sum(x) FROM seq",[],|r|r.get::<_,i64>(0));assert!(result.is_err());
}

#[test]
fn full_cursor_store_is_not_reported_as_a_duplicate_page(){
    let mut db=DiskIndex::with_pages(Arc::new(AtomicBool::new(false)),uuid::Uuid::new_v4().to_string(),16).unwrap();
    let mut failure=None;
    for i in 0..10_000{if let Err(error)=db.append(Vec::new(),None,Some(&format!("source-{i:0100}"))){failure=Some(error);break;}}
    assert_eq!(failure.unwrap().code,"CLOUD_LIVE_HISTORY_STORAGE");assert_eq!(db.count(),0);
}

#[test]
fn counts_cache_and_current_query_do_not_read_all_payloads(){
    let mut db=index();let m=measured(0,0);
    for start in (0..10_000u64).step_by(120){db.append((start..(start+120).min(10_000)).map(|i|row(i,1,Some((i*90,1)))).collect(),None,None).unwrap();}
    assert_eq!(db.counts(Some(&m),11.).unwrap(),(10_000,1));
    let generation=db.generation;let cached=db.mapped;
    for _ in 0..20{assert_eq!(db.counts(Some(&m),11.).unwrap(),(10_000,1));assert_eq!(db.generation,generation);assert_eq!(db.mapped,cached);}
    let page=db.select("current",None,5,Some(450_000),Some(&m),11.).unwrap();
    assert_eq!(page.rows.iter().map(|r|r.ordinal).collect::<Vec<_>>(),vec![4995,4996,4997,4998,4999]);assert_eq!(page.next_at,Some(450_089));
    db.append(vec![row(10_000,1,None)],None,None).unwrap();assert_eq!(db.counts(Some(&m),11.).unwrap(),(10_001,2));
    let plan:String=db.db.query_row("EXPLAIN QUERY PLAN SELECT ord FROM rows WHERE hi<=?1 ORDER BY hi DESC,ord DESC LIMIT 120",[key(400_000).as_slice()],|r|r.get(3)).unwrap();assert!(plan.contains("by_hi"));
}

#[test]
fn current_uses_latest_playback_envelopes_then_restores_source_order(){
    let mut db=index();let m=measured(0,0);
    db.append(vec![row(0,1,Some((800,10))),row(1,1,Some((100,1))),row(2,1,Some((600,5))),row(3,1,Some((799,20)))],None,None).unwrap();
    // The overlapping fourth interval is not yet fully elapsed at810. The
    // two latest fully elapsed source windows are ord0 and2, not ord2 and3.
    let p=db.select("current",None,2,Some(810),Some(&m),1.).unwrap();
    assert_eq!(p.rows.iter().map(|r|r.ordinal).collect::<Vec<_>>(),vec![0,2]);
    assert_eq!(p.rows[0].interval.as_ref().unwrap().upper_ticks90k,810);
    let p=db.select("current",None,2,Some(820),Some(&m),1.).unwrap();
    assert_eq!(p.rows.iter().map(|r|r.ordinal).collect::<Vec<_>>(),vec![0,3]);
}
