use super::*;
use crate::platform_media::soop_display::{Basis, Control, Evidence, Theme, Values, FORMAT};
const BASE: u64 = 1_700_000_000_000;
fn control(at: u64, seq: u64, theme: Theme) -> Value {
    serde_json::to_value(Control {
        format: FORMAT.into(),
        receiver_id: "90000000-0000-4000-8000-000000000001".into(),
        document_epoch: 1,
        sequence: seq,
        received_at_ms: at,
        observed_at_ms: Some(at),
        initial: seq == 1,
        values: Values {
            theme: Some(theme),
            fan_message_color: Some(true),
            ..Default::default()
        },
        basis: Evidence {
            theme: Some(Basis::Dom),
            fan_message_color: Some(Basis::SavedOption),
            ..Default::default()
        },
        palette: None,
        palette_incomplete: false,
        surface: None,
        surface_incomplete: false,
    })
    .unwrap()
}
fn row(at: u64) -> Value {
    serde_json::json!({"format":"atsumi-chat-v1","kind":"chat","receivedAtMs":at,"author":"Fixture","text":"public fixture","soopDisplay":{"fanMessageColor":false,"showEmoticons":true,"smallOgq":null}})
}
fn fixture(rows: &[Value], mapped: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if mapped {
        std::fs::write(dir.path().join("chat-timeline.json"),r#"{"timelineQuality":"receive-time-approximate","firstSegmentCommittedAtMs":1700000004000,"firstSegmentDurationMs":4000}"#).unwrap();
    }
    std::fs::write(
        dir.path().join("chat.events.jsonl"),
        rows.iter().map(|v| format!("{v}\n")).collect::<String>(),
    )
    .unwrap();
    dir
}
fn page(cache: &IndexCache, dir: &Path, provider: &str, at: u64) -> ReplayChatPage {
    for _ in 0..128 {
        let p = read_page_using(cache, dir, provider, at, None, 120, SLICE).unwrap();
        if p.state != "indexing" {
            return p;
        }
    }
    panic!("fixture index did not finish");
}
#[test]
fn initial_and_quiet_settings_seek_without_becoming_chat_rows() {
    let dir = fixture(
        &[
            control(BASE - 2000, 1, Theme::Light),
            row(BASE + 500),
            control(BASE + 1200, 2, Theme::Dark),
            control(BASE + 2700, 3, Theme::Light),
        ],
        true,
    );
    let cache = IndexCache::default();
    let p = page(&cache, dir.path(), "soop", 0);
    assert_eq!(p.state, "ready");
    assert_eq!(p.total, 1);
    assert!(p.items.is_empty());
    assert_eq!(p.next_at_ms, Some(500));
    let d = p.soop_display.unwrap();
    assert_eq!(d.time_ms, Some(0));
    assert_eq!(d.values.theme, Some(Theme::Light));
    let p = page(&cache, dir.path(), "soop", 1500);
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.next_at_ms, Some(2700));
    assert_eq!(p.soop_display.unwrap().values.theme, Some(Theme::Dark));
    assert_eq!(
        p.items[0].soop_display.as_ref().unwrap().fan_message_color,
        Some(false)
    );
    let earlier = page(&cache, dir.path(), "soop", 600);
    assert_eq!(earlier.next_at_ms, Some(1200));
    assert_eq!(
        earlier.soop_display.unwrap().values.theme,
        Some(Theme::Light)
    );
    assert_eq!(earlier.unsupported_items, 0);
    assert_eq!(earlier.malformed_lines, 0);
    drop(cache);
    let reopened = page(&IndexCache::default(), dir.path(), "soop", 2800);
    assert_eq!(reopened.soop_display.unwrap().sequence, 3);
}
#[test]
fn settings_only_archive_is_ready_and_unknown_time_is_never_zero() {
    for mapped in [true, false] {
        let dir = fixture(
            &[
                control(BASE - 1, 1, Theme::Dark),
                control(BASE + 500, 2, Theme::Light),
            ],
            mapped,
        );
        let cache = IndexCache::default();
        let p = page(&cache, dir.path(), "soop", 1000);
        assert_eq!(p.total, 0);
        assert!(p.items.is_empty());
        assert_eq!(p.unsupported_items, 0);
        assert_eq!(p.state, if mapped { "ready" } else { "unmapped" });
        let d = p.soop_display.unwrap();
        assert_eq!(d.time_ms, mapped.then_some(500));
        assert_eq!(
            d.timing_quality,
            if mapped {
                "receive-time-approximate"
            } else {
                "unmapped"
            }
        );
    }
}
#[test]
fn malformed_control_does_not_pollute_chat_counts_or_drop_healthy_row() {
    let mut bad = control(BASE, 1, Theme::Dark);
    bad["values"]["fontLevel"] = serde_json::json!(99);
    let dir = fixture(&[bad, row(BASE + 1)], true);
    let p = page(&IndexCache::default(), dir.path(), "soop", 1000);
    assert_eq!(p.total, 1);
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].text, "public fixture");
    assert_eq!(p.unsupported_items, 0);
    assert_eq!(p.malformed_lines, 0);
    assert!(p.soop_display_incomplete);
    assert!(p.soop_display.is_none());
    assert!(p.items[0].soop_display.is_some());
}
#[test]
fn bounded_global_history_overflow_keeps_chat_and_creation_state() {
    let mut rows = (0..4097)
        .map(|n| control(BASE + n, n + 1, Theme::Dark))
        .collect::<Vec<_>>();
    rows.push(row(BASE + 5000));
    let dir = fixture(&rows, true);
    let p = page(&IndexCache::default(), dir.path(), "soop", 6000);
    assert_eq!(p.total, 1);
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.unsupported_items, 0);
    assert!(p.soop_display_incomplete);
    assert!(p.soop_display.is_none());
    assert_eq!(
        p.items[0].soop_display.as_ref().unwrap().show_emoticons,
        Some(true)
    );
}
#[test]
fn legacy_and_other_provider_never_gain_soop_settings() {
    let mut legacy = row(BASE);
    legacy.as_object_mut().unwrap().remove("soopDisplay");
    let dir = fixture(&[legacy], true);
    let p = page(&IndexCache::default(), dir.path(), "soop", 1000);
    assert!(p.soop_display.is_none());
    assert!(p.items[0].soop_display.is_none());
    assert!(!p.soop_display_incomplete);
    let parsed = parse_line(&serde_json::to_vec(&row(BASE)).unwrap(), "youtube").unwrap();
    assert!(parsed.items[0].item.soop_display.is_none());
    assert_eq!(parsed.unsupported, 0);
    let parsed = parse_line(
        &serde_json::to_vec(&control(BASE, 1, Theme::Dark)).unwrap(),
        "youtube",
    )
    .unwrap();
    assert!(parsed.display.is_none());
    assert!(!parsed.display_invalid);
}
#[test]
fn malformed_creation_metadata_keeps_text_and_soop_badges() {
    let mut r = row(BASE);
    r["soopDisplay"]["showEmoticons"] = serde_json::json!("false");
    r["soop"] = serde_json::json!({"flag1":32,"flag2":0,"role":"fan","badge":"fan"});
    let baseline = parse_line(&serde_json::to_vec(&r).unwrap(), "soop").unwrap();
    assert_eq!(baseline.items.len(), 1);
    assert_eq!(baseline.items[0].item.text, "public fixture");
    assert!(baseline.items[0].item.soop_display.is_none());
    assert_eq!(baseline.unsupported, 1);
    assert_eq!(baseline.items[0].item.soop.as_ref().unwrap().badge, "fan");
    let mut clean = r.clone();
    clean.as_object_mut().unwrap().remove("soopDisplay");
    let clean = parse_line(&serde_json::to_vec(&clean).unwrap(), "soop").unwrap();
    assert_eq!(
        serde_json::to_value(&baseline.items[0].item.soop).unwrap(),
        serde_json::to_value(&clean.items[0].item.soop).unwrap()
    );
}

