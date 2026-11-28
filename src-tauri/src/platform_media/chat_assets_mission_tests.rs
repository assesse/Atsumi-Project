use super::*;
use crate::soop::chat::{gifts::assets::Context,missions};
fn mission(context:&Context)->(missions::Metadata,missions::Request){
    let value=serde_json::json!({"type":"GIFT","user_nick":"Public","gift_count":23,"is_relay":false,"image":"2"}).to_string();
    let request=context.mission(&[&value]).unwrap();let mut meta=missions::from_stored(&serde_json::json!({"kind":"battle","event":"gift","count":23,"relay":false})).unwrap();assert!(request.apply(&mut meta));(meta,request)
}
fn png(value:u8)->Blob{let image=image::RgbImage::from_pixel(3,3,image::Rgb([value,0,0]));let mut data=std::io::Cursor::new(Vec::new());image::DynamicImage::ImageRgb8(image).write_to(&mut data,image::ImageFormat::Png).unwrap();blob(data.into_inner(),Some("image/png")).unwrap()}
fn until(done:impl Fn()->bool){let end=Instant::now()+Duration::from_secs(3);while !done(){assert!(Instant::now()<end,"bounded mission asset fixture");std::thread::sleep(Duration::from_millis(2));}}
#[test]
fn mission_image_immutable_bytes_deduplicate_only_within_the_bound_connection(){
    let(a,ra)=mission(&Context::new());let(b,rb)=mission(&Context::new());assert_eq!(ra.url(),rb.url());assert_ne!(ra.id(),rb.id());
    let calls=Arc::new(AtomicUsize::new(0));let c=calls.clone();let mut session=SessionAssets::new();session.fetcher=Arc::new(move|source|{assert!(source.valid());assert_eq!(source.url(),"https://res.sooplive.com/images/new_app/chat/mission/m_2.png");Ok(png(if c.fetch_add(1,Ordering::AcqRel)==0{20}else{220}))});
    let root=tempfile::tempdir().unwrap();let target=ArchiveAssets::new(root.path()).unwrap();
    for _ in 0..4{session.observe_mission(&a,Some(&ra),Some(&target));}until(||target.state.lock().unwrap().stored.contains(ra.id()));
    session.observe_mission(&b,Some(&rb),Some(&target));until(||target.state.lock().unwrap().stored.len()==2);
    assert_eq!(calls.load(Ordering::Acquire),2);assert!(!target.close());session.stop();until(||session.is_done());drop(target);
    assert_eq!(replay_asset(root.path(),ra.id()).unwrap(),Some(data_url(&png(20))));assert_eq!(replay_asset(root.path(),rb.id()).unwrap(),Some(data_url(&png(220))));
    let manifest=fs::read_to_string(root.path().join("chat-assets/manifest.jsonl")).unwrap();assert_eq!(manifest.lines().count(),2);assert!(!manifest.contains("https://"));
}
#[test]
fn mission_missing_or_forged_capabilities_cannot_fetch_current_artwork(){
    let mut session=SessionAssets::new();session.fetcher=Arc::new(|_|panic!("saved JSON cannot grant network"));
    let root=tempfile::tempdir().unwrap();let target=ArchiveAssets::new(root.path()).unwrap();let(mut m,request)=mission(&Context::new());
    session.observe_mission(&m,None,Some(&target));m.image_asset_id=None;session.observe_mission(&m,None,Some(&target));
    session.observe(&[ChatPart::Emoji{asset_id:asset_id(request.url()),alt:String::new(),source_url:Some(request.url().into())}],Some(&target));
    assert!(session.core.state.lock().unwrap().entries.is_empty());assert!(session.incomplete());assert!(target.close());session.stop();
}
