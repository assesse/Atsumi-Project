use super::*;
fn packet(value:&Value)->Vec<u8>{let body=format!("\x0c{}\x0c",value);format!("\x1d\x09{:04}{:06}00{ZERO_ID}{body}",121,body.len()).into_bytes()}
#[test]
fn mission_public_capture_attaches_only_bound_image_capability_and_keeps_system_semantics(){
    let started=Instant::now();let mut summary=ChatSummary::default();let mut seen=Seen::default();let mut rows=Vec::new();
    let context=gifts::assets::Context::new();
    for kind in ["GIFT","SETTLE","NOTICE","CHALLENGE_GIFT","CHALLENGE_SETTLE","CHALLENGE_NOTICE"]{
        let v=serde_json::json!({"type":kind,"user_nick":"Public","user_id":"private-user","gift_count":11,"settle_count":10,"is_relay":false,"image":"1","winner":"Public team","title":"Public mission","mission_status":"SUCCESS"});
        let bytes=packet(&v);let frame=parse(&bytes).ok().unwrap();
        receive_public_assets(&frame,started,&mut|row|{rows.push(row);Ok(())},&mut summary,&mut seen,Some("bound-public-channel"),None,None,Some(&context)).ok().unwrap();
    }
    assert_eq!(summary.messages,6);assert_eq!(summary.unsupported_rich_events,0);assert_eq!(summary.malformed_rich_events,0);
    assert_eq!(rows.iter().filter(|r|r.kind=="donation").count(),2);assert_eq!(rows.iter().filter(|r|r.kind=="system").count(),4);
    assert_eq!(rows.iter().filter(|r|r.mission_asset.is_some()).count(),4);
    for row in &rows{assert!(row.gift_asset.is_none());if row.kind=="system"{assert!(row.nickname.is_empty());}}
    let stored=serde_json::to_string(&rows).unwrap();for secret in ["private-user","bound-public-channel","https://","missionAsset","user_id"]{assert!(!stored.contains(secret));}
}
