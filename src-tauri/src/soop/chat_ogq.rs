//! Public SVC_OGQ_EMOTICON display fields and capture-only asset capabilities.
//! Evidence: pinned LivePlayer109, LiveView5187 and ViewVendor5336 (2026-11-17).
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Ogq,
    Gem,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub kind: Kind,
    pub animated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_image_asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_static_image_asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animated_image_asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_animated_image_asset_id: Option<String>,
}
impl Metadata {
    fn new(kind: Kind, animated: bool) -> Self {
        Self {
            kind,
            animated,
            static_image_asset_id: None,
            small_static_image_asset_id: None,
            animated_image_asset_id: None,
            small_animated_image_asset_id: None,
        }
    }
    pub(crate) fn ids(&self) -> Vec<&str> {
        [
            &self.static_image_asset_id,
            &self.small_static_image_asset_id,
            &self.animated_image_asset_id,
            &self.small_animated_image_asset_id,
        ]
        .into_iter()
        .filter_map(|id| id.as_deref())
        .collect()
    }
    pub(crate) fn expected_count(&self) -> usize {
        if self.animated {
            4
        } else {
            2
        }
    }
}
fn id(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| super::gifts::assets::valid_id(s))
        .map(str::to_owned)
}
pub(crate) fn from_stored(value: &Value) -> Option<Metadata> {
    let kind = match value["kind"].as_str()? {
        "ogq" => Kind::Ogq,
        "gem" => Kind::Gem,
        _ => return None,
    };
    let animated = value["animated"].as_bool()?;
    Some(Metadata {
        kind,
        animated,
        static_image_asset_id: id(&value["staticImageAssetId"]),
        small_static_image_asset_id: id(&value["smallStaticImageAssetId"]),
        animated_image_asset_id: animated
            .then(|| id(&value["animatedImageAssetId"]))
            .flatten(),
        small_animated_image_asset_id: animated
            .then(|| id(&value["smallAnimatedImageAssetId"]))
            .flatten(),
    })
}
pub(crate) fn invalid_stored(value: &Value) -> bool {
    if value.is_null() {
        return false;
    }
    let Some(meta) = from_stored(value) else {
        return true;
    };
    [
        "staticImageAssetId",
        "smallStaticImageAssetId",
        "animatedImageAssetId",
        "smallAnimatedImageAssetId",
    ]
    .iter()
    .any(|key| {
        let animation_field = matches!(*key, "animatedImageAssetId" | "smallAnimatedImageAssetId");
        value.get(*key).is_some_and(|v| {
            !v.is_null() && (id(v).is_none() || (!meta.animated && animation_field))
        })
    })
}
pub(crate) fn nickname_key(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    matches!(
        value,
        "E12E2E"
            | "A90A0A"
            | "EB500D"
            | "C25111"
            | "E07E0A"
            | "B26C03"
            | "648E0B"
            | "119937"
            | "158304"
            | "047143"
            | "0799B9"
            | "0B6D82"
            | "0C80D3"
            | "0C61E1"
            | "1915BF"
            | "6518C8"
            | "AD19E1"
            | "DD32E0"
            | "DC3585"
            | "C40F70"
    )
    .then(|| value.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variant {
    Normal,
    Small,
    Animated,
    SmallAnimated,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    url: String,
    id: String,
    kind: Kind,
    variant: Variant,
}
fn selector(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn numbering(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 10
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u32>().is_ok()
}
pub(crate) fn canonical_url(value: &str) -> Option<&str> {
    let path = value.strip_prefix("https://ogq-sticker-global-cdn-z01.sooplive.com/sticker/")?;
    let (group, file) = path.split_once('/')?;
    if !selector(group) {
        return None;
    }
    let name = file
        .strip_suffix(".png")
        .or_else(|| file.strip_suffix(".webp"))?;
    let (number, size) = name.rsplit_once('_')?;
    (numbering(number) && matches!(size, "80" | "160")).then_some(value)
}
impl Request {
    fn new(group: &str, number: &str, kind: Kind, variant: Variant) -> Self {
        let size = if matches!(variant, Variant::Small | Variant::SmallAnimated) {
            80
        } else {
            160
        };
        let extension = if matches!(variant, Variant::Animated | Variant::SmallAnimated) {
            "webp"
        } else {
            "png"
        };
        let url=format!("https://ogq-sticker-global-cdn-z01.sooplive.com/sticker/{group}/{number}_{size}.{extension}");
        let id = format!("{:x}", Sha256::digest(format!("soop-ogq-v1:{url}")));
        Self {
            url,
            id,
            kind,
            variant,
        }
    }
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn apply(&self, meta: &mut Metadata) -> bool {
        if meta.kind != self.kind {
            return false;
        }
        let target = match self.variant {
            Variant::Normal => &mut meta.static_image_asset_id,
            Variant::Small => &mut meta.small_static_image_asset_id,
            Variant::Animated if meta.animated => &mut meta.animated_image_asset_id,
            Variant::SmallAnimated if meta.animated => &mut meta.small_animated_image_asset_id,
            _ => return false,
        };
        *target = Some(self.id.clone());
        true
    }
}
pub(crate) struct Public<'a> {
    pub text: &'a str,
    pub nickname: &'a str,
    pub flags: &'a str,
    pub metadata: Option<Metadata>,
    pub requests: Vec<Request>,
    pub partial: bool,
}
pub(crate) fn parse<'a>(fields: &[&'a str]) -> Result<Option<Public<'a>>, ()> {
    if !(8..=32).contains(&fields.len())
        || fields[1].len() > 16 * 1024
        || fields[6].len() > 512
        || fields[7].len() > 128
    {
        return Err(());
    }
    let mut result = Public {
        text: fields[1],
        nickname: fields[6],
        flags: fields[7],
        metadata: None,
        requests: Vec::new(),
        partial: false,
    };
    let kind = fields.get(10).copied().unwrap_or("");
    let kind = if kind.trim().is_empty() {
        Some(0)
    } else {
        kind.parse::<i32>().ok()
    };
    if matches!(kind, Some(1 | 2)) {
        return Ok(None);
    }
    // The official default branch accepts other types, but only 0/3 semantics
    // are claimed here. Preserve their public text with explicit partial cover.
    if !matches!(kind, Some(0 | 3)) {
        result.partial = true;
        return Ok(Some(result));
    }
    let group = fields[2];
    let number = fields[3];
    let display_kind = if group.to_ascii_lowercase().contains("gem") {
        Kind::Gem
    } else {
        Kind::Ogq
    };
    let animation = fields.get(17).copied().unwrap_or("");
    let animated = if animation.trim().is_empty() {
        false
    } else if animation.len() <= 32 {
        match animation.parse::<f64>() {
            Ok(v) if v.is_finite() => v == 1.0,
            _ => {
                result.partial = true;
                false
            }
        }
    } else {
        result.partial = true;
        false
    };
    result.metadata = Some(Metadata::new(display_kind, animated));
    if !selector(group) || !numbering(number) {
        result.partial = true;
        return Ok(Some(result));
    }
    for variant in [Variant::Normal, Variant::Small] {
        result
            .requests
            .push(Request::new(group, number, display_kind, variant));
    }
    if animated {
        for variant in [Variant::Animated, Variant::SmallAnimated] {
            result
                .requests
                .push(Request::new(group, number, display_kind, variant));
        }
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fields() -> Vec<&'static str> {
        vec![
            "room-private",
            "public text",
            "gem_fixture",
            "0",
            "version-not-url",
            "sender-private",
            "Public",
            "32|262144",
            "255",
            "0",
            "0",
            "jpg",
            "7",
            "E12E2E",
            "anything",
            "9",
            "7",
            "1",
            "-1",
        ]
    }
    #[test]
    fn exact_variants_keep_gem_identifier_and_ignore_source_ext_version() {
        let f = fields();
        let parsed = parse(&f).unwrap().unwrap();
        assert_eq!(parsed.text, "public text");
        assert_eq!(parsed.requests.len(), 4);
        let mut meta = parsed.metadata.unwrap();
        for request in &parsed.requests {
            assert!(request.apply(&mut meta));
            assert_eq!(canonical_url(request.url()), Some(request.url()));
        }
        assert_eq!(meta.ids().len(), 4);
        assert!(parsed.requests[0].url().ends_with("/gem_fixture/0_160.png"));
        assert!(parsed.requests[3].url().ends_with("/gem_fixture/0_80.webp"));
        let mut changed = f.clone();
        changed[4] = "other-version";
        changed[11] = "https://untrusted";
        assert_eq!(parsed.requests, parse(&changed).unwrap().unwrap().requests);
        let mut missing = f.clone();
        missing.truncate(8);
        assert_eq!(parse(&missing).unwrap().unwrap().requests.len(), 2);
    }
    #[test]
    fn rejected_selectors_keep_public_message_and_empty_emote_rows() {
        for group in ["../x", "a/b", "a%20b", "a?x", "https://evil"] {
            let mut f = fields();
            f[1] = "";
            f[2] = group;
            let p = parse(&f).unwrap().unwrap();
            assert!(
                p.partial && p.requests.is_empty() && p.metadata.is_some() && p.text.is_empty()
            );
        }
        for number in ["-1", "1/2", "4294967296", ""] {
            let mut f = fields();
            f[3] = number;
            assert!(parse(&f).unwrap().unwrap().partial);
        }
        for kind in ["1", "2"] {
            let mut f = fields();
            f[10] = kind;
            assert!(parse(&f).unwrap().is_none());
        }
        let mut f = fields();
        f[10] = "4";
        let p = parse(&f).unwrap().unwrap();
        assert!(p.partial && p.metadata.is_none() && p.text == "public text");
        assert!(canonical_url(
            "https://ogq-sticker-global-cdn-z01.sooplive.com/sticker/a/1_160.png?v=1"
        )
        .is_none());
        assert!(canonical_url("https://evil/sticker/a/1_160.png").is_none());
    }
    #[test]
    fn stored_animation_and_palette_metadata_are_closed() {
        for field in ["animatedImageAssetId", "smallAnimatedImageAssetId"] {
            let mut value = serde_json::json!({"kind":"ogq","animated":false});
            value[field] = Value::String("a".repeat(64));
            assert!(invalid_stored(&value));
            assert!(from_stored(&value).unwrap().ids().is_empty());
            value["animated"] = Value::Bool(true);
            assert!(!invalid_stored(&value));
            assert_eq!(from_stored(&value).unwrap().ids().len(), 1);
        }
        for key in ["E12E2E", "C40F70"] {
            assert!(nickname_key(&Value::String(key.into())).is_some());
        }
        for key in ["e12e2e", "#E12E2E", "000000", "url(https://evil)"] {
            assert!(nickname_key(&Value::String(key.into())).is_none());
        }
    }
}
