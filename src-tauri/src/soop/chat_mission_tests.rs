use super::*;
fn payload(kind:&str)->Value {serde_json::json!({"type":kind,"user_nick":"Public","user_id":"private-id","gift_count":19,"is_relay":1,"settle_count":0,"image":"2","fan_order":3,"top_fan":1,"title":"Saved mission","mission_status":"SUCCESS","winner":"Saved team"})}
fn decoded(v:&Value)->Result<PublicMission,()>{parse(&[&v.to_string()])}

#[test]
fn mission_six_original_branches_do_not_turn_settlement_or_notices_into_new_gifts(){
    for(kind,expected,event)in[("GIFT",Kind::Battle,Event::Gift),("SETTLE",Kind::Battle,Event::Settle),("NOTICE",Kind::Battle,Event::Notice),
        ("CHALLENGE_GIFT",Kind::Challenge,Event::Gift),("CHALLENGE_SETTLE",Kind::Challenge,Event::Settle),("CHALLENGE_NOTICE",Kind::Challenge,Event::Notice)]{
        let row=decoded(&payload(kind)).ok().unwrap();assert_eq!(row.metadata.kind,expected);assert_eq!(row.metadata.event,event);
        assert_eq!(row.metadata.row_kind(),if event==Event::Gift{"donation"}else{"system"});
        assert_eq!(row.nickname,if event==Event::Gift{"Public"}else{""});
        assert_eq!(row.metadata.count,match event{Event::Gift=>Some(19),Event::Settle=>Some(0),Event::Notice=>None});
        let encoded=serde_json::to_string(&row.metadata).unwrap();
        for secret in ["private-id","user_id","https://","gift_count","is_relay"]{assert!(!encoded.contains(secret));}
        assert!(!row.partial);
    }
}
#[test]
fn mission_notice_priority_and_challenge_status_follow_original(){
    let mut v=payload("NOTICE");v["draw"]=true.into();v["rank"]=2.into();v["my_team_name"]="Team".into();
    assert_eq!(decoded(&v).ok().unwrap().metadata.notice,Some(Notice::Draw));
    v["draw"]=false.into();assert_eq!(decoded(&v).ok().unwrap().metadata.notice,Some(Notice::Rank{rank:2,team:"Team".into()}));
    v["rank"]=0.into();assert_eq!(decoded(&v).ok().unwrap().metadata.notice,Some(Notice::Winner{winner:"Saved team".into()}));
    for status in ["SUCCESS","FAIL","OTHER_PUBLIC_STATUS"]{let mut v=payload("CHALLENGE_NOTICE");v["mission_status"]=status.into();
        assert_eq!(decoded(&v).ok().unwrap().metadata.notice,Some(Notice::Challenge{title:"Saved mission".into(),successful:status=="SUCCESS"}));}
}
#[test]
fn mission_malformed_payloads_bounded_without_dropping_partial_image_text(){
    assert!(parse(&[]).is_err());assert!(parse(&[&"x".repeat(16385)]).is_err());assert!(parse(&["[]"]).is_err());
    for kind in ["UNKNOWN","gift",""]{assert!(decoded(&payload(kind)).is_err());}
    for bad in [serde_json::json!(-1),serde_json::json!(4294967296u64),serde_json::json!(1.5),serde_json::json!("1e3"),serde_json::json!(0)]{
        let mut v=payload("GIFT");v["gift_count"]=bad;assert!(decoded(&v).is_err());}
    for bad in [Value::Null,serde_json::json!("../../private"),serde_json::json!("https://other/path"),serde_json::json!("x".repeat(65))]{
        let mut v=payload("GIFT");v["image"]=bad;let row=decoded(&v).ok().unwrap();assert!(row.partial);assert_eq!(row.metadata.count,Some(19));
        assert!(Request::from_fields(uuid::Uuid::new_v4(),&[&v.to_string()]).is_none());}
    let mut v=payload("GIFT");v["user_nick"]="bad\nname".into();assert!(decoded(&v).is_err());
}
#[test]
fn mission_image_identity_is_connection_bound_and_url_is_not_deserializable(){
    let ns=uuid::Uuid::new_v4();let v=payload("GIFT").to_string();
    let a=Request::from_fields(ns,&[&v]).unwrap();let b=Request::from_fields(ns,&[&v]).unwrap();let c=Request::from_fields(uuid::Uuid::new_v4(),&[&v]).unwrap();
    assert_eq!(a.id(),b.id());assert_ne!(a.id(),c.id());assert_eq!(a.url(),"https://res.sooplive.com/images/new_app/chat/mission/m_2.png");
    let challenge=Request::from_fields(ns,&[&payload("CHALLENGE_GIFT").to_string()]).unwrap();
    assert_eq!(challenge.url(),"https://res.sooplive.com/images/chat/mission/m_2.png");
    let mut row=decoded(&payload("GIFT")).ok().unwrap();assert!(a.apply(&mut row.metadata));assert!(!c.apply(&mut row.metadata));assert!(!challenge.matches(&row.metadata));
    let stored=serde_json::to_value(row.metadata).unwrap();assert!(!stored.to_string().contains("https://"));assert!(from_stored(&stored).is_some());
    for bad in ["https://res.sooplive.com/images/chat/mission/m_../x.png","https://res.sooplive.com/images/chat/mission/m_1.png?x=1","http://res.sooplive.com/images/chat/mission/m_1.png"]{assert!(canonical_url(bad).is_none());}
    let mut bad=stored;bad["imageAssetId"]="https://evil".into();assert!(invalid_stored(&bad));assert!(from_stored(&bad).unwrap().image_asset_id.is_none());
    bad["requestUrl"]="https://evil".into();assert!(from_stored(&bad).is_none());
}
