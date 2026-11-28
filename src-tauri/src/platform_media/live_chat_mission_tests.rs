use super::*;
#[test]
fn mission_capability_crosses_native_bridge_and_normal_stop_keeps_local_image(){
    use crate::soop::chat::{gifts::assets::Context,missions};
    let mut png=std::io::Cursor::new(Vec::new());image::DynamicImage::new_rgb8(3,3).write_to(&mut png,image::ImageFormat::Png).unwrap();
    let image=png.into_inner();let expected=image.clone();let calls=Arc::new(std::sync::atomic::AtomicUsize::new(0));let counter=calls.clone();
    let mut chat=SharedChat::new();Arc::get_mut(&mut chat).unwrap().assets=SessionAssets::with_test_fetcher(move|url|{assert_eq!(url,"https://res.sooplive.com/images/chat/mission/m_2.png");counter.fetch_add(1,Ordering::AcqRel);Ok(image.clone())});
    let wire=serde_json::json!({"type":"CHALLENGE_SETTLE","settle_count":11,"image":"2"}).to_string();
    let request=Context::new().mission(&[&wire]).unwrap();let mut meta=missions::from_stored(&serde_json::json!({"kind":"challenge","event":"settle","count":11})).unwrap();assert!(request.apply(&mut meta));
    let root=tempfile::tempdir().unwrap();let id=uuid::Uuid::new_v4().to_string();chat.attach_recording_archive(root.path(),"soop",&id).unwrap();
    let raw=serde_json::to_vec(&serde_json::json!({"kind":"system","receivedAtMs":1700000001000u64,"messageId":"single-public-mission","nickname":"","text":"","extra":{"service":121,"soopMission":meta}})).unwrap();
    for _ in 0..2{chat.ingest_received("soop",&raw,None,Vec::new(),Vec::new(),None,None,Some(request.clone()),Instant::now()).unwrap();}
    let deadline=Instant::now()+Duration::from_secs(3);while !root.path().join("chat-assets").join(request.id()).exists(){assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(2));}
    let summary=chat.detach_archive().unwrap().unwrap().persist(root.path(),"soop",&id).unwrap();assert!(!summary.assets_incomplete);chat.stop();assert_eq!(calls.load(Ordering::Acquire),1);
    let events=fs::read_to_string(root.path().join("chat.events.jsonl")).unwrap();assert_eq!(events.lines().count(),1);assert!(!events.contains("https://")&&!events.contains("missionAsset"));
    let event:NormalizedChatEvent=serde_json::from_str(events.lines().next().unwrap()).unwrap();assert!(event.mission_asset.is_none());assert_eq!(event.soop_mission.as_ref(),Some(&meta));assert_eq!(event.kind,"system");assert!(event.author.is_empty()&&event.amount.is_none());
    let stored=super::super::chat_assets::replay_asset(root.path(),request.id()).unwrap().unwrap();use base64::Engine;assert_eq!(base64::engine::general_purpose::STANDARD.decode(stored.split_once(',').unwrap().1).unwrap(),expected);
}
