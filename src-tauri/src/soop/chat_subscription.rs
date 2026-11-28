//! Receive-only public subscription ceremonies91/93. Pinned original LIVE
//! product table and renderers: soop-subscription-source-proof.json.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[path = "chat_subscription_products.rs"]
mod products;

const ROOT: &str = "https://static.file.sooplive.com/subscription_ceremony/m/";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Event {
    New,
    Renewal,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub event: Event,
    pub display_tier: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_vod: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_months: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub months: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_asset_id: Option<String>,
}
impl Metadata {
    pub(crate) fn ids(&self) -> Vec<&str> {
        [&self.image_asset_id, &self.fallback_asset_id]
            .into_iter()
            .filter_map(|v| v.as_deref())
            .collect()
    }
    fn same_display(&self, other: &Self) -> bool {
        self.event == other.event
            && self.display_tier == other.display_tier
            && self.from_vod == other.from_vod
            && self.product_months == other.product_months
            && self.months == other.months
    }
}
pub(crate) fn label(value: &str) -> Option<String> {
    (!value.trim().is_empty()
        && value.len() <= 320
        && value.chars().count() <= 80
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}
fn asset_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| super::gifts::assets::valid_id(s))
        .map(str::to_owned)
}
fn absent(value: &Value) -> bool {
    value.is_null()
}
pub(crate) fn from_stored(value: &Value) -> Option<Metadata> {
    let event = match value["event"].as_str()? {
        "new" => Event::New,
        "renewal" => Event::Renewal,
        _ => return None,
    };
    let tier = value["displayTier"].as_u64()?;
    if !matches!(tier, 1 | 2) {
        return None;
    }
    let (from_vod, product_months, months) = match event {
        Event::New => {
            let months = value["productMonths"].as_u64()?;
            if !matches!(months, 1 | 3 | 6 | 12) || !absent(&value["months"]) {
                return None;
            }
            (Some(value["fromVod"].as_bool()?), Some(months as u32), None)
        }
        Event::Renewal => {
            let months = value["months"].as_u64()?;
            if !(1..=1200).contains(&months)
                || !absent(&value["fromVod"])
                || !absent(&value["productMonths"])
            {
                return None;
            }
            (None, None, Some(months as u32))
        }
    };
    Some(Metadata {
        event,
        display_tier: tier as u8,
        from_vod,
        product_months,
        months,
        tier_nickname: value["tierNickname"].as_str().and_then(label),
        image_asset_id: asset_id(&value["imageAssetId"]),
        fallback_asset_id: asset_id(&value["fallbackAssetId"]),
    })
}
pub(crate) fn invalid_stored(value: &Value) -> bool {
    if value.is_null() {
        return false;
    }
    if from_stored(value).is_none() {
        return true;
    }
    ["imageAssetId", "fallbackAssetId"]
        .iter()
        .any(|key| !value[*key].is_null() && asset_id(&value[*key]).is_none())
        || (!value["tierNickname"].is_null()
            && value["tierNickname"].as_str().and_then(label).is_none())
}
fn decimal(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}
pub(crate) fn canonical_url(value: &str) -> Option<&str> {
    let tail = value.strip_prefix(ROOT)?;
    let file = if let Some((file, version)) = tail.split_once("?v=") {
        if !decimal(version, 20) {
            return None;
        }
        file
    } else {
        tail
    };
    let stem = file.strip_prefix("gudok_")?.strip_suffix(".png")?;
    let stem = stem.strip_suffix("_en").unwrap_or(stem);
    let valid = if let Some(month) = stem.strip_suffix("monthsGift_1") {
        matches!(month, "1" | "3" | "6" | "12")
    } else if let Some(days) = stem.strip_suffix("days_1") {
        matches!(days, "30" | "90" | "180" | "360")
    } else {
        decimal(stem, 4) && stem.parse::<u32>().is_ok_and(|n| (1..=1200).contains(&n))
    };
    valid.then_some(value)
}
/// Not Deserialize: only validated receive fields can mint this capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    url: String,
    id: String,
    display: Metadata,
    fallback: bool,
}
impl Request {
    fn new(url: String, display: &Metadata, fallback: bool) -> Self {
        debug_assert!(canonical_url(&url).is_some());
        let id = format!(
            "{:x}",
            Sha256::digest(format!("soop-subscription-v1:{url}"))
        );
        Self {
            url,
            id,
            display: display.clone(),
            fallback,
        }
    }
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn apply(&self, metadata: &mut Metadata) -> bool {
        if !self.display.same_display(metadata) {
            return false;
        }
        if self.fallback {
            metadata.fallback_asset_id = Some(self.id.clone());
        } else {
            metadata.image_asset_id = Some(self.id.clone());
        }
        true
    }
}
pub(crate) struct Public<'a> {
    pub nickname: &'a str,
    pub metadata: Metadata,
    pub requests: Vec<Request>,
    pub partial: bool,
}
pub(crate) fn parse<'a>(
    service: u16,
    fields: &[&'a str],
    labels: Option<&[Option<String>; 2]>,
) -> Result<Public<'a>, ()> {
    if !matches!(service, 91 | 93) || !(8..=32).contains(&fields.len()) {
        return Err(());
    }
    let nickname = fields[if service == 91 { 3 } else { 2 }];
    if nickname.len() > 512 {
        return Err(());
    }
    let mut metadata = Metadata {
        event: if service == 91 {
            Event::New
        } else {
            Event::Renewal
        },
        display_tier: 1,
        tier_nickname: None,
        from_vod: None,
        product_months: None,
        months: None,
        image_asset_id: None,
        fallback_asset_id: None,
    };
    let mut partial = false;
    let filename = if service == 91 {
        let code = fields[4];
        if code.len() > 11 || !(code == "-1" || decimal(code, 10)) {
            return Err(());
        }
        let code = code.parse::<i32>().map_err(|_| ())?;
        let (product, from_vod) = products::find(code).ok_or(())?;
        metadata.display_tier = product.tier;
        metadata.from_vod = Some(from_vod);
        metadata.product_months = Some(product.months);
        if product.gift {
            format!("{}monthsGift_1", product.months)
        } else if !product.auto_pay {
            format!("{}days_1", 30 * product.months)
        } else {
            "1".into()
        }
    } else {
        let month = fields[3];
        if !decimal(month, 4) {
            return Err(());
        }
        let month = month.parse::<u32>().map_err(|_| ())?;
        if !(1..=1200).contains(&month) {
            return Err(());
        }
        metadata.months = Some(month);
        let tier = fields[7];
        // Original Number(tier) === 2 selects Plus; other numeric tiers use Basic.
        match if tier.trim().is_empty() {
            Some(0.0)
        } else if tier.len() <= 32 {
            tier.parse::<f64>().ok().filter(|n| n.is_finite())
        } else {
            None
        } {
            Some(value) => metadata.display_tier = if value == 2.0 { 2 } else { 1 },
            None => partial = true,
        }
        month.to_string()
    };
    metadata.tier_nickname =
        labels.and_then(|labels| labels[(metadata.display_tier - 1) as usize].clone());
    let language = fields.get(9).copied().unwrap_or("");
    let valid_language = language.len() <= 16
        && language
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || b == b'_');
    let suffix = if valid_language && !language.is_empty() && language != "ko_KR" {
        "_en"
    } else {
        ""
    };
    let version = fields.get(10).copied().unwrap_or("");
    let valid_version = version.is_empty() || decimal(version, 20);
    let mut requests = Vec::with_capacity(2);
    if valid_language && valid_version {
        let query = if version.is_empty() {
            String::new()
        } else {
            format!("?v={version}")
        };
        requests.push(Request::new(
            format!("{ROOT}gudok_{filename}{suffix}.png{query}"),
            &metadata,
            false,
        ));
    } else {
        partial = true;
    }
    requests.push(Request::new(
        format!("{ROOT}gudok_1{suffix}.png"),
        &metadata,
        true,
    ));
    Ok(Public {
        nickname,
        metadata,
        requests,
        partial,
    })
}

#[cfg(test)]
#[path = "chat_subscription_tests.rs"]
mod tests;
