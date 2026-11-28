//! Receive-only public gift fields from official LivePlayer 18/33/105.
//! Evidence: 20261117-cloud-goal-resume/soop-donation-protocol-evidence.json.
//! Account identifiers, private packets and packet-supplied URLs never leave this parser.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[path = "chat_gift_asset.rs"]
pub(crate) mod assets;
pub(crate) const COVERAGE_WARNING: &str = "일부 SOOP 채팅 기록을 저장하지 못했습니다.";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SoopGiftKind {
    Balloon,
    VideoBalloon,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SoopGiftMetadata {
    pub kind: SoopGiftKind,
    pub count: u32,
    pub relay: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_order: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_fan: Option<bool>,
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "asset_id")]
    pub image_asset_id: Option<String>,
}
fn asset_id<'de, D: serde::Deserializer<'de>>(deserializer:D)->Result<Option<String>,D::Error> {
    let value=Value::deserialize(deserializer)?;
    Ok(value.as_str().filter(|value|assets::valid_id(value)).map(str::to_owned))
}

pub(super) struct PublicGift<'a> {
    pub nickname: &'a str,
    pub file_name: &'a str,
    pub metadata: SoopGiftMetadata,
}

fn integer(value: &str) -> Result<u32, ()> {
    if value.is_empty() || value.len() > 10 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    value.parse().map_err(|_| ())
}
fn optional_integer(value: &str) -> Result<Option<u32>, ()> {
    if value.is_empty() { Ok(None) } else { integer(value).map(Some) }
}
fn boolean(value: &str) -> Result<bool, ()> {
    match value { "" | "0" => Ok(false), "1" => Ok(true), _ => Err(()) }
}

pub(super) fn parse<'a>(service: u16, fields: &[&'a str]) -> Result<Option<PublicGift<'a>>, ()> {
    let (nickname, count, fan, file, default, top, minimum) = match service {
        18 => (2, 3, 4, 7, 8, 9, 10),
        33 => (4, 5, 6, 8, 9, 10, 11),
        105 => (3, 4, 5, 12, 13, 7, 14),
        _ => return Ok(None),
    };
    if !(minimum..=32).contains(&fields.len()) || fields[nickname].len() > 512 {
        return Err(());
    }
    let count = integer(fields[count])?;
    if count == 0 { return Err(()); }
    let original_file_name = fields[file];
    let file_name = if original_file_name.len() > 256 || (!original_file_name.is_empty()
        && (!original_file_name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            || original_file_name.contains("..") || original_file_name.starts_with('.'))) { "" } else { original_file_name };
    let metadata = SoopGiftMetadata {
        kind: if service == 105 { SoopGiftKind::VideoBalloon } else { SoopGiftKind::Balloon },
        count,
        relay: if service == 105 { boolean(fields[8])? } else { service == 33 },
        fan_order: optional_integer(fields[fan])?,
        top_fan: optional_integer(fields[top])?.map(|value| value > 0),
        // Official LIVE constructor: isDefault || !fileName.
        is_default: boolean(fields[default])? || original_file_name.is_empty(),
        image_asset_id: None,
    };
    Ok(Some(PublicGift { nickname: fields[nickname], file_name, metadata }))
}

pub(crate) fn from_stored(value: &Value) -> Option<SoopGiftMetadata> {
    let gift: SoopGiftMetadata = serde_json::from_value(value.clone()).ok()?;
    (gift.count > 0).then_some(gift)
}
pub(crate) fn from_raw(value: &Value) -> Option<SoopGiftMetadata> {
    if value["kind"] != "donation" { return None; }
    let service = value["extra"]["service"].as_u64()?;
    let gift = from_stored(&value["extra"]["soopGift"])?;
    let consistent = match service {
        18 => gift.kind == SoopGiftKind::Balloon && !gift.relay,
        33 => gift.kind == SoopGiftKind::Balloon && gift.relay,
        105 => gift.kind == SoopGiftKind::VideoBalloon,
        _ => false,
    };
    consistent.then_some(gift)
}

