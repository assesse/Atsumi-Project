use super::*;
use std::time::Duration;
const ID: &str = "90000000-0000-4000-8000-000000000001";
const WALL: u64 = 1_700_000_000_000;

fn palette(color: &str) -> Value {
    serde_json::json!({"version":1,"basis":"computed-css-variable",
        "nickname":vec![Some(color);20],
        "roles":{"normal":color,"streamer":color,"manager":color,"vip":color,
        "subscribe":color,"fan":color,"support":color,"nightbot":color}})
}
fn wire(palette: Value) -> Value {
    serde_json::json!({"sequence":1,"actualChannel":"fixture","actualBroadcast":"123",
        "values":{"theme":"dark","fontLevel":8,"randomNicknameColor":false,
        "fanMessageColor":false,"showEmoticons":false,"animateEmoticons":false,"smallOgq":false},
        "basis":{"theme":"dom","fontLevel":"dom","randomNicknameColor":"dom",
        "fanMessageColor":"saved-option","showEmoticons":"saved-option",
        "animateEmoticons":"saved-option","smallOgq":"saved-option"},"palette":palette})
}
fn observe(seq: u64, palette: Value) -> Observation {
    let mut value = wire(palette);
    value["sequence"] = seq.into();
    parse(value, "fixture", "123").unwrap()
}
#[test]
fn exact_palette_roundtrips_and_nullable_entries_are_not_corruption() {
    let mut value = palette("#abcdef");
    value["nickname"][4] = Value::Null;
    value["roles"]["normal"] = Value::Null;
    let o = observe(1, value.clone());
    assert!(!o.palette_incomplete);
    assert_eq!(serde_json::to_value(o.palette).unwrap(), value);
    for unknown in [None, Some(Value::Null)] {
        let mut value = wire(Value::Null);
        if unknown.is_none() {
            value.as_object_mut().unwrap().remove("palette");
        }
        let o = parse(value, "fixture", "123").unwrap();
        assert!(o.palette.is_none());
        assert!(!o.palette_incomplete);
        assert_eq!(o.values.theme, Some(Theme::Dark));
    }
}
#[test]
fn malformed_palette_cannot_inject_css_or_discard_healthy_preferences() {
    let mut cases = Vec::new();
    for color in [
        "red",
        "#ABCDEF",
        "#abc",
        "#abcd1234",
        "rgb(0,0,0)",
        "var(--x)",
        "url(https://invalid.test/x)",
        "#abcdef;display:none",
        "#абсdef",
    ] {
        let mut v = palette("#abcdef");
        v["nickname"][0] = color.into();
        cases.push(v);
    }
    let mut oversized = palette("#abcdef");
    oversized["roles"]["fan"] = "a".repeat(16_384).into();
    cases.push(oversized);
    let mut missing = palette("#abcdef");
    missing["roles"].as_object_mut().unwrap().remove("fan");
    cases.push(missing);
    let mut extra = palette("#abcdef");
    extra["roles"]["unknown"] = Value::Null;
    cases.push(extra);
    let mut extra = palette("#abcdef");
    extra["css"] = "display:none".into();
    cases.push(extra);
    for n in [0, 19, 21] {
        let mut v = palette("#abcdef");
        v["nickname"] = serde_json::json!(vec![Some("#abcdef"); n]);
        cases.push(v);
    }
    for (key, val) in [
        ("version", serde_json::json!(2)),
        ("basis", serde_json::json!("guessed")),
    ] {
        let mut v = palette("#abcdef");
        v[key] = val;
        cases.push(v);
    }
    for bad in cases {
        let o = observe(1, bad);
        assert!(o.palette.is_none());
        assert!(o.palette_incomplete);
        assert_eq!(o.values.theme, Some(Theme::Dark));
        assert_eq!(o.values.fan_message_color, Some(false));
        let mut c = Capture::default();
        let t = Instant::now();
        c.reset(ID, 1, t, WALL);
        c.observe(ID, 1, o, t, WALL);
        let control = c.initial().unwrap();
        let encoded = serde_json::to_vec(&control).unwrap();
        assert!(encoded.len() <= MAX_CONTROL_BYTES);
        let restored = Control::parse(&serde_json::from_slice(&encoded).unwrap())
            .unwrap()
            .page(Some(0));
        assert!(restored.palette.is_none());
        assert!(restored.palette_incomplete);
        assert_eq!(restored.values.theme, Some(Theme::Dark));
    }
    let mut untrusted = wire(palette("#abcdef"));
    untrusted["paletteIncomplete"] = true.into();
    assert!(parse(untrusted, "fixture", "123").is_none());
}
#[test]
fn palette_only_changes_are_recorded_and_source_reset_clears_old_colors() {
    let t = Instant::now();
    let mut c = Capture::default();
    c.reset(ID, 1, t, WALL);
    c.take_pending();
    assert!(c.observe(ID, 1, observe(1, palette("#112233")), t, WALL));
    let original_creation = c.creation(t);
    assert!(c.observe(
        ID,
        1,
        observe(2, palette("#445566")),
        t + Duration::from_millis(1),
        WALL + 1
    ));
    assert!(c.observe(
        ID,
        1,
        observe(3, palette("#445566")),
        t + Duration::from_millis(2),
        WALL + 2
    ));
    let rows = c.take_pending();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].1.palette.as_ref().unwrap().roles.fan.as_deref(),
        Some("#112233")
    );
    assert_eq!(
        rows[1].1.palette.as_ref().unwrap().roles.fan.as_deref(),
        Some("#445566")
    );
    assert_eq!(c.creation(t + Duration::from_millis(2)), original_creation);
    c.reset(ID, 2, t + Duration::from_millis(3), WALL + 3);
    assert!(!c.observe(
        ID,
        1,
        observe(4, palette("#112233")),
        t + Duration::from_millis(4),
        WALL + 4
    ));
    let unknown = c.initial().unwrap();
    assert!(unknown.palette.is_none());
    assert!(!unknown.palette_incomplete);
    assert_eq!(unknown.values, Values::default());
    for n in 1..=70 {
        c.observe(
            ID,
            2,
            observe(n, palette(if n % 2 == 0 { "#112233" } else { "#445566" })),
            t + Duration::from_millis(10 + n),
            WALL + 10 + n,
        );
    }
    assert_eq!(c.pending.len(), HISTORY);
    let overflow = c.take_pending();
    let unknown = &overflow.last().unwrap().1;
    assert!(unknown.palette.is_none());
    assert!(!unknown.palette_incomplete);
    assert!(c.initial().unwrap().palette.is_none());
}
#[test]
fn full_worst_case_control_preserves_existing_2048_byte_budget() {
    let mut c = Capture::default();
    let t = Instant::now();
    c.reset(ID, u64::MAX, t, 9_000_000_000_000);
    c.observe(
        ID,
        u64::MAX,
        observe(9_007_199_254_740_991, palette("#ffffff")),
        t,
        9_000_000_000_000,
    );
    let mut control = c.initial().unwrap();
    control.initial = false;
    control.values.theme = Some(Theme::Light);
    let bytes = serde_json::to_vec(&control).unwrap();
    assert!(
        bytes.len() <= MAX_CONTROL_BYTES,
        "full persisted control exceeded existing budget"
    );
    let parsed = Control::parse(&serde_json::from_slice(&bytes).unwrap()).unwrap();
    assert_eq!(parsed.palette, control.palette);
    assert_eq!(parsed.document_epoch, u64::MAX);
}
