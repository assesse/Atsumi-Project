//! Display hints from the same authorized response; never alter canonical IDs/tags.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TagTranslation {
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ko: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub en: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub romaji: Option<String>,
}

fn field(value: &Value, original: &str) -> Option<String> {
    let raw = value.as_str()?;
    let text = raw.trim();
    (!text.is_empty() && text != original && text.chars().count() <= 120
        && !raw.chars().any(|c| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')))
        .then(|| text.to_owned())
}

fn row(tag: &str, value: &Value) -> Option<TagTranslation> {
    if tag.trim().is_empty() || tag.chars().count() > 120 || !value.is_object()
        || tag.chars().any(|c| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) { return None; }
    let result = TagTranslation { tag: tag.to_owned(), ko: field(&value["ko"], tag),
        en: field(&value["en"], tag), romaji: field(&value["romaji"], tag) };
    (result.ko.is_some() || result.en.is_some() || result.romaji.is_some()).then_some(result)
}

pub(super) fn from_map(value: &Value, tags: &[String]) -> Vec<TagTranslation> {
    let Some(map) = value.as_object().filter(|v| v.len() <= 1000) else { return vec![]; };
    let mut result = Vec::new();
    for tag in tags.iter().take(70) {
        if result.iter().any(|v: &TagTranslation| v.tag == *tag) { continue; }
        if let Some(item) = map.get(tag).and_then(|v| row(tag, v)) { result.push(item); }
    }
    result
}

pub(super) fn from_work(value: &Value, tags: &[String]) -> Vec<TagTranslation> {
    let mut result = from_map(&value["tagTranslation"], tags);
    if let Some(rows) = value.pointer("/tags/tags").and_then(Value::as_array).filter(|v| v.len() <= 1000) {
        for value in rows.iter().take(50) {
            let Some(tag) = value["tag"].as_str().filter(|tag| tags.iter().any(|v| v == tag)) else { continue; };
            if let Some(item) = row(tag, &value["translation"]) { merge(&mut result, vec![item]); }
        }
    }
    result
}

pub(super) fn merge(target: &mut Vec<TagTranslation>, incoming: Vec<TagTranslation>) {
    for item in incoming {
        if let Some(current) = target.iter_mut().find(|v| v.tag == item.tag) {
            // Prefer the more specific work-owned hint; fill only missing fields.
            if current.ko.is_none() { current.ko = item.ko; }
            if current.en.is_none() { current.en = item.en; }
            if current.romaji.is_none() { current.romaji = item.romaji; }
        } else if target.len() < 70 { target.push(item); }
    }
}

pub(super) fn enrich(work: &mut super::PixivWork, value: &Value) {
    merge(&mut work.tag_translations, from_map(value, &work.tags));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn exact_join_keeps_korean_english_and_reading_without_changing_tags() {
        let tags = vec!["風景".into(), "猫".into(), "風景".into()];
        let rows = from_map(&json!({"風景":{"ko":"풍경","en":"landscape","romaji":"fuukei"},"猫":{"romaji":"neko"},"unrelated":{"ko":"숨김"}}), &tags);
        assert_eq!(rows.len(), 2); assert_eq!(rows[0].ko.as_deref(), Some("풍경"));
        assert_eq!(rows[0].romaji.as_deref(), Some("fuukei")); assert_eq!(rows[1].tag, "猫");
    }
    #[test]
    fn malformed_fields_fail_soft_and_never_use_prefix_matches() {
        let value = json!({"風景":{"ko":"bad\nvalue","en":"landscape","romaji":42},"長名":{"ko":"x".repeat(121)},"猫":{"ko":"\u{202e}evil"},"原文":{"ko":"原文"}});
        let rows=from_map(&value,&["風景".into(),"風".into(),"長名".into(),"猫".into(),"原文".into()]);
        assert_eq!(rows.len(),1); assert_eq!(rows[0].ko,None); assert_eq!(rows[0].en.as_deref(),Some("landscape"));
        assert!(from_map(&json!([]), &["風景".into()]).is_empty());
    }
    #[test]
    fn embedded_work_hints_and_same_response_map_fill_missing_languages() {
        let tags=vec!["風景".into()];
        let value=json!({"tags":{"tags":[{"tag":"風景","translation":{"en":"landscape"}},{"tag":"other","translation":{"ko":"숨김"}}]}});
        let mut result=from_work(&value,&tags);
        merge(&mut result,from_map(&json!({"風景":{"ko":"풍경","en":"replacement","romaji":"fuukei"}}),&tags));
        assert_eq!(result.len(),1); assert_eq!(result[0].en.as_deref(),Some("landscape")); assert_eq!(result[0].ko.as_deref(),Some("풍경"));
    }
    #[test]
    fn bounded_projection_preserves_unicode() {
        let map: serde_json::Map<String,Value>=(0..1001).map(|i|(i.to_string(),json!({"ko":"한국어"}))).collect();
        assert!(from_map(&Value::Object(map),&["0".into()]).is_empty());
        assert!(field(&json!("𠮷".repeat(120)),"name").is_some());
    }
}
