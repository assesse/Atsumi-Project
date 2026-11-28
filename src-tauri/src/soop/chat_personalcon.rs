//! Capture-time personalcon selection from the already validated public
//! CHANNEL response. Never fetches at replay time or accepts caller URLs.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ROOT: &str = "https://static.file.sooplive.com/spcon/";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub image_asset_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_nickname: Option<String>,
    pub tier: u8,
    pub months: i16,
    pub source: String,
}
pub(crate) fn from_stored(v: &Value) -> Option<Metadata> {
    let id = v["imageAssetId"]
        .as_str()
        .filter(|id| super::gifts::assets::valid_id(id))?;
    let tier = v["tier"].as_u64().filter(|n| (1..=3).contains(n))? as u8;
    let months = v["months"]
        .as_i64()
        .filter(|n| *n == -1 || (1..=1200).contains(n))? as i16;
    (v["source"] == "connectionMetadata").then(|| Metadata {
        image_asset_id: id.into(),
        tier,
        months,
        source: "connectionMetadata".into(),
        tier_nickname: v["tierNickname"]
            .as_str()
            .and_then(super::subscriptions::label),
    })
}
pub(crate) fn canonical_url(value: &str) -> Option<&str> {
    if value.len() > 1024 {
        return None;
    }
    let tail = value.strip_prefix(ROOT)?;
    let (file, version) = tail
        .split_once("?_=")
        .map_or((tail, None), |(p, q)| (p, Some(q)));
    if version
        .is_some_and(|v| v.is_empty() || v.len() > 20 || !v.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let stem = file.strip_suffix(".png")?;
    let default = matches!(stem, "personalcon_default" | "personalcon_default_2");
    let custom =
        !stem.is_empty() && stem.len() <= 128 && stem.bytes().all(|b| b.is_ascii_hexdigit());
    (default || custom).then_some(value)
}
fn months(v: &Value) -> Option<u16> {
    v.as_u64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|n| *n <= 1200)
        .map(|n| n as u16)
}
#[derive(Clone)]
struct Entry {
    month: u16,
    url: String,
}
/// A connection snapshot, not a promise that the channel never changes artwork.
pub(crate) struct Context {
    tiers: [Option<Vec<Entry>>; 3],
    legacy: Option<Vec<Entry>>,
    valid: bool,
    labels: [Option<String>; 2],
    nonce: String,
}
impl Context {
    pub(crate) fn from_channel(row: &Value) -> Self {
        let source = &row["PCON_OBJECT"];
        let parse = |v: &Value| -> Option<Vec<Entry>> {
            let rows = v.as_array()?;
            if rows.len() > 64 {
                return None;
            }
            rows.iter()
                .map(|r| {
                    Some(Entry {
                        month: months(&r["MONTH"])?,
                        url: canonical_url(r["FILENAME"].as_str()?)?.into(),
                    })
                })
                .collect()
        };
        let mut tiers: [Option<Vec<Entry>>; 3] = [None, None, None];
        let mut valid = source.is_null();
        let legacy = if source.is_array() {
            let list = parse(source);
            valid = list.is_some();
            list
        } else {
            None
        };
        if source.is_object() {
            valid = true;
            for (i, key) in ["tier1", "tier2", "tier3"].iter().enumerate() {
                if !source[*key].is_null() {
                    tiers[i] = parse(&source[*key]);
                    valid &= tiers[i].is_some();
                }
            }
        }
        Self {
            tiers,
            legacy,
            valid,
            labels: ["TIER1_NICK", "TIER2_NICK"]
                .map(|k| row[k].as_str().and_then(super::subscriptions::label)),
            nonce: uuid::Uuid::new_v4().to_string(),
        }
    }
    pub(crate) fn portrait(
        &self,
        channel: &str,
        sender: &str,
        flags: &str,
        representative: &str,
    ) -> Option<super::portrait::Request> {
        super::portrait::Request::for_context(channel, sender, &self.nonce, flags, representative)
    }
    /// The exact official selector: reverse original array order, not sorting.
    pub(crate) fn select(
        &self,
        flags: &str,
        accumulated: &str,
        representative: &str,
    ) -> (Option<Request>, bool) {
        let Some((a, b)) = flags.split_once('|') else {
            return (None, true);
        };
        let word = |s: &str| {
            s.parse::<i64>()
                .ok()
                .filter(|n| (i32::MIN as i64..=u32::MAX as i64).contains(n))
                .map(|n| n as u32)
        };
        let (Some(a), Some(b)) = (word(a), word(b)) else {
            return (None, true);
        };
        let streamer = a & 4 != 0;
        let tier = if b & (1 << 19) != 0 {
            2
        } else if b & (1 << 18) != 0 {
            1
        } else if b & (1 << 20) != 0 {
            3
        } else {
            0
        };
        if !streamer && a & (1 << 26) != 0 {
            return (None, false);
        }
        let rep = months(&Value::String(representative.into())).filter(|n| *n > 0);
        if tier == 0 && (!streamer || rep.is_none()) {
            return (None, false);
        }
        let (selected_tier, month) = if streamer && rep.is_some() {
            (1, rep.unwrap() as i16)
        } else {
            let month = if matches!(accumulated, "" | "0" | "-1") {
                -1
            } else {
                let Some(month) = months(&Value::String(accumulated.into())).filter(|n| *n > 0)
                else {
                    return (None, true);
                };
                month as i16
            };
            (tier, month)
        };
        if !self.valid {
            return (None, true);
        }
        // Official _tier is empty for tier3; it uses no tier3 list and basic default.
        let list = self.legacy.as_ref().or_else(|| {
            if selected_tier <= 2 {
                self.tiers[(selected_tier - 1) as usize].as_ref()
            } else {
                None
            }
        });
        let chosen = list.and_then(|items| items.iter().rev().find(|v| month >= v.month as i16));
        let url = chosen.map(|v| v.url.clone()).unwrap_or_else(|| {
            format!(
                "{ROOT}{}.png",
                if selected_tier == 2 {
                    "personalcon_default_2"
                } else {
                    "personalcon_default"
                }
            )
        });
        let id = format!(
            "{:x}",
            Sha256::digest(format!("soop-personalcon-v1:{}:{url}", self.nonce))
        );
        let metadata = Metadata {
            image_asset_id: id,
            tier: selected_tier,
            months: month,
            source: "connectionMetadata".into(),
            tier_nickname: if selected_tier <= 2 {
                self.labels[(selected_tier - 1) as usize].clone()
            } else {
                None
            },
        };
        (Some(Request { url, metadata }), false)
    }
}
/// Typed, non-deserializable network capability; raw archives carry only ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    url: String,
    metadata: Metadata,
}
impl Request {
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
    pub(crate) fn id(&self) -> &str {
        &self.metadata.image_asset_id
    }
    pub(crate) fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub(crate) fn matches(&self, metadata: &Metadata) -> bool {
        self.metadata == *metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> Context {
        Context::from_channel(
            &serde_json::json!({"TIER1_NICK":"Basic","PCON_OBJECT":{"tier1":[{"MONTH":0,"FILENAME":format!("{ROOT}aa.png?_=1")},{"MONTH":12,"FILENAME":format!("{ROOT}bb.png?_=1")},{"MONTH":3,"FILENAME":format!("{ROOT}cc.png?_=1")}]}}),
        )
    }
    #[test]
    fn official_order_tier_and_representative_selection() {
        let c = context();
        let r = c.select("32|262144", "17", "0").0.unwrap();
        assert!(r.url().contains("cc.png"));
        assert_eq!(r.metadata.months, 17);
        let r = c.select("4|524288", "99", "1").0.unwrap();
        assert!(r.url().contains("aa.png"));
        assert_eq!(r.metadata.tier, 1);
        assert_eq!(r.metadata.months, 1);
        assert!(c.select("67108864|262144", "3", "0").0.is_none());
        assert!(c.select("0|0", "0", "0").0.is_none());
        assert!(c
            .select("0|524288", "0", "0")
            .0
            .unwrap()
            .url()
            .ends_with("personalcon_default_2.png"));
        assert!(c
            .select("0|1048576", "0", "0")
            .0
            .unwrap()
            .url()
            .ends_with("personalcon_default.png"));
    }
    #[test]
    fn constructor_zero_and_missing_months_use_default_not_month_zero_custom() {
        let c = context();
        for month in ["0", "", "-1"] {
            let r = c.select("0|262144", month, "0").0.unwrap();
            assert!(r.url().ends_with("personalcon_default.png"));
            assert_eq!(r.metadata.months, -1);
        }
        let r = c.select("0|786432", "10", "0").0.unwrap();
        assert_eq!(r.metadata.tier, 2);
        assert!(r.url().ends_with("personalcon_default_2.png"));
        assert!(c.select("4|0", "10", "0").0.is_none());
        assert_eq!(c.select("0|262144", "999999", "0"), (None, true));
    }
    #[test]
    fn unsafe_metadata_never_mints_request_and_connections_do_not_reuse_mutable_url() {
        for u in [
            "https://evil.test/spcon/a.png",
            "https://static.file.sooplive.com.evil/spcon/a.png",
            "https://static.file.sooplive.com/spcon/../a.png",
            "https://static.file.sooplive.com/spcon/a.png?_=1&x=2",
            "https://static.file.sooplive.com/spcon/a.svg",
        ] {
            assert!(canonical_url(u).is_none());
            let c = Context::from_channel(
                &serde_json::json!({"PCON_OBJECT":[{"MONTH":0,"FILENAME":u}]}),
            );
            assert_eq!(c.select("0|262144", "1", "0"), (None, true));
        }
        let a = context().select("0|262144", "1", "0").0.unwrap();
        let b = context().select("0|262144", "1", "0").0.unwrap();
        assert_ne!(a.id(), b.id());
        assert_eq!(a.url(), b.url());
        assert_eq!(
            from_stored(&serde_json::to_value(a.metadata()).unwrap()),
            Some(a.metadata().clone())
        );
    }
}
