use super::*;
use crate::platform_media::{
    relay_chat::{Binding, RelayChatLease},
    soop_display::{Basis, Evidence, Observation, Values},
};
const BASE: u64 = 1_700_000_000_000;
fn observation(seq: u64, on: bool) -> Observation {
    Observation {
        sequence: seq,
        values: Values {
            fan_message_color: Some(on),
            show_emoticons: Some(on),
            ..Default::default()
        },
        basis: Evidence {
            fan_message_color: Some(Basis::Dom),
            show_emoticons: Some(Basis::Dom),
            ..Default::default()
        },
        palette: None,
        palette_incomplete: false,
        surface: None,
        surface_incomplete: false,
    }
}
fn controls(dir: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(dir.join("chat.events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn finish(relay: &mut RelayChatLease) -> super::super::relay_chat::RelayChatSummary {
    relay.begin_finish(false);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(summary) = relay.poll_join().unwrap() {
            return summary;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn recording_and_relay_flush_quiet_changes_and_initial_snapshot_independently() {
    let chat = SharedChat::new();
    let receiver = uuid::Uuid::new_v4().to_string();
    chat.reset_display(&receiver, 1);
    chat.observe_display(&receiver, 1, observation(1, true));
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("archive");
    fs::create_dir(&archive).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(&archive, "soop", &id)
        .unwrap();
    let binding = Binding {
        receiver_id: receiver.clone(),
        run_id: uuid::Uuid::new_v4().to_string(),
        source_id: uuid::Uuid::new_v4().to_string(),
        document_epoch: 1,
        channel_id: "fixture".into(),
        broadcast_id: "123".into(),
    };
    let mut relay = RelayChatLease::attach(chat.clone(), None, binding, root.path()).unwrap();
    chat.observe_display(&receiver, 1, observation(2, false));
    chat.detach_archive()
        .unwrap()
        .unwrap()
        .persist(&archive, "soop", &id)
        .unwrap();
    chat.observe_display(&receiver, 1, observation(3, true));
    let summary = finish(&mut relay);
    let original = controls(&archive);
    let relayed = controls(&summary.directory);
    assert_eq!(original.len(), 2);
    assert_eq!(relayed.len(), 3);
    assert!(original[0]["initial"].as_bool().unwrap());
    assert!(relayed[0]["initial"].as_bool().unwrap());
    assert_eq!(original[1]["values"]["showEmoticons"], false);
    assert_eq!(relayed[2]["values"]["showEmoticons"], true);
    assert_eq!(summary.raw_rows, 0);
    assert_eq!(summary.event_rows, 0);
    assert!(summary.joined);
    assert!(summary.failure.is_none());
    assert!(!chat.stop.load(Ordering::Acquire));
    chat.stop();
}
#[test]
fn stored_creation_uses_native_packet_receipt_even_after_later_change() {
    let chat = SharedChat::new();
    let receiver = uuid::Uuid::new_v4().to_string();
    let t = Instant::now();
    {
        let mut display = chat.display.lock().unwrap();
        display.reset(&receiver, 1, t, BASE);
        display.observe(
            &receiver,
            1,
            observation(1, true),
            t + Duration::from_millis(10),
            BASE + 10,
        );
        display.observe(
            &receiver,
            1,
            observation(2, false),
            t + Duration::from_millis(20),
            BASE + 20,
        );
    }
    let root = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(root.path(), "soop", &id)
        .unwrap();
    let line=serde_json::to_vec(&serde_json::json!({"kind":"chat","receivedAtMs":BASE+15,"nickname":"Fixture","text":"public fixture","messageId":"fixture","extra":{"service":5}})).unwrap();
    chat.ingest_received(
        "soop",
        &line,
        None,
        vec![],
        vec![],
        None,
        None,
        None,
        t + Duration::from_millis(15),
    )
    .unwrap();
    chat.detach_archive()
        .unwrap()
        .unwrap()
        .persist(root.path(), "soop", &id)
        .unwrap();
    let rows = controls(root.path());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["values"]["fanMessageColor"], false);
    assert_eq!(rows[1]["soopDisplay"]["fanMessageColor"], true);
    assert_eq!(rows[1]["text"], "public fixture");
    chat.stop();
}
#[test]
fn new_recording_gets_current_snapshot_without_prior_pending_history() {
    let chat = SharedChat::new();
    let receiver = uuid::Uuid::new_v4().to_string();
    chat.reset_display(&receiver, 1);
    chat.observe_display(&receiver, 1, observation(1, true));
    chat.observe_display(&receiver, 1, observation(2, false));
    let root = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(root.path(), "soop", &id)
        .unwrap();
    chat.detach_archive()
        .unwrap()
        .unwrap()
        .persist(root.path(), "soop", &id)
        .unwrap();
    let rows = controls(root.path());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["sequence"], 2);
    assert_eq!(rows[0]["values"]["showEmoticons"], false);
    chat.stop();
}

fn palette_observation(seq: u64, color: &str) -> Observation {
    let mut o = observation(seq, true);
    o.palette=Some(serde_json::from_value(serde_json::json!({"version":1,"basis":"computed-css-variable",
        "nickname":vec![Some(color);20],"roles":{"normal":color,"streamer":null,"manager":null,"vip":null,
        "subscribe":null,"fan":null,"support":null,"nightbot":null}})).unwrap());
    o.surface=Some(serde_json::from_value(serde_json::json!({"version":1,"basis":"computed-dom","foreground":color,"background":"#0c0d0e"})).unwrap());
    o
}
#[test]
fn palette_and_surface_history_fan_out_and_independent_detach_preserves_snapshots() {
    let chat = SharedChat::new();
    let receiver = uuid::Uuid::new_v4().to_string();
    chat.reset_display(&receiver, 1);
    chat.observe_display(&receiver, 1, palette_observation(1, "#112233"));
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("archive");
    fs::create_dir(&archive).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(&archive, "soop", &id)
        .unwrap();
    let binding = Binding {
        receiver_id: receiver.clone(),
        run_id: uuid::Uuid::new_v4().to_string(),
        source_id: uuid::Uuid::new_v4().to_string(),
        document_epoch: 1,
        channel_id: "fixture".into(),
        broadcast_id: "123".into(),
    };
    let mut relay = RelayChatLease::attach(chat.clone(), None, binding, root.path()).unwrap();
    chat.observe_display(&receiver, 1, palette_observation(2, "#445566"));
    chat.detach_archive()
        .unwrap()
        .unwrap()
        .persist(&archive, "soop", &id)
        .unwrap();
    chat.observe_display(&receiver, 1, palette_observation(3, "#778899"));
    let summary = finish(&mut relay);
    let original = controls(&archive);
    let relayed = controls(&summary.directory);
    assert_eq!(original.len(), 2);
    assert_eq!(relayed.len(), 3);
    for i in 0..2 {
        assert_eq!(original[i]["palette"], relayed[i]["palette"]);
        assert_eq!(original[i]["surface"], relayed[i]["surface"]);
    }
    assert_eq!(original[0]["palette"]["roles"]["normal"], "#112233");
    assert_eq!(original[1]["palette"]["roles"]["normal"], "#445566");
    assert_eq!(relayed[2]["palette"]["roles"]["normal"], "#778899");
    assert_eq!(original[0]["surface"]["foreground"], "#112233");
    assert_eq!(original[1]["surface"]["foreground"], "#445566");
    assert_eq!(relayed[2]["surface"]["foreground"], "#778899");
    assert_eq!(summary.event_rows, 0);
    assert_eq!(summary.raw_rows, 0);
    assert!(summary.joined);
    assert!(summary.failure.is_none());
    assert!(!chat.stop.load(Ordering::Acquire));
    chat.stop();
}
