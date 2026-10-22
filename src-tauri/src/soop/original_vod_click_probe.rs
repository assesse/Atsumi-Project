//! Opt-in, isolated debug QA observation. No input, media or account authority.
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
};

const FLAG: &str = "ATSUMI_SOOP_VOD_CLICK_PROBE";
const PREFIX: &str = "ATSUMI_SOOP_VOD_CLICK_PROBE:";
const OUTPUT: &str = "SOOP_VOD_CLICK_PROBE=";
const ANCHOR: &str = "  document.addEventListener('pointerdown', event => {";
const SCRIPT: &str = include_str!("original_vod_page.js");
const OBSERVER: &str = include_str!("original_vod_click_probe.js");
const LIMIT: usize = 16;

fn eligible(debug: bool, flag: bool, identifier: &str, video: &str) -> bool {
    debug && flag && video == "209244227" && identifier
        .strip_prefix("local.atsumi.platformqa.")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}
pub(super) fn enabled(identifier: &str, video: &str) -> bool {
    eligible(cfg!(debug_assertions), std::env::var(FLAG).as_deref() == Ok("1"), identifier, video)
}
fn insert_observer(script: &str, enabled: bool) -> Cow<'_, str> {
    if !enabled || script.matches(ANCHOR).count() != 1 { return Cow::Borrowed(script); }
    Cow::Owned(script.replacen(ANCHOR, &format!("{OBSERVER}\n{ANCHOR}"), 1))
}
pub(super) fn initialization(identifier: &str, video: &str) -> Cow<'static, str> {
    insert_observer(SCRIPT, enabled(identifier, video))
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum EventKind { Pointerdown, Click }
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Target { FullscreenButton, PlayerOther, Chat, Excluded }
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DomRow {
    kind: EventKind,
    target: Target,
    trusted: bool,
    left: bool,
    configured: bool,
    fullscreen: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeRow {
    kind: &'static str,
    source_accepted: bool,
    epoch_accepted: bool,
    accepted: bool,
}
pub(super) struct Budget { enabled: bool, used: AtomicUsize }
impl Budget {
    pub(super) fn new(enabled: bool) -> Self { Self { enabled, used: AtomicUsize::new(0) } }
    pub(super) fn enabled(&self) -> bool { self.enabled }
    fn encode<T: Serialize>(&self, row: &T) -> Option<String> {
        if !self.enabled { return None; }
        // Serialization is only from the closed native types, never a page Value.
        let output = serde_json::to_string(row).ok()?;
        self.used.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |used| (used < LIMIT).then_some(used + 1)).ok()?;
        Some(output)
    }
    fn dom_row(&self, raw: &str, source_accepted: bool) -> Option<String> {
        if !source_accepted || raw.len() > 512 { return None; }
        self.encode(&serde_json::from_str::<DomRow>(raw).ok()?)
    }
    fn emit(value: Option<String>) {
        if let Some(value) = value {
            // Ignore broken stdout; a diagnostic must never fail the native callback.
            let _ = writeln!(std::io::stdout().lock(), "{OUTPUT}{value}");
        }
    }
    pub(super) fn observe_dom(&self, body: &str, source_accepted: bool) -> bool {
        let Some(raw) = body.strip_prefix(PREFIX) else { return false; };
        Self::emit(self.dom_row(raw, source_accepted));
        true
    }
    pub(super) fn observe_bridge(&self, source_accepted: bool, epoch_accepted: bool, accepted: bool) {
        Self::emit(self.encode(&BridgeRow { kind: "bridge", source_accepted, epoch_accepted, accepted }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "local.atsumi.platformqa.0123456789abcdef0123456789abcdef";
    const DOM: &str = r#"{"kind":"pointerdown","target":"fullscreen_button","trusted":true,"left":true,"configured":true,"fullscreen":false}"#;
    #[test]
    fn activation_requires_debug_optin_exact_isolated_identity_and_public_title() {
        assert!(eligible(true, true, ID, "209244227"));
        for (debug, flag, id, video) in [(false,true,ID,"209244227"),(true,false,ID,"209244227"),
            (true,true,"local.atsumi","209244227"),(true,true,"local.atsumi.platformqa.0123456789ABCDEF0123456789ABCDEF","209244227"),
            (true,true,"local.atsumi.platformqa.0123456789abcdef0123456789abcdef.extra","209244227"),(true,true,ID,"209244228")] {
            assert!(!eligible(debug,flag,id,video));
        }
    }
    #[test]
    fn disabled_or_changed_anchor_preserves_exact_original_bytes() {
        assert!(matches!(insert_observer(SCRIPT,false),Cow::Borrowed(_)));
        assert_eq!(insert_observer(SCRIPT,false),SCRIPT);
        assert_eq!(insert_observer("no anchor",true),"no anchor");
        let duplicate=format!("{ANCHOR}\n{ANCHOR}");
        assert_eq!(insert_observer(&duplicate,true),duplicate);
        let patched=insert_observer(SCRIPT,true);
        assert_eq!(patched.matches(OBSERVER).count(),1);
        assert_eq!(patched.replacen(&format!("{OBSERVER}\n"),"",1),SCRIPT);
    }
    #[test]
    fn strict_projection_rejects_extra_raw_fields_types_unknown_enums_and_oversize() {
        let budget=Budget::new(true);
        assert_eq!(budget.dom_row(DOM,true).as_deref(),Some(DOM));
        for raw in [DOM.replace("\"pointerdown\"","\"keydown\""),DOM.replace("\"fullscreen_button\"","\"raw-class\""),
            DOM.replace("true","1"),DOM.replace("false}","false,\"url\":\"secret\"}")," ".repeat(513),"{}".into(),
            DOM.replace("false}","false,\"left\":true}")] {
            assert!(budget.dom_row(&raw,true).is_none());
        }
        assert!(budget.dom_row(DOM,false).is_none());
        assert_eq!(budget.used.load(Ordering::Acquire),1);
        assert!(Budget::new(false).dom_row(DOM,true).is_none());
    }
    #[test]
    fn dom_and_bridge_share_saturating_sixteen_row_budget() {
        let budget=Budget::new(true);
        for index in 0..LIMIT {
            let value=if index%2==0 { budget.dom_row(DOM,true) } else {
                budget.encode(&BridgeRow {kind:"bridge",source_accepted:true,epoch_accepted:false,accepted:false})
            };
            assert!(value.is_some());
        }
        assert!(budget.dom_row(DOM,true).is_none());
        assert_eq!(budget.used.load(Ordering::Acquire),LIMIT);
    }
    #[test]
    fn concurrent_observation_cannot_exceed_native_budget() {
        let budget=std::sync::Arc::new(Budget::new(true));
        let handles:Vec<_>=(0..64).map(|_| {let budget=budget.clone();std::thread::spawn(move||budget.dom_row(DOM,true).is_some())}).collect();
        assert_eq!(handles.into_iter().map(|handle|handle.join().unwrap()).filter(|accepted|*accepted).count(),LIMIT);
    }
}
