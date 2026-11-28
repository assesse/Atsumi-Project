use super::*;
use std::time::Duration;
const ID: &str = "90000000-0000-4000-8000-000000000001";
const WALL: u64 = 1_700_000_000_000;
fn surface(color: &str) -> Value {
    serde_json::json!({"version":1,"basis":"computed-dom","foreground":color,"background":"#0c0d0e"})
}
fn wire(seq: u64, surface: Value) -> Value {
    serde_json::json!({"sequence":seq,"actualChannel":"fixture","actualBroadcast":"123",
        "values":{"theme":"dark","fontLevel":8,"randomNicknameColor":false,"fanMessageColor":false,"showEmoticons":false,"animateEmoticons":false,"smallOgq":false},
        "basis":{"theme":"dom","fontLevel":"dom","randomNicknameColor":"dom","fanMessageColor":"saved-option","showEmoticons":"saved-option","animateEmoticons":"saved-option","smallOgq":"saved-option"},
        "surface":surface})
}
fn observation(seq: u64, value: Value) -> Observation {
    parse(wire(seq, value), "fixture", "123").unwrap()
}
#[test]
fn missing_nullable_and_partial_surface_are_independent_unknowns() {
    let mut absent = wire(1, Value::Null);
    absent.as_object_mut().unwrap().remove("surface");
    for value in [absent, wire(1, Value::Null)] {
        let o = parse(value, "fixture", "123").unwrap();
        assert!(o.surface.is_none() && !o.surface_incomplete);
    }
    for field in ["foreground", "background"] {
        let mut value = surface("#e2e4e9");
        value[field] = Value::Null;
        let o = observation(1, value.clone());
        assert!(!o.surface_incomplete);
        assert_eq!(serde_json::to_value(o.surface).unwrap(), value);
    }
    let mut forged = wire(1, surface("#e2e4e9"));
    forged["surfaceIncomplete"] = true.into();
    assert!(parse(forged, "fixture", "123").is_none());
    assert!(parse(wire(1, surface("#e2e4e9")), "other", "123").is_none());
}
#[test]
fn malformed_surface_quarantines_only_surface_and_cannot_inject_css() {
    let mut cases = vec![Value::Bool(true), serde_json::json!([])];
    for color in [
        "red",
        "#E2E4E9",
        "#abc",
        "#aabbccdd",
        "rgba(1,2,3,.5)",
        "url(https://invalid.test/x)",
        "var(--x)",
        "#abcdef;display:none",
    ] {
        for field in ["foreground", "background"] {
            let mut value = surface("#e2e4e9");
            value[field] = color.into();
            cases.push(value);
        }
    }
    for key in ["version", "basis", "foreground", "background"] {
        let mut value = surface("#e2e4e9");
        value.as_object_mut().unwrap().remove(key);
        cases.push(value);
    }
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("basis", serde_json::json!("guessed")),
        ("extra", Value::Null),
        ("foreground", Value::String("a".repeat(16384))),
    ] {
        let mut bad = surface("#e2e4e9");
        bad[key] = value;
        cases.push(bad);
    }
    for bad in cases {
        let o = observation(1, bad);
        assert!(o.surface.is_none() && o.surface_incomplete);
        assert_eq!(o.values.theme, Some(Theme::Dark));
        let mut capture = Capture::default();
        let t = Instant::now();
        capture.reset(ID, 1, t, WALL);
        capture.observe(ID, 1, o, t, WALL);
        let original = serde_json::to_value(capture.initial().unwrap()).unwrap();
        let page = Control::parse(&original).unwrap().page(Some(0));
        assert!(page.surface.is_none() && page.surface_incomplete);
        assert_eq!(page.values.theme, Some(Theme::Dark));
    }
    let mut capture = Capture::default();
    let t = Instant::now();
    capture.reset(ID, 1, t, WALL);
    capture.observe(ID, 1, observation(1, surface("#e2e4e9")), t, WALL);
    let mut value = serde_json::to_value(capture.initial().unwrap()).unwrap();
    value["surfaceIncomplete"] = true.into();
    assert!(Control::parse(&value).unwrap().surface.is_none());
}
#[test]
fn surface_changes_clear_on_absence_epoch_reset_and_bounded_overflow() {
    let mut c = Capture::default();
    let t = Instant::now();
    c.reset(ID, 1, t, WALL);
    c.take_pending();
    for (n, value) in [
        (1, surface("#112233")),
        (2, surface("#445566")),
        (3, surface("#445566")),
        (4, Value::Null),
    ] {
        assert!(c.observe(
            ID,
            1,
            observation(n, value),
            t + Duration::from_millis(n),
            WALL + n
        ));
    }
    let rows = c.take_pending();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0].1.surface.as_ref().unwrap().foreground.as_deref(),
        Some("#112233")
    );
    assert_eq!(
        rows[1].1.surface.as_ref().unwrap().foreground.as_deref(),
        Some("#445566")
    );
    assert!(rows[2].1.surface.is_none());
    c.observe(ID, 1, observation(5, surface("#112233")), t, WALL);
    c.reset(ID, 2, t, WALL);
    assert!(c.initial().unwrap().surface.is_none());
    assert!(!c.observe(ID, 1, observation(6, surface("#112233")), t, WALL));
    for n in 1..=70 {
        c.observe(
            ID,
            2,
            observation(n, surface(if n % 2 == 0 { "#112233" } else { "#445566" })),
            t,
            WALL,
        );
    }
    assert_eq!(c.pending.len(), HISTORY);
    assert!(c.initial().unwrap().surface.is_none());
    assert!(c.take_pending().last().unwrap().1.surface.is_none());
}
#[test]
fn full_settings_palette_and_surface_fit_existing_control_budget() {
    let mut value = wire(9_007_199_254_740_991, surface("#ffffff"));
    value["palette"] = serde_json::json!({"version":1,"basis":"computed-css-variable","nickname":vec![Some("#ffffff");20],"roles":{"normal":"#ffffff","streamer":"#ffffff","manager":"#ffffff","vip":"#ffffff","subscribe":"#ffffff","fan":"#ffffff","support":"#ffffff","nightbot":"#ffffff"}});
    let mut c = Capture::default();
    let t = Instant::now();
    c.reset(ID, u64::MAX, t, 9_000_000_000_000);
    c.observe(
        ID,
        u64::MAX,
        parse(value, "fixture", "123").unwrap(),
        t,
        9_000_000_000_000,
    );
    let mut control = c.initial().unwrap();
    control.initial = false;
    control.values.theme = Some(Theme::Light);
    let bytes = serde_json::to_vec(&control).unwrap();
    assert!(
        bytes.len() <= MAX_CONTROL_BYTES,
        "combined control bytes={}",
        bytes.len()
    );
    let parsed = Control::parse(&serde_json::from_slice(&bytes).unwrap()).unwrap();
    assert_eq!(parsed.palette, control.palette);
    assert_eq!(parsed.surface, control.surface);
    let mut damaged = serde_json::to_value(control).unwrap();
    damaged["surface"]["foreground"] = "red".into();
    let parsed = Control::parse(&damaged).unwrap();
    assert!(parsed.palette.is_some());
    assert!(parsed.surface.is_none() && parsed.surface_incomplete);
}
