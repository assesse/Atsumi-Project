use super::*;
#[test]
fn same_video_owned_fullscreen_does_not_publish_ordinary_channel_event(){
    let ordinary=super::super::tests::session();let mut owned=super::super::tests::session();
    let owner=Uuid::new_v4();owned.owned_by=Some(owner);
    assert_eq!(ordinary.video,owned.video);
    for enabled in [true,false] {
        let (event,payload)=fullscreen_notice(&ordinary,enabled);
        assert_eq!(event,"youtube-original:fullscreen");assert_eq!(payload["channelId"],ordinary.video);
        assert_eq!(payload["enabled"],enabled);assert!(payload.get("owner").is_none());
        let (event,payload)=fullscreen_notice(&owned,enabled);
        assert_eq!(event,"youtube-owned-vod:fullscreen");assert_eq!(payload["owner"],owner.to_string());
        assert_eq!(payload["enabled"],enabled);assert!(payload.get("channelId").is_none());
        assert!(payload.get("token").is_none());
    }
}
#[test]
fn same_video_owned_document_cannot_be_aliased_or_controlled_with_public_tokens(){
    let mut ordinary=super::super::tests::session();ordinary.token="ordinary".into();
    let ordinary=Arc::new(ordinary);let mut owned=super::super::tests::session();
    owned.token="private-owned".into();owned.owned_by=Some(Uuid::new_v4());let owned=Arc::new(owned);
    let host=WatchHost::default();{
        let mut r=lock(&host.registry);r.sessions.insert(ordinary.token.clone(),ordinary.clone());
        r.sessions.insert(owned.token.clone(),owned.clone());r.tokens.insert("ordinary-alias".into(),ordinary.token.clone());
    }
    assert_eq!(host.count(),2);assert!(host.session("private-owned").is_err());
    assert!(Arc::ptr_eq(&host.session("ordinary-alias").unwrap(),&ordinary));
    assert!(Arc::ptr_eq(&reuse_or_admit(&lock(&host.registry),&owned.video,false).unwrap().unwrap(),&ordinary));
    let last=release_viewer(&mut lock(&host.registry),"ordinary-alias").unwrap();assert!(Arc::ptr_eq(&last,&ordinary));
    lock(&host.registry).sessions.remove(&ordinary.token);
    assert!(reuse_or_admit(&lock(&host.registry),&owned.video,false).is_err());
    assert!(reuse_or_admit(&lock(&host.registry),&owned.video,true).unwrap().is_none());
    assert!(release_viewer(&mut lock(&host.registry),"private-owned").is_none());
    assert!(lock(&host.registry).sessions.contains_key(&owned.token));
}
#[test]
fn owned_capture_requires_exact_native_samples_and_identity(){
    use crate::cloud_recording::live_fingerprint::FrameFeature;
    use super::super::frames::NativeFrame;
    let a=NativeFrameWindow{capture_id:Uuid::new_v4(),video_id:"abcdefghijk".into(),document_epoch:2,
        source_id:Uuid::new_v4(),source_generation:1,seek_generation:3,
        frames:(0..4).map(|i|NativeFrame{media_pts90k:i*7500,uncertainty_ticks:4500,
            feature:FrameFeature{gradient:[i as u32;4],mean:10,spread:20}}).collect()};
    let mut b=a.clone();b.frames=a.frames[1..3].to_vec();assert!(same_capture_subset(&a,&b));
    b.frames[0].feature.mean+=1;assert!(!same_capture_subset(&a,&b));
    b=a.clone();b.source_id=Uuid::new_v4();assert!(!same_capture_subset(&a,&b));
    b=a.clone();b.seek_generation+=1;assert!(!same_capture_subset(&a,&b));
    b=a.clone();b.capture_id=Uuid::new_v4();assert!(!same_capture_subset(&a,&b));
    b=a.clone();b.frames.clear();assert!(!same_capture_subset(&a,&b));
}