fn palette_control(at: u64, seq: u64, color: &str) -> Value {
    let mut c = control(at, seq, Theme::Dark);
    c["palette"] = serde_json::json!({"version":1,"basis":"computed-css-variable",
        "nickname":vec![Some(color);20],"roles":{"normal":color,"streamer":color,
        "manager":color,"vip":color,"subscribe":color,"fan":color,"support":color,"nightbot":color}});
    c
}
#[test]
fn quiet_palette_changes_restore_on_seek_reopen_without_changing_creation_or_chat() {
    let dir = fixture(
        &[
            palette_control(BASE - 2000, 1, "#112233"),
            row(BASE + 500),
            palette_control(BASE + 1200, 2, "#445566"),
            palette_control(BASE + 2700, 3, "#112233"),
        ],
        true,
    );
    let cache = IndexCache::default();
    let before = page(&cache, dir.path(), "soop", 0);
    assert_eq!(before.total, 1);
    assert!(before.items.is_empty());
    assert_eq!(
        before
            .soop_display
            .unwrap()
            .palette
            .unwrap()
            .roles
            .normal
            .as_deref(),
        Some("#112233")
    );
    let after = page(&cache, dir.path(), "soop", 1500);
    assert_eq!(after.next_at_ms, Some(2700));
    assert_eq!(after.total, 1);
    assert_eq!(
        after
            .soop_display
            .unwrap()
            .palette
            .unwrap()
            .roles
            .normal
            .as_deref(),
        Some("#445566")
    );
    let original_item = serde_json::to_value(&after.items[0]).unwrap();
    let backward = page(&cache, dir.path(), "soop", 600);
    assert_eq!(
        serde_json::to_value(&backward.items[0]).unwrap(),
        original_item
    );
    assert_eq!(
        backward
            .soop_display
            .unwrap()
            .palette
            .unwrap()
            .roles
            .normal
            .as_deref(),
        Some("#112233")
    );
    assert_eq!(backward.unsupported_items, 0);
    assert_eq!(backward.malformed_lines, 0);
    drop(cache);
    let reopened = page(&IndexCache::default(), dir.path(), "soop", 2800);
    assert_eq!(
        reopened
            .soop_display
            .unwrap()
            .palette
            .unwrap()
            .roles
            .normal
            .as_deref(),
        Some("#112233")
    );
    assert_eq!(
        serde_json::to_value(&reopened.items[0]).unwrap(),
        original_item
    );
}
#[test]
fn malformed_optional_palette_is_qualified_as_of_without_discarding_settings_or_rows() {
    let mut bad = palette_control(BASE + 1000, 2, "#112233");
    bad["palette"]["roles"]["normal"] = "url(https://invalid.test/x)".into();
    let dir = fixture(
        &[
            palette_control(BASE - 1, 1, "#112233"),
            row(BASE + 100),
            bad,
            palette_control(BASE + 2000, 3, "#445566"),
        ],
        true,
    );
    let cache = IndexCache::default();
    let good = page(&cache, dir.path(), "soop", 500).soop_display.unwrap();
    assert!(good.palette.is_some());
    assert!(!good.palette_incomplete);
    let bad = page(&cache, dir.path(), "soop", 1500);
    assert_eq!(bad.total, 1);
    assert_eq!(bad.items.len(), 1);
    assert_eq!(bad.unsupported_items, 0);
    assert!(!bad.soop_display_incomplete);
    let d = bad.soop_display.unwrap();
    assert!(d.palette.is_none());
    assert!(d.palette_incomplete);
    assert_eq!(d.values.theme, Some(Theme::Dark));
    assert_eq!(d.values.fan_message_color, Some(true));
    assert_eq!(
        bad.items[0]
            .soop_display
            .as_ref()
            .unwrap()
            .fan_message_color,
        Some(false)
    );
    let restored = page(&cache, dir.path(), "soop", 2200).soop_display.unwrap();
    assert!(restored.palette.is_some());
    assert!(!restored.palette_incomplete);
    assert!(
        !page(&cache, dir.path(), "soop", 500)
            .soop_display
            .unwrap()
            .palette_incomplete
    );
}
#[test]
fn legacy_and_other_provider_do_not_gain_palette_and_empty_palette_history_still_advances() {
    let dir = fixture(
        &[
            control(BASE - 1, 1, Theme::Dark),
            palette_control(BASE + 500, 2, "#112233"),
        ],
        true,
    );
    let cache = IndexCache::default();
    let old = page(&cache, dir.path(), "soop", 0);
    assert_eq!(old.total, 0);
    assert_eq!(old.next_at_ms, Some(500));
    let old = old.soop_display.unwrap();
    assert!(old.palette.is_none());
    assert!(!old.palette_incomplete);
    let p = page(&cache, dir.path(), "soop", 600);
    assert_eq!(p.state, "ready");
    assert_eq!(p.total, 0);
    assert!(p.soop_display.unwrap().palette.is_some());
    let other = parse_line(
        &serde_json::to_vec(&palette_control(BASE, 1, "#112233")).unwrap(),
        "youtube",
    )
    .unwrap();
    assert!(other.display.is_none());
    assert!(!other.display_invalid);
}