pub(crate) fn known_unsupported(service: u16) -> bool {
    // Each service has an observed official public chat renderer. Account-only
    // receipt 144, private packets and unverified legacy37/38 stay excluded.
    // Evidence: soop-gift-coverage-amendment.json, checked2026-11-17.
    matches!(service, 20 | 34 | 45 | 70 | 71 | 86 | 87 | 92 | 102 | 103
        | 107 | 108 | 118 | 120 | 125 | 142 | 143)
}
pub(crate) fn is_public_rich(service: u16) -> bool {
    matches!(service, 18 | 33 | 91 | 93 | 105 | 109 | 121) || known_unsupported(service)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fields(service: u16) -> Vec<&'static str> {
        match service {
            18 => vec!["receiver-secret", "sender-secret", "Public viewer", "100", "7", "ignored", "ignored", "", "0", "1"],
            33 => vec!["room-secret", "receiver-secret", "ignored", "sender-secret", "Public viewer", "100", "7", "ignored", "custom_art", "0", "1"],
            105 => vec!["room-secret", "receiver-secret", "sender-secret", "Public viewer", "100", "7", "ignored", "1", "1", "ignored", "ignored", "ignored", "video", "1"],
            _ => unreachable!(),
        }
    }
    #[test]
    fn exact_public_service_fields_and_official_default_rule() {
        for service in [18, 33, 105] {
            let input = fields(service);
            let result = parse(service, &input).unwrap().unwrap();
            assert_eq!(result.nickname, "Public viewer");
            assert_eq!(result.metadata.count, 100);
            assert_eq!(result.metadata.fan_order, Some(7));
            assert_eq!(result.metadata.top_fan, Some(true));
            assert_eq!(result.metadata.relay, service != 18);
            assert_eq!(result.metadata.is_default, service != 33);
            assert_eq!(result.metadata.kind, if service == 105 { SoopGiftKind::VideoBalloon } else { SoopGiftKind::Balloon });
            assert!(!serde_json::to_string(&result.metadata).unwrap().contains("secret"));
        }
    }
    #[test]
    fn malformed_fields_do_not_become_trusted_gifts() {
        for service in [18, 33, 105] {
            let valid = fields(service);
            for length in 0..valid.len() { assert!(parse(service, &valid[..length]).is_err()); }
            let count_index = match service { 18 => 3, 33 => 5, _ => 4 };
            for invalid in ["", "-1", "0", "1.1", "1e2", "4294967296", "<b>1</b>"] {
                let mut row = valid.clone(); row[count_index] = invalid;
                assert!(parse(service, &row).is_err());
            }
            let file_index = match service { 18 => 7, 33 => 8, _ => 12 };
            for invalid in ["../outside", "..", "https://evil.test/a", "a\\b", "a?x=1"] {
                let mut row = valid.clone(); row[file_index] = invalid;
                let gift = parse(service, &row).unwrap().unwrap();
                assert_eq!(gift.metadata.count, 100);
                assert!(!gift.metadata.is_default || service == 105);
                assert!(gift.file_name.is_empty());
            }
        }
        assert!(parse(9, &["private text", "secret account"]).unwrap().is_none());
        for service in [4, 9, 16, 26, 37, 38, 130, 144] {
            assert!(!known_unsupported(service));
            assert!(!is_public_rich(service));
        }
        assert!(!known_unsupported(109) && is_public_rich(109));
        for service in [91,93,121] {assert!(!known_unsupported(service) && is_public_rich(service));}
        for service in [20, 34, 45, 70, 71, 86, 87, 92, 102, 103, 107, 108, 118, 120, 125, 142, 143] {
            assert!(known_unsupported(service));
            assert!(parse(service, &["never-store-public-rich-wire"]).unwrap().is_none());
        }
    }
    #[test]
    fn stored_schema_rejects_invalid_counts_enums_and_flags() {
        let base = serde_json::to_value(parse(18, &fields(18)).unwrap().unwrap().metadata).unwrap();
        assert!(from_stored(&base).is_some());
        for (key, value) in [("count", serde_json::json!(0)), ("count", serde_json::json!(-1)),
            ("count", serde_json::json!(4294967296u64)), ("kind", serde_json::json!("mission")),
            ("relay", serde_json::json!("true")), ("topFan", serde_json::json!("1"))] {
            let mut row = base.clone(); row[key] = value; assert!(from_stored(&row).is_none());
        }
    }
}
