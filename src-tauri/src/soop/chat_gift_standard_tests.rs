use super::*;

fn fields(service: u16, count: u32, default: bool, file: &str) -> Vec<String> {
    let mut f = vec![String::new(); 15];
    let (channel, nick, amount, filename, flag) = match service {
        18 => (0, 2, 3, 7, 8),
        33 => (1, 4, 5, 8, 9),
        105 => (1, 3, 4, 12, 13),
        _ => unreachable!(),
    };
    f[channel] = "fixture(1)".into();
    f[nick] = "Public".into();
    f[amount] = count.to_string();
    f[filename] = file.into();
    f[flag] = if default { "1" } else { "0" }.into();
    f
}
fn request(context: &Context, service: u16, f: &[String]) -> Option<Request> {
    context.from_fields(service, &f.iter().map(String::as_str).collect::<Vec<_>>(), "FIXTURE")
}
fn metadata(service: u16, f: &[String]) -> super::super::SoopGiftMetadata {
    super::super::parse(service, &f.iter().map(String::as_str).collect::<Vec<_>>()).unwrap().unwrap().metadata
}

#[test]
fn all_official_step_boundaries_and_empty_filename_use_exact_urls() {
    let context = Context::new();
    for service in [18, 33] {
        for (count, step) in [(1,1),(9,1),(10,2),(49,2),(50,3),(99,3),(100,4),(499,4),(500,5),(999,5),(1000,6),(u32::MAX,6)] {
            for (default, file) in [(true, "unused-file"), (false, "")] {
                let f = fields(service, count, default, file);
                let r = request(&context, service, &f).unwrap();
                assert!(r.valid());
                assert_eq!(r.url(), format!("https://res.sooplive.com/new_player/items/ba_step{step}.png"));
                let mut meta = metadata(service, &f);
                assert!(r.apply(&mut meta));
                assert_eq!(meta.image_asset_id.as_deref(), Some(r.id()));
            }
        }
        assert!(request(&context, service, &fields(service, 0, true, "")).is_none());
    }
}

#[test]
fn video_always_uses_fixed_art_while_custom_selectors_keep_old_semantics() {
    let context = Context::new();
    for default in [false, true] {
        for file in ["", "ignored", "../malformed-optional"] {
            let f = fields(105, 17, default, file);
            let r = request(&context, 105, &f).unwrap();
            assert_eq!(r.url(), "https://res.sooplive.com/new_player/items/m_video_balloon.png");
            assert!(r.matches(&metadata(105, &f)));
        }
    }
    let mut f = fields(18, 50, true, "ignored");
    f[12] = "not-a-language".into(); f[13] = "?url=untrusted".into();
    assert!(request(&context, 18, &f).is_some());
    f[8] = "0".into(); f[7] = "50".into();
    assert!(request(&context, 18, &f).is_none());
    assert!(Request::from_fields(18, &f.iter().map(String::as_str).collect::<Vec<_>>(), "fixture").is_none());
    f[12] = "ko_KR".into(); f[13] = "123".into();
    let custom = Request::from_fields(18, &f.iter().map(String::as_str).collect::<Vec<_>>(), "fixture").unwrap();
    assert_eq!(custom.url(), "https://res.sooplive.com/new_player/items/m_balloon_50.png?v=123");
    assert_eq!(custom.id(), format!("{:x}", Sha256::digest(format!("soop-gift-v1:{}", custom.url()))));
}

#[test]
fn connection_namespaces_do_not_alias_and_apply_checks_art_and_existing_identity() {
    let first = Context::new(); let second = Context::new();
    let f = fields(18, 211, true, "");
    let a = request(&first, 18, &f).unwrap();
    let b = request(&second, 18, &f).unwrap();
    assert_eq!(a, request(&first, 18, &f).unwrap());
    assert_eq!(a.url(), b.url()); assert_ne!(a.id(), b.id());
    assert!(canonical_url(a.url()).is_none(), "custom URL validator must remain narrow");
    let mut wrong_step = metadata(18, &fields(18, 50, true, ""));
    let mut video = metadata(105, &fields(105, 211, true, ""));
    let mut custom = metadata(18, &fields(18, 211, false, "50"));
    for meta in [&mut wrong_step, &mut video, &mut custom] { assert!(!a.apply(meta)); assert!(meta.image_asset_id.is_none()); }
    let mut meta = metadata(18, &f); meta.image_asset_id = Some(b.id().into());
    assert!(!a.apply(&mut meta)); assert_eq!(meta.image_asset_id.as_deref(), Some(b.id()));
}

#[test]
fn wrong_channel_and_malformed_nonempty_custom_file_never_become_default() {
    let context = Context::new();
    for service in [18,33,105] {
        let f = fields(service, 211, true, "");
        assert!(context.from_fields(service, &f.iter().map(String::as_str).collect::<Vec<_>>(), "other").is_none());
    }
    for service in [18,33] {
        let f = fields(service, 211, false, "../bad");
        assert!(!metadata(service, &f).is_default);
        assert!(request(&context, service, &f).is_none());
    }
}

#[test]
fn standard_capability_validator_rejects_every_nonexact_url() {
    let context = Context::new();
    let r = request(&context, 18, &fields(18, 211, true, "")).unwrap();
    for url in [format!("{}?v=1",r.url()),format!("{}#x",r.url()),r.url().replace("https:","http:"),
        r.url().replace("res.sooplive.com","res.sooplive.com.evil"),r.url().replace("res.sooplive.com","user@res.sooplive.com"),
        r.url().replace("res.sooplive.com","res.sooplive.com:443"),r.url().replace("ba_step4","ba_step04"),
        r.url().replace("ba_step4","ba_step3"),r.url().replace("ba_step4","%62a_step4")] {
        let mut changed = r.clone(); changed.url = url; assert!(!changed.valid());
    }
}