fn surface_control(at: u64, seq: u64, foreground: Option<&str>) -> Value {
    let mut c = palette_control(at, seq, "#123456");
    c["surface"] = serde_json::json!({"version":1,"basis":"computed-dom","foreground":foreground,"background":"#0c0d0e"});
    c
}
#[test]
fn surface_late_first_observation_seek_reopen_and_unknown_do_not_recolor_old_rows() {
    let mut malformed = surface_control(BASE + 2000, 3, Some("#e2e4e9"));
    malformed["surface"]["background"] = "transparent".into();
    let dir = fixture(&[
        control(BASE - 1, 1, Theme::Dark), row(BASE + 100),
        surface_control(BASE + 1000, 2, Some("#e2e4e9")), malformed,
        surface_control(BASE + 3000, 4, None), control(BASE + 4000, 5, Theme::Dark),
    ], true);
    let cache = IndexCache::default();
    let before = page(&cache, dir.path(), "soop", 500);
    assert_eq!(before.next_at_ms, Some(1000));
    assert!(before.soop_display.unwrap().surface.is_none());
    let original = serde_json::to_value(&before.items[0]).unwrap();
    for (at, expected, incomplete) in [(1500, Some("#e2e4e9"), false), (2500, None, true), (3500, None, false), (4500, None, false), (1500, Some("#e2e4e9"), false)] {
        let p = page(&cache, dir.path(), "soop", at);
        assert_eq!(p.total, 1); assert_eq!(p.unsupported_items, 0); assert_eq!(p.malformed_lines, 0);
        assert_eq!(serde_json::to_value(&p.items[0]).unwrap(), original);
        let d = p.soop_display.unwrap();
        assert_eq!(d.surface.as_ref().and_then(|s| s.foreground.as_deref()), expected);
        assert_eq!(d.surface_incomplete, incomplete);
        assert_eq!(d.values.theme, Some(Theme::Dark));
        if at < 4000 { assert!(d.palette.is_some()); }
    }
    drop(cache);
    let reopened = page(&IndexCache::default(), dir.path(), "soop", 1500);
    assert_eq!(reopened.soop_display.unwrap().surface.unwrap().foreground.as_deref(), Some("#e2e4e9"));
    let other = parse_line(&serde_json::to_vec(&surface_control(BASE, 1, Some("#e2e4e9"))).unwrap(), "youtube").unwrap();
    assert!(other.display.is_none() && !other.display_invalid);
}
