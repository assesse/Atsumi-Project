use super::*;
fn raw(kind:&str,meta:Value)->Value{serde_json::json!({"kind":kind,"receivedAtMs":1700000001000u64,"messageId":"public-message","nickname":"Public","text":"legacy","extra":{"service":121,"soopMission":meta}})}
#[test]
fn mission_roundtrip_preserves_clock_local_asset_and_provider_isolation(){
    for(kind,meta)in[("donation",serde_json::json!({"kind":"battle","event":"gift","count":11,"relay":false,"imageAssetId":"a".repeat(64)})),
        ("system",serde_json::json!({"kind":"challenge","event":"settle","count":0,"imageAssetId":"b".repeat(64)})),
        ("system",serde_json::json!({"kind":"battle","event":"notice","notice":{"result":"draw"}}))]{
        let events=normalize_public_line("soop",&serde_json::to_vec(&raw(kind,meta)).unwrap()).unwrap();assert_eq!(events.len(),1);
        let event=&events[0];assert_eq!(event.kind,kind);assert!(event.amount.is_none());assert_eq!(event.source_timestamp_ms,Some(1700000001000));
        assert_eq!(event.message_id.as_ref().unwrap().len(),64);if kind=="system"{assert!(event.author.is_empty());}
        let stored=serde_json::to_vec(event).unwrap();let replay=parse_line(&stored,"soop").unwrap();assert_eq!(replay.unsupported,0);
        assert_eq!(replay.items[0].item.soop_mission,event.soop_mission);
        let other=parse_line(&stored,"youtube").unwrap();assert!(other.items.iter().all(|row|row.item.soop_mission.is_none()));
        let mut forged=serde_json::to_value(event).unwrap();forged["missionAsset"]=serde_json::json!({"url":"https://private"});
        assert!(serde_json::from_value::<NormalizedChatEvent>(forged).unwrap().mission_asset.is_none());
    }
}
#[test]
fn mission_missing_optional_image_retains_event_but_wrong_source_or_variant_is_not_a_donation(){
    let mut value=raw("system",serde_json::json!({"kind":"battle","event":"settle","count":12,"imageAssetId":"https://evil"}));
    let parsed=parse_line(&serde_json::to_vec(&value).unwrap(),"soop").unwrap();assert_eq!(parsed.unsupported,1);assert_eq!(parsed.items.len(),1);
    assert!(parsed.items[0].item.soop_mission.as_ref().unwrap().image_asset_id.is_none());assert!(parsed.items[0].item.text.contains("12개 획득"));
    value["kind"]="donation".into();assert!(parse_line(&serde_json::to_vec(&value).unwrap(),"soop").unwrap().items.is_empty());
    value["kind"]="system".into();value["extra"]["service"]=144.into();assert!(parse_line(&serde_json::to_vec(&value).unwrap(),"soop").unwrap().items.is_empty());
    let invalid=serde_json::json!({"format":"atsumi-chat-v1","kind":"system","text":"legacy saved notice","soopMission":{"kind":"battle","event":"settle","count":-1}});
    let parsed=parse_line(&serde_json::to_vec(&invalid).unwrap(),"soop").unwrap();assert_eq!(parsed.unsupported,1);assert_eq!(parsed.items[0].item.text,"legacy saved notice");assert!(parsed.items[0].item.soop_mission.is_none());
    for metadata in [None,Some(Value::Null)] {
        let mut unknown=serde_json::json!({"format":"atsumi-chat-v1","kind":"system","text":"unverified non-mission system event"});
        if let Some(metadata)=metadata {unknown["soopMission"]=metadata;}
        let parsed=parse_line(&serde_json::to_vec(&unknown).unwrap(),"soop").unwrap();assert_eq!(parsed.unsupported,1);assert!(parsed.items.is_empty());
    }
    for (kind,mission) in [
        ("donation",serde_json::json!({"kind":"battle","event":"settle","count":100})),
        ("donation",serde_json::json!({"kind":"battle","event":"notice","notice":{"result":"draw"}})),
        ("system",serde_json::json!({"kind":"challenge","event":"gift","count":100,"relay":false})),
        ("chat",serde_json::json!({"kind":"battle","event":"gift","count":100,"relay":false})),
    ] {
        let inconsistent=serde_json::json!({"format":"atsumi-chat-v1","kind":kind,"author":"must not become a donor","amount":"100","text":"legacy","soopMission":mission});
        let parsed=parse_line(&serde_json::to_vec(&inconsistent).unwrap(),"soop").unwrap();assert_eq!(parsed.unsupported,1);assert!(parsed.items.is_empty());
    }
}
