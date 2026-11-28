//! Allowlisted observed viewer presentation. No account/storage/network reads.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::VecDeque,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(super) const FORMAT: &str = "atsumi-soop-display-v1";
const KEYS: [&str; 7] = [
    "theme",
    "fontLevel",
    "randomNicknameColor",
    "fanMessageColor",
    "showEmoticons",
    "animateEmoticons",
    "smallOgq",
];
const HISTORY: usize = 64;
pub(super) const MAX_CONTROL_BYTES: usize = 2048;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PaletteRoles {
    pub normal: Option<String>,
    pub streamer: Option<String>,
    pub manager: Option<String>,
    pub vip: Option<String>,
    pub subscribe: Option<String>,
    pub fan: Option<String>,
    pub support: Option<String>,
    pub nightbot: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub version: u8,
    pub basis: String,
    pub nickname: [Option<String>; 20],
    pub roles: PaletteRoles,
}
impl Palette {
    fn parse(value: Value) -> Option<Self> {
        if !exact(&value, &["version", "basis", "nickname", "roles"])
            || !exact(
                &value["roles"],
                &[
                    "normal",
                    "streamer",
                    "manager",
                    "vip",
                    "subscribe",
                    "fan",
                    "support",
                    "nightbot",
                ],
            )
        {
            return None;
        }
        let p: Self = serde_json::from_value(value).ok()?;
        let roles = [
            &p.roles.normal,
            &p.roles.streamer,
            &p.roles.manager,
            &p.roles.vip,
            &p.roles.subscribe,
            &p.roles.fan,
            &p.roles.support,
            &p.roles.nightbot,
        ];
        let canonical = p.nickname.iter().chain(roles).all(|color| {
            color.as_ref().is_none_or(|color| {
                let b = color.as_bytes();
                b.len() == 7
                    && b[0] == b'#'
                    && b[1..]
                        .iter()
                        .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(v))
            })
        });
        (p.version == 1 && p.basis == "computed-css-variable" && canonical).then_some(p)
    }
}
// Malformed optional appearance must not discard healthy settings or chat.
fn take_palette(value: &mut Value) -> (Option<Palette>, bool) {
    match value.as_object_mut().and_then(|v| v.remove("palette")) {
        None | Some(Value::Null) => (None, false),
        Some(value) => match Palette::parse(value) {
            Some(palette) => (Some(palette), false),
            None => (None, true),
        },
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub version: u8,
    pub basis: String,
    pub foreground: Option<String>,
    pub background: Option<String>,
}
impl Surface {
    fn parse(value: Value) -> Option<Self> {
        if !exact(&value, &["version", "basis", "foreground", "background"]) {
            return None;
        }
        let surface: Self = serde_json::from_value(value).ok()?;
        let canonical = [&surface.foreground, &surface.background]
            .into_iter()
            .all(|color| {
                color.as_ref().is_none_or(|color| {
                    let bytes = color.as_bytes();
                    bytes.len() == 7
                        && bytes[0] == b'#'
                        && bytes[1..]
                            .iter()
                            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(v))
                })
            });
        (surface.version == 1 && surface.basis == "computed-dom" && canonical).then_some(surface)
    }
}
fn take_surface(value: &mut Value) -> (Option<Surface>, bool) {
    match value.as_object_mut().and_then(|v| v.remove("surface")) {
        None | Some(Value::Null) => (None, false),
        Some(value) => match Surface::parse(value) {
            Some(surface) => (Some(surface), false),
            None => (None, true),
        },
    }
}
fn is_false(value: &bool) -> bool {
    !*value
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Basis {
    Dom,
    SavedOption,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Values {
    pub theme: Option<Theme>,
    pub font_level: Option<u8>,
    pub random_nickname_color: Option<bool>,
    pub fan_message_color: Option<bool>,
    pub show_emoticons: Option<bool>,
    pub animate_emoticons: Option<bool>,
    pub small_ogq: Option<bool>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evidence {
    pub theme: Option<Basis>,
    pub font_level: Option<Basis>,
    pub random_nickname_color: Option<Basis>,
    pub fan_message_color: Option<Basis>,
    pub show_emoticons: Option<Basis>,
    pub animate_emoticons: Option<Basis>,
    pub small_ogq: Option<Basis>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Creation {
    pub fan_message_color: Option<bool>,
    pub show_emoticons: Option<bool>,
    pub small_ogq: Option<bool>,
}
impl Creation {
    pub(super) fn from_stored(value: &Value) -> Option<Self> {
        let keys = ["fanMessageColor", "showEmoticons", "smallOgq"];
        if !exact(value, &keys) {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }
}
fn exact(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|m| m.len() == keys.len() && keys.iter().all(|k| m.contains_key(*k)))
}
fn valid(values: &Values, basis: &Evidence) -> bool {
    if values.font_level.is_some_and(|v| !(1..=8).contains(&v)) {
        return false;
    }
    let presence = [
        values.theme.is_some(),
        values.font_level.is_some(),
        values.random_nickname_color.is_some(),
        values.fan_message_color.is_some(),
        values.show_emoticons.is_some(),
        values.animate_emoticons.is_some(),
        values.small_ogq.is_some(),
    ];
    let proof = [
        basis.theme,
        basis.font_level,
        basis.random_nickname_color,
        basis.fan_message_color,
        basis.show_emoticons,
        basis.animate_emoticons,
        basis.small_ogq,
    ];
    presence
        .into_iter()
        .zip(proof)
        .enumerate()
        .all(|(i, (present, proof))| {
            present == proof.is_some() && (i >= 3 || proof != Some(Basis::SavedOption))
        })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Wire {
    sequence: u64,
    values: Values,
    basis: Evidence,
    actual_channel: String,
    actual_broadcast: String,
}
pub(super) struct Observation {
    pub sequence: u64,
    pub values: Values,
    pub basis: Evidence,
    pub palette: Option<Palette>,
    pub palette_incomplete: bool,
    pub surface: Option<Surface>,
    pub surface_incomplete: bool,
}
pub(super) fn parse(mut value: Value, channel: &str, broadcast: &str) -> Option<Observation> {
    if !exact(&value["values"], &KEYS) || !exact(&value["basis"], &KEYS) {
        return None;
    }
    let (palette, palette_incomplete) = take_palette(&mut value);
    let (surface, surface_incomplete) = take_surface(&mut value);
    let wire: Wire = serde_json::from_value(value).ok()?;
    if wire.actual_channel != channel
        || wire.actual_broadcast != broadcast
        || wire.sequence == 0
        || wire.sequence > 9_007_199_254_740_991
        || !valid(&wire.values, &wire.basis)
    {
        return None;
    }
    Some(Observation {
        sequence: wire.sequence,
        values: wire.values,
        basis: wire.basis,
        palette,
        palette_incomplete,
        surface,
        surface_incomplete,
    })
}
pub(super) fn wall() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Control {
    pub format: String,
    pub receiver_id: String,
    pub document_epoch: u64,
    pub sequence: u64,
    pub received_at_ms: u64,
    pub observed_at_ms: Option<u64>,
    pub initial: bool,
    pub values: Values,
    pub basis: Evidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Palette>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub palette_incomplete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<Surface>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub surface_incomplete: bool,
}
impl Control {
    pub(super) fn parse(value: &Value) -> Option<Self> {
        if !exact(&value["values"], &KEYS) || !exact(&value["basis"], &KEYS) {
            return None;
        }
        let mut value = value.clone();
        let (palette, palette_incomplete) = take_palette(&mut value);
        let (surface, surface_incomplete) = take_surface(&mut value);
        let mut c: Self = serde_json::from_value(value).ok()?;
        c.palette = palette;
        c.palette_incomplete |= palette_incomplete;
        if c.palette_incomplete {
            c.palette = None;
        }
        c.surface = surface;
        c.surface_incomplete |= surface_incomplete;
        if c.surface_incomplete {
            c.surface = None;
        }
        if c.format != FORMAT
            || uuid::Uuid::parse_str(&c.receiver_id).ok()?.is_nil()
            || c.document_epoch == 0
            || c.sequence > 9_007_199_254_740_991
            || !(1_000_000_000_000..=9_000_000_000_000).contains(&c.received_at_ms)
            || c.observed_at_ms.is_some_and(|v| v > c.received_at_ms)
            || !valid(&c.values, &c.basis)
            || serde_json::to_vec(&c).ok()?.len() > MAX_CONTROL_BYTES
        {
            return None;
        }
        Some(c)
    }
    pub(super) fn clear(&mut self) {
        self.values = Values::default();
        self.basis = Evidence::default();
        self.observed_at_ms = None;
        self.palette = None;
        self.palette_incomplete = false;
        self.surface = None;
        self.surface_incomplete = false;
    }
    pub(super) fn page(self, time_ms: Option<u64>) -> Page {
        Page {
            version: 1,
            values: self.values,
            basis: self.basis,
            palette: self.palette,
            palette_incomplete: self.palette_incomplete,
            surface: self.surface,
            surface_incomplete: self.surface_incomplete,
            document_epoch: self.document_epoch,
            sequence: self.sequence,
            observed_at_ms: self.observed_at_ms,
            time_ms,
            timing_quality: if time_ms.is_some() {
                "receive-time-approximate"
            } else {
                "unmapped"
            },
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub version: u8,
    pub values: Values,
    pub basis: Evidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palette: Option<Palette>,
    #[serde(skip_serializing_if = "is_false")]
    pub palette_incomplete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<Surface>,
    #[serde(skip_serializing_if = "is_false")]
    pub surface_incomplete: bool,
    pub document_epoch: u64,
    pub sequence: u64,
    pub observed_at_ms: Option<u64>,
    pub time_ms: Option<u64>,
    pub timing_quality: &'static str,
}
#[derive(Default)]
pub(super) struct Capture {
    epoch: u64,
    sequence: u64,
    history: VecDeque<(Instant, Control)>,
    pending: VecDeque<(Instant, Control)>,
}
impl Capture {
    pub(super) fn reset(&mut self, receiver: &str, epoch: u64, now: Instant, wall: u64) {
        if epoch < self.epoch || epoch == 0 {
            return;
        }
        if epoch == self.epoch
            && self.history.back().is_some_and(|(_, c)| {
                c.values == Values::default()
                    && c.basis == Evidence::default()
                    && c.palette.is_none()
                    && !c.palette_incomplete
                    && c.surface.is_none()
                    && !c.surface_incomplete
            })
        {
            return;
        }
        if epoch != self.epoch {
            self.sequence = 0;
        }
        self.epoch = epoch;
        self.push(
            now,
            Control {
                format: FORMAT.into(),
                receiver_id: receiver.into(),
                document_epoch: epoch,
                sequence: self.sequence,
                received_at_ms: wall,
                observed_at_ms: None,
                initial: false,
                values: Default::default(),
                basis: Default::default(),
                palette: None,
                palette_incomplete: false,
                surface: None,
                surface_incomplete: false,
            },
        );
    }
    pub(super) fn observe(
        &mut self,
        receiver: &str,
        epoch: u64,
        observation: Observation,
        now: Instant,
        wall: u64,
    ) -> bool {
        if epoch != self.epoch || observation.sequence <= self.sequence {
            return false;
        }
        self.sequence = observation.sequence;
        if self.history.back().is_some_and(|(_, c)| {
            c.document_epoch == epoch
                && c.values == observation.values
                && c.basis == observation.basis
                && c.palette == observation.palette
                && c.palette_incomplete == observation.palette_incomplete
                && c.surface == observation.surface
                && c.surface_incomplete == observation.surface_incomplete
        }) {
            return true;
        }
        self.push(
            now,
            Control {
                format: FORMAT.into(),
                receiver_id: receiver.into(),
                document_epoch: epoch,
                sequence: observation.sequence,
                received_at_ms: wall,
                observed_at_ms: Some(wall),
                initial: false,
                values: observation.values,
                basis: observation.basis,
                palette: observation.palette,
                palette_incomplete: observation.palette_incomplete,
                surface: observation.surface,
                surface_incomplete: observation.surface_incomplete,
            },
        );
        true
    }
    fn push(&mut self, at: Instant, mut c: Control) {
        // At most63 exact pending transitions plus an explicit Unknown after
        // overflow. Never silently use a stale last-known value on overflow.
        if self.pending.len() >= HISTORY {
            c.clear();
            if let Some((_, last)) = self.pending.back_mut() {
                last.clear();
            }
        } else {
            self.pending.push_back((at, c.clone()));
        }
        self.history.push_back((at, c));
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
    }
    pub(super) fn creation(&self, at: Instant) -> Creation {
        self.history
            .iter()
            .rev()
            .find(|(received, _)| *received <= at)
            .map(|(_, c)| Creation {
                fan_message_color: c.values.fan_message_color,
                show_emoticons: c.values.show_emoticons,
                small_ogq: c.values.small_ogq,
            })
            .unwrap_or_default()
    }
    pub(super) fn take_pending(&mut self) -> Vec<(Instant, Control)> {
        self.pending.drain(..).collect()
    }
    pub(super) fn initial(&self) -> Option<Control> {
        self.history.back().map(|(_, c)| {
            let mut c = c.clone();
            c.initial = true;
            c
        })
    }
}

#[cfg(test)]
#[path = "soop_display_palette_tests.rs"]
mod palette_tests;

#[cfg(test)]
#[path = "soop_display_surface_tests.rs"]
mod surface_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    const ID: &str = "90000000-0000-4000-8000-000000000001";
    const WALL: u64 = 1_700_000_000_000;
    fn observed(sequence: u64, enabled: bool) -> Observation {
        Observation {
            sequence,
            values: Values {
                fan_message_color: Some(enabled),
                show_emoticons: Some(enabled),
                ..Default::default()
            },
            basis: Evidence {
                fan_message_color: Some(Basis::SavedOption),
                show_emoticons: Some(Basis::Dom),
                ..Default::default()
            },
            palette: None,
            palette_incomplete: false,
            surface: None,
            surface_incomplete: false,
        }
    }
    fn wire() -> Value {
        let o = observed(1, true);
        serde_json::json!({"sequence":1,"actualChannel":"fixture","actualBroadcast":"123","values":o.values,"basis":o.basis})
    }
    #[test]
    fn only_exact_observed_fields_and_bound_source_are_admitted() {
        assert!(parse(wire(), "fixture", "123").is_some());
        assert!(parse(wire(), "other", "123").is_none());
        assert!(parse(wire(), "fixture", "124").is_none());
        for pointer in ["/values/showEmoticons", "/basis/showEmoticons"] {
            let mut v = wire();
            *v.pointer_mut(pointer).unwrap() = Value::Null;
            assert!(parse(v, "fixture", "123").is_none());
        }
        for invalid in [
            serde_json::json!(0),
            serde_json::json!(9),
            serde_json::json!(1.5),
        ] {
            let mut v = wire();
            v["values"]["fontLevel"] = invalid;
            v["basis"]["fontLevel"] = serde_json::json!("dom");
            assert!(parse(v, "fixture", "123").is_none());
        }
        let mut v = wire();
        v["values"]["theme"] = serde_json::json!("dark");
        v["basis"]["theme"] = serde_json::json!("saved-option");
        assert!(parse(v, "fixture", "123").is_none());
        let mut v = wire();
        v["storageDump"] = serde_json::json!({});
        assert!(parse(v, "fixture", "123").is_none());
        let mut v = wire();
        v["values"].as_object_mut().unwrap().remove("smallOgq");
        assert!(parse(v, "fixture", "123").is_none());
    }
    #[test]
    fn delayed_chat_uses_receipt_history_not_ingestion_order() {
        let t = Instant::now();
        let mut c = Capture::default();
        c.reset(ID, 1, t, WALL);
        c.observe(
            ID,
            1,
            observed(1, true),
            t + Duration::from_millis(10),
            WALL + 10,
        );
        let delayed_packet = t + Duration::from_millis(15);
        c.observe(
            ID,
            1,
            observed(2, false),
            t + Duration::from_millis(20),
            WALL + 20,
        );
        assert_eq!(c.creation(delayed_packet).fan_message_color, Some(true));
        assert_eq!(
            c.creation(t + Duration::from_millis(21)).fan_message_color,
            Some(false)
        );
        assert_eq!(c.creation(t).fan_message_color, None);
    }
    #[test]
    fn source_reset_and_epoch_fence_cannot_restore_stale_observation() {
        let t = Instant::now();
        let mut c = Capture::default();
        c.reset(ID, 3, t, WALL);
        assert!(c.observe(
            ID,
            3,
            observed(7, true),
            t + Duration::from_millis(1),
            WALL + 1
        ));
        c.reset(ID, 3, t + Duration::from_millis(2), WALL + 2);
        assert!(!c.observe(
            ID,
            3,
            observed(6, true),
            t + Duration::from_millis(3),
            WALL + 3
        ));
        assert_eq!(
            c.creation(t + Duration::from_millis(3)),
            Creation::default()
        );
        assert!(c.observe(
            ID,
            3,
            observed(8, false),
            t + Duration::from_millis(4),
            WALL + 4
        ));
        c.reset(ID, 2, t + Duration::from_millis(5), WALL + 5);
        assert_eq!(
            c.creation(t + Duration::from_millis(5)).fan_message_color,
            Some(false)
        );
        c.reset(ID, 4, t + Duration::from_millis(6), WALL + 6);
        assert!(!c.observe(
            ID,
            3,
            observed(9, true),
            t + Duration::from_millis(7),
            WALL + 7
        ));
        assert!(c.observe(
            ID,
            4,
            observed(1, true),
            t + Duration::from_millis(8),
            WALL + 8
        ));
    }
    #[test]
    fn expired_history_and_overflow_are_unknown_and_bounded() {
        let t = Instant::now();
        let mut c = Capture::default();
        c.reset(ID, 1, t, WALL);
        for n in 1..=130 {
            c.observe(
                ID,
                1,
                observed(n, n % 2 == 0),
                t + Duration::from_millis(n),
                WALL + n,
            );
        }
        assert_eq!(c.history.len(), HISTORY);
        assert_eq!(c.pending.len(), HISTORY);
        assert_eq!(
            c.creation(t + Duration::from_millis(2)),
            Creation::default()
        );
        assert_eq!(
            c.creation(t + Duration::from_millis(131)),
            Creation::default()
        );
        let records = c.take_pending();
        assert_eq!(records.last().unwrap().1.values, Values::default());
        assert!(c.observe(
            ID,
            1,
            observed(131, true),
            t + Duration::from_millis(132),
            WALL + 132
        ));
        assert_eq!(
            c.creation(t + Duration::from_millis(132)).show_emoticons,
            Some(true)
        );
    }
    #[test]
    fn initial_snapshot_does_not_drain_or_relabel_original_controls() {
        let t = Instant::now();
        let mut c = Capture::default();
        c.reset(ID, 1, t, WALL);
        c.observe(ID, 1, observed(1, true), t, WALL);
        let snapshot = c.initial().unwrap();
        assert!(snapshot.initial);
        assert_eq!(snapshot.observed_at_ms, Some(WALL));
        assert!(c.take_pending().iter().all(|(_, v)| !v.initial));
        assert!(Control::parse(&serde_json::to_value(snapshot).unwrap()).is_some());
    }
}
