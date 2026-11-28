//! Public broadcaster portrait snapshot, bound to the current native channel.
//! No arbitrary viewer profile lookup and no replay-time URL.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
const ROOT: &str = "https://profile.img.sooplive.com/LOGO/";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub image_asset_id: String,
    pub kind: String,
    pub source: String,
}
pub(crate) fn from_stored(v: &Value) -> Option<Metadata> {
    let id = v["imageAssetId"]
        .as_str()
        .filter(|id| super::gifts::assets::valid_id(id))?;
    (v["kind"] == "broadcaster" && v["source"] == "connectionMetadata").then(|| Metadata {
        image_asset_id: id.into(),
        kind: "broadcaster".into(),
        source: "connectionMetadata".into(),
    })
}
fn channel(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub(crate) fn canonical_url(s: &str) -> Option<&str> {
    let tail = s.strip_prefix(ROOT)?;
    let parts: Vec<_> = tail.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let id = parts[1];
    (channel(id) && parts[0] == &id[..id.len().min(2)] && parts[2] == format!("{id}.jpg"))
        .then_some(s)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    url: String,
    metadata: Metadata,
}
impl Request {
    pub(super) fn for_context(
        bound_channel: &str,
        sender: &str,
        nonce: &str,
        flags: &str,
        representative: &str,
    ) -> Option<Self> {
        if !channel(bound_channel) {
            return None;
        }
        let real = if sender.ends_with(')') {
            let (base, suffix) = sender.rsplit_once('(')?;
            if suffix.len() != 2 || !suffix.as_bytes()[0].is_ascii_digit() {
                return None;
            }
            base
        } else {
            sender
        };
        if !real.eq_ignore_ascii_case(bound_channel) {
            return None;
        }
        let (a, b) = flags.split_once('|')?;
        let word = |s: &str| {
            s.parse::<i64>()
                .ok()
                .filter(|n| (i32::MIN as i64..=u32::MAX as i64).contains(n))
                .map(|n| n as u32)
        };
        let (a, b) = (word(a)?, word(b)?);
        if a & 4 == 0 || b & ((1 << 18) | (1 << 19) | (1 << 20)) != 0 {
            return None;
        }
        let rep = if representative.is_empty() {
            -1
        } else {
            representative.parse::<i32>().ok()?
        };
        if !(-1..=0).contains(&rep) {
            return None;
        }
        let url = format!(
            "{ROOT}{}/{bound_channel}/{bound_channel}.jpg",
            &bound_channel[..bound_channel.len().min(2)]
        );
        canonical_url(&url)?;
        let id = format!(
            "{:x}",
            Sha256::digest(format!("soop-broadcaster-v1:{nonce}:{url}"))
        );
        Some(Self {
            url,
            metadata: Metadata {
                image_asset_id: id,
                kind: "broadcaster".into(),
                source: "connectionMetadata".into(),
            },
        })
    }
    pub(crate) fn id(&self) -> &str {
        &self.metadata.image_asset_id
    }
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
    pub(crate) fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub(crate) fn matches(&self, m: &Metadata) -> bool {
        self.metadata == *m
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_bound_broadcaster_fallback_mints_portrait() {
        let r = Request::for_context("fixture", "fixture(1)", "epoch", "4|0", "0").unwrap();
        assert_eq!(
            r.url(),
            "https://profile.img.sooplive.com/LOGO/fi/fixture/fixture.jpg"
        );
        assert_eq!(
            from_stored(&serde_json::to_value(r.metadata()).unwrap()),
            Some(r.metadata().clone())
        );
        for (flags, rep) in [
            ("0|0", "0"),
            ("4|262144", "0"),
            ("4|0", "3"),
            ("4|0", "bad"),
        ] {
            assert!(Request::for_context("fixture", "fixture", "epoch", flags, rep).is_none());
        }
        assert!(Request::for_context("fixture", "other", "epoch", "4|0", "0").is_none());
        assert!(Request::for_context("../other", "../other", "epoch", "4|0", "0").is_none());
        for u in [
            "https://profile.img.sooplive.com/LOGO/fi/fixture/other.jpg",
            "https://profile.img.sooplive.com/LOGO/xx/fixture/fixture.jpg",
            "https://profile.img.sooplive.com/LOGO/fi/fixture/fixture.jpg?url=x",
            "https://profile.img.sooplive.com.evil/LOGO/fi/fixture/fixture.jpg",
        ] {
            assert!(canonical_url(u).is_none());
        }
        assert_ne!(
            r.id(),
            Request::for_context("fixture", "fixture", "other-epoch", "4|0", "0")
                .unwrap()
                .id()
        );
    }
}
