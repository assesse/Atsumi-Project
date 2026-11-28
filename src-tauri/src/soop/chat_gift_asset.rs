//! Capture-only SOOP balloon image selection from pinned official rules.
//! Never deserializable: a stored/caller URL cannot become a network request.
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    url: String,
    id: String,
    kind: RequestKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestKind {
    Custom,
    Standard(StandardArt),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StandardArt { Step1, Step2, Step3, Step4, Step5, Step6, Video }
impl StandardArt {
    fn select(gift: &super::SoopGiftMetadata) -> Option<Self> {
        if gift.count == 0 { return None; }
        if gift.kind == super::SoopGiftKind::VideoBalloon { return Some(Self::Video); }
        if !gift.is_default { return None; }
        Some(match gift.count {
            1..=9 => Self::Step1, 10..=49 => Self::Step2, 50..=99 => Self::Step3,
            100..=499 => Self::Step4, 500..=999 => Self::Step5, _ => Self::Step6,
        })
    }
    fn url(self) -> &'static str {
        match self {
            Self::Step1 => "https://res.sooplive.com/new_player/items/ba_step1.png",
            Self::Step2 => "https://res.sooplive.com/new_player/items/ba_step2.png",
            Self::Step3 => "https://res.sooplive.com/new_player/items/ba_step3.png",
            Self::Step4 => "https://res.sooplive.com/new_player/items/ba_step4.png",
            Self::Step5 => "https://res.sooplive.com/new_player/items/ba_step5.png",
            Self::Step6 => "https://res.sooplive.com/new_player/items/ba_step6.png",
            Self::Video => "https://res.sooplive.com/new_player/items/m_video_balloon.png",
        }
    }
}
/// One immutable response identity per selector and bound connection. A reconnect
/// cannot reuse a previous connection's mutable-URL response as new history.
pub(crate) struct Context { namespace: uuid::Uuid }
impl Context {
    pub(crate) fn new() -> Self { Self { namespace: uuid::Uuid::new_v4() } }
    pub(crate) fn mission(&self, fields:&[&str])->Option<crate::soop::chat::missions::Request> {
        crate::soop::chat::missions::Request::from_fields(self.namespace,fields)
    }
    pub(crate) fn from_fields(&self, service: u16, fields: &[&str], bound_channel: &str) -> Option<Request> {
        let gift = super::parse(service, fields).ok()??;
        let art = StandardArt::select(&gift.metadata)?;
        let index = match service { 18 => 0, 33 | 105 => 1, _ => return None };
        if !broadcaster(fields.get(index)?)?.eq_ignore_ascii_case(bound_channel) { return None; }
        let url = art.url().to_owned();
        let id = format!("{:x}", Sha256::digest(format!("soop-standard-gift-v1:{}:{url}", self.namespace)));
        Some(Request { url, id, kind: RequestKind::Standard(art) })
    }
}
fn name(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn language(value: &str) -> Option<&str> {
    if value.is_empty() {
        return Some("ko_KR");
    }
    let bytes = value.as_bytes();
    ((bytes.len() == 2 && bytes.iter().all(u8::is_ascii_lowercase))
        || (bytes.len() == 5
            && bytes[..2].iter().all(u8::is_ascii_lowercase)
            && bytes[2] == b'_'
            && bytes[3..].iter().all(u8::is_ascii_uppercase)))
    .then_some(value)
}
fn version(value: &str) -> bool {
    value.len() <= 20 && value.bytes().all(|b| b.is_ascii_digit())
}
fn broadcaster(value: &str) -> Option<&str> {
    let base = if value.ends_with(')') {
        let (base, suffix) = value.rsplit_once('(')?;
        if suffix.len() != 2 || !suffix.as_bytes()[0].is_ascii_digit() {
            return None;
        }
        base
    } else {
        value
    };
    (!base.is_empty()
        && base.len() <= 40
        && base.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    .then_some(base)
}
pub(crate) fn valid_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub(crate) fn canonical_url(value: &str) -> Option<&str> {
    let (path, query) = value
        .split_once('?')
        .map_or((value, None), |(p, q)| (p, Some(q)));
    if let Some(query) = query {
        let value = query.strip_prefix("v=")?;
        if value.is_empty() || !version(value) {
            return None;
        }
    }
    let file = path
        .strip_prefix("https://res.sooplive.com/new_player/items/m_balloon_")
        .or_else(|| path.strip_prefix("https://static.file.sooplive.com/starballoon/story_m/"))?
        .strip_suffix(".png")?;
    name(file, 260).then_some(value)
}
impl Request {
    pub(crate) fn from_fields(service: u16, fields: &[&str], bound_channel: &str) -> Option<Self> {
        let gift = super::parse(service, fields).ok()??;
        if gift.metadata.is_default || gift.metadata.kind != super::SoopGiftKind::Balloon {
            return None;
        }
        let (channel, file, lang, modified) = match service {
            18 => (0, 7, 12, 13),
            33 => (1, 8, 13, 14),
            _ => return None,
        };
        let channel = broadcaster(fields.get(channel)?)?;
        if !channel.eq_ignore_ascii_case(bound_channel) {
            return None;
        }
        let file = *fields.get(file)?;
        if !name(file, 256) {
            return None;
        }
        let lang = language(fields.get(lang).copied().unwrap_or(""))?;
        let modified = fields.get(modified).copied().unwrap_or("");
        if !version(modified) {
            return None;
        }
        let signature = file.contains(channel);
        let event = file.contains("evt");
        let suffix = if event {
            match lang {
                "ko_KR" => "",
                "zh_CN" => "_cn",
                "zh_TW" => "_tw",
                _ => "",
            }
        } else if !signature && lang != "ko_KR" {
            "_en"
        } else {
            ""
        };
        let suffix = if event && !matches!(lang, "ko_KR" | "zh_CN" | "zh_TW") {
            format!("_{}", &lang[..2])
        } else {
            suffix.into()
        };
        let mut url = if signature || event {
            format!("https://static.file.sooplive.com/starballoon/story_m/{file}{suffix}.png")
        } else {
            format!("https://res.sooplive.com/new_player/items/m_balloon_{file}{suffix}.png")
        };
        if !modified.is_empty() {
            url.push_str("?v=");
            url.push_str(modified);
        }
        canonical_url(&url)?;
        let id = format!("{:x}", Sha256::digest(format!("soop-gift-v1:{url}")));
        Some(Self { url, id, kind: RequestKind::Custom })
    }
    pub(crate) fn valid(&self) -> bool {
        valid_id(&self.id) && match self.kind {
            RequestKind::Custom => canonical_url(&self.url) == Some(self.url.as_str()),
            RequestKind::Standard(art) => self.url == art.url(),
        }
    }
    pub(crate) fn matches(&self, gift: &super::SoopGiftMetadata) -> bool {
        self.valid() && gift.count > 0 && match self.kind {
            RequestKind::Custom => !gift.is_default && gift.kind == super::SoopGiftKind::Balloon,
            RequestKind::Standard(art) => StandardArt::select(gift) == Some(art),
        }
    }
    pub(crate) fn apply(&self, gift: &mut super::SoopGiftMetadata) -> bool {
        if !self.matches(gift) || gift.image_asset_id.as_deref().is_some_and(|id| id != self.id) { return false; }
        gift.image_asset_id = Some(self.id.clone());
        true
    }
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
}

#[cfg(test)]
#[path = "chat_gift_standard_tests.rs"]
mod standard_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn fields(service: u16, file: &str, lang: &str, stamp: &str) -> Vec<String> {
        let mut values = vec![String::new(); if service == 18 { 14 } else { 15 }];
        for (index, value) in if service == 18 {
            vec![
                (0, "fixture(1)"),
                (2, "Public"),
                (3, "50"),
                (7, file),
                (8, "0"),
                (12, lang),
                (13, stamp),
            ]
        } else {
            vec![
                (1, "fixture"),
                (4, "Public"),
                (5, "50"),
                (8, file),
                (9, "0"),
                (13, lang),
                (14, stamp),
            ]
        } {
            values[index] = value.into();
        }
        values
    }
    fn request(service: u16, file: &str, lang: &str, stamp: &str) -> Option<Request> {
        let values = fields(service, file, lang, stamp);
        Request::from_fields(
            service,
            &values.iter().map(String::as_str).collect::<Vec<_>>(),
            "fixture",
        )
    }
    #[test]
    fn official_context_suffix_version_and_overlap_rules() {
        for service in [18, 33] {
            for (file, lang, host, result) in [
                ("50", "ko_KR", "res", "m_balloon_50"),
                ("50", "en_US", "res", "m_balloon_50_en"),
                ("50", "ja_JP", "res", "m_balloon_50_en"),
                ("fixture_art", "ja_JP", "static", "fixture_art"),
                ("evt_50", "ko_KR", "static", "evt_50"),
                ("evt_50", "zh_CN", "static", "evt_50_cn"),
                ("evt_50", "zh_TW", "static", "evt_50_tw"),
                ("evt_50", "en_US", "static", "evt_50_en"),
                ("evt_fixture", "ja_JP", "static", "evt_fixture_ja"),
            ] {
                let r = request(service, file, lang, "123").unwrap();
                assert!(r.url().starts_with(&format!("https://{host}")));
                assert!(r.url().ends_with(&format!("/{result}.png?v=123")));
            }
        }
        assert_eq!(
            request(18, "50", "", "").unwrap(),
            request(18, "50", "ko_KR", "").unwrap()
        );
        let mut absent = fields(18, "50", "", "");
        absent.truncate(10);
        assert_eq!(
            Request::from_fields(
                18,
                &absent.iter().map(String::as_str).collect::<Vec<_>>(),
                "FIXTURE"
            ),
            request(18, "50", "", "")
        );
        assert!(request(18, "", "", "").is_none());
        assert_ne!(
            request(18, "50", "", "").unwrap().id(),
            request(18, "50", "", "0").unwrap().id()
        );
        assert_ne!(
            request(18, "50", "", "").unwrap().id(),
            request(18, "50", "ja_JP", "").unwrap().id()
        );
    }
    #[test]
    fn unknown_selectors_and_unbound_broadcaster_never_make_urls() {
        for bad in [
            "../x",
            "a.b",
            "a/b",
            "a\\b",
            "a%20b",
            "https://evil/x",
            "a?x",
            "a#x",
        ] {
            assert!(request(18, bad, "", "").is_none());
        }
        for bad in ["ko KR", "english", "../../", "ko_kr", "zh_CN?x"] {
            assert!(request(18, "50", bad, "").is_none());
        }
        for bad in ["-1", "a", "1&url=x", "1#x", "123456789012345678901"] {
            assert!(request(18, "50", "", bad).is_none());
        }
        let values = fields(18, "fixture_art", "", "");
        let refs = values.iter().map(String::as_str).collect::<Vec<_>>();
        assert!(Request::from_fields(18, &refs, "different").is_none());
        for bad in [
            "https://res.sooplive.com.evil/new_player/items/m_balloon_50.png",
            "http://res.sooplive.com/new_player/items/m_balloon_50.png",
            "https://user@res.sooplive.com/new_player/items/m_balloon_50.png",
            "https://res.sooplive.com:444/new_player/items/m_balloon_50.png",
            "https://res.sooplive.com/new_player/items/m_balloon_../x.png",
            "https://res.sooplive.com/new_player/items/m_balloon_50.png?v=1&x=2",
        ] {
            assert!(canonical_url(bad).is_none());
        }
    }
}
