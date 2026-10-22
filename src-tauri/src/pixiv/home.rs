//! Official finite recommendation pool. Search suggestions provide tags only.
use super::{
    json_with_session, session, text, valid_id, work_with_display, DisplayPolicy, PixivSearchPage,
    PixivWork, SITE,
};
use reqwest::Url;
use serde::Serialize;
use serde_json::Value;
use super::localization;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const SCHEMA_ERROR: &str = "Pixiv 추천 응답 형식이 변경되었습니다. 다시 시도해 주세요.";
const CURSOR_ERROR: &str =
    "추천 목록이 만료되었거나 로그인·표시 설정이 바뀌었습니다. 추천 작품을 새로고침해 주세요.";
const MAX_SOURCE_IDS: usize = 1000;
const BATCH_SIZE: usize = 60;
const MAX_POOLS: usize = 24;
const POOL_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_TAGS: usize = 20;
const MAX_FAVORITE_TAGS: usize = 10;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSuggestions {
    pub recommended_tags: Vec<String>,
    pub popular_tags: Vec<String>,
    pub popular_novel_tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorite_tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorite_tag_translations: Option<Vec<FavoriteTagTranslation>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tag_translations: Vec<localization::TagTranslation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FavoriteTagTranslation {
    pub tag: String,
    pub translation: String,
    pub language: &'static str,
}

#[derive(Clone)]
struct Pool {
    key: String,
    scope: session::Scope,
    safe_only: bool,
    ids: Arc<Vec<String>>,
    touched: Instant,
}
#[derive(Default)]
struct Pools(VecDeque<Pool>);
impl Pools {
    fn insert(&mut self, pool: Pool, now: Instant) {
        self.0
            .retain(|row| now.saturating_duration_since(row.touched) < POOL_TTL);
        while self.0.len() >= MAX_POOLS {
            self.0.pop_front();
        }
        self.0.push_back(pool);
    }
    fn get(
        &mut self,
        key: &str,
        lease: &session::Lease,
        safe_only: bool,
        now: Instant,
    ) -> Result<Pool, String> {
        self.0
            .retain(|row| now.saturating_duration_since(row.touched) < POOL_TTL);
        let index = self
            .0
            .iter()
            .position(|row| {
                row.key == key && row.scope.matches(lease) && row.safe_only == safe_only
            })
            .ok_or(CURSOR_ERROR)?;
        let mut pool = self.0.remove(index).ok_or(CURSOR_ERROR)?;
        pool.touched = now;
        self.0.push_back(pool.clone());
        Ok(pool)
    }
}
fn pools() -> &'static Mutex<Pools> {
    static POOLS: OnceLock<Mutex<Pools>> = OnceLock::new();
    POOLS.get_or_init(|| Mutex::new(Pools::default()))
}
fn cursor_parts(cursor: &str, page: u32) -> Result<(&str, usize), String> {
    if cursor.len() > 96 {
        return Err(CURSOR_ERROR.into());
    }
    let (key, position) = cursor.split_once(':').ok_or(CURSOR_ERROR)?;
    if key.len() != 32 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CURSOR_ERROR.into());
    }
    let offset = position.parse::<usize>().map_err(|_| CURSOR_ERROR)?;
    if position != offset.to_string()
        || offset == 0
        || offset >= MAX_SOURCE_IDS
        || offset % BATCH_SIZE != 0
        || page as usize != offset / BATCH_SIZE + 1
    {
        return Err(CURSOR_ERROR.into());
    }
    Ok((key, offset))
}
fn recommendation_ids(value: &Value) -> Result<Vec<String>, String> {
    if value["error"].as_bool() != Some(false) {
        return Err(SCHEMA_ERROR.into());
    }
    let rows = value
        .pointer("/body/recommended_work_ids")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= MAX_SOURCE_IDS)
        .ok_or(SCHEMA_ERROR)?;
    let mut seen = HashSet::new();
    let ids: Vec<_> = rows
        .iter()
        .map(text)
        .filter(|id| valid_id(id))
        .filter(|id| seen.insert(id.clone()))
        .collect();
    if !rows.is_empty() && ids.is_empty() {
        return Err(SCHEMA_ERROR.into());
    }
    Ok(ids)
}
fn tags(value: &Value) -> Vec<String> {
    let Some(rows) = value.as_array().filter(|rows| rows.len() <= MAX_SOURCE_IDS) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    rows.iter()
        .filter_map(|row| row.get("tag").and_then(Value::as_str))
        .map(str::trim)
        .filter(|tag| {
            !tag.is_empty() && tag.chars().count() <= 120 && !tag.chars().any(char::is_control)
        })
        .filter(|tag| seen.insert((*tag).to_owned()))
        .take(MAX_TAGS)
        .map(str::to_owned)
        .collect()
}
// Official search favorites are strings, not artwork bookmark folders or the
// object rows used by recommendation tags. None is unavailable, not empty.
fn favorite_tags(value: &Value) -> Option<Vec<String>> {
    let rows = value
        .as_array()
        .filter(|rows| rows.len() <= MAX_SOURCE_IDS)?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let tag = row.as_str()?.trim();
        if tag.chars().count() > 120 || tag.chars().any(char::is_control) {
            return None;
        }
        if !tag.is_empty() && seen.insert(tag) {
            if result.len() == MAX_FAVORITE_TAGS {
                return None;
            }
            result.push(tag.to_owned());
        }
    }
    Some(result)
}
// Display-only enrichment from the same official response. Do not translate the
// canonical search key, accept unrelated account tags, or make tags depend on it.
fn favorite_tag_translations(
    value: &Value,
    favorites: &[String],
) -> Option<Vec<FavoriteTagTranslation>> {
    let translations = value
        .as_object()
        .filter(|rows| rows.len() <= MAX_SOURCE_IDS)?;
    let result: Vec<_> = favorites
        .iter()
        .take(MAX_FAVORITE_TAGS)
        .filter_map(|tag| {
            let row = translations.get(tag)?.as_object()?;
            ["ko", "en"].into_iter().find_map(|language| {
                let raw = row.get(language)?.as_str()?;
                let translated = raw.trim();
                (!translated.is_empty()
                    && translated != tag
                    && translated.chars().count() <= 120
                    && !raw.chars().any(char::is_control))
                .then(|| FavoriteTagTranslation {
                    tag: tag.clone(),
                    translation: translated.to_owned(),
                    language,
                })
            })
        })
        .collect();
    (!result.is_empty()).then_some(result)
}
pub(super) fn suggestions(value: Option<&Value>, authenticated: bool) -> HomeSuggestions {
    let body = value
        .filter(|value| value["error"].as_bool() == Some(false))
        .map(|value| &value["body"]);
    let favorite_tags = authenticated
        .then(|| body.and_then(|body| favorite_tags(&body["myFavoriteTags"])))
        .flatten();
    let favorite_tag_translations = favorite_tags.as_ref().and_then(|favorites| {
        body.and_then(|body| favorite_tag_translations(&body["tagTranslation"], favorites))
    });
    let recommended_tags = body
            .map(|body| tags(&body["recommendTags"]["illust"]))
            .unwrap_or_default();
    let popular_tags = body
            .map(|body| tags(&body["popularTags"]["illust"]))
            .unwrap_or_default();
    let popular_novel_tags = body
            .map(|body| tags(&body["popularTags"]["novel"]))
            .unwrap_or_default();
    let visible_tags = favorite_tags.iter().flatten().chain(recommended_tags.iter())
        .chain(popular_tags.iter()).chain(popular_novel_tags.iter()).cloned().collect::<Vec<_>>();
    let tag_translations = body.map(|body| localization::from_map(&body["tagTranslation"], &visible_tags)).unwrap_or_default();
    HomeSuggestions {
        recommended_tags, popular_tags, popular_novel_tags, tag_translations,
        favorite_tags,
        favorite_tag_translations,
    }
}
#[cfg(test)]
fn project_batch(
    value: &Value,
    ids: &[String],
    authenticated: bool,
) -> Result<Vec<PixivWork>, String> {
    project_batch_with_display(value, ids, DisplayPolicy::new(authenticated, false))
}
fn project_batch_with_display(
    value: &Value,
    ids: &[String],
    display: DisplayPolicy,
) -> Result<Vec<PixivWork>, String> {
    if value["error"].as_bool() != Some(false) || ids.len() > BATCH_SIZE {
        return Err(SCHEMA_ERROR.into());
    }
    let rows = value
        .pointer("/body/illusts")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= MAX_SOURCE_IDS)
        .ok_or(SCHEMA_ERROR)?;
    let mut by_id = HashMap::new();
    for row in rows {
        let id = text(&row["id"]);
        if valid_id(&id) {
            by_id.entry(id).or_insert(row);
        }
    }
    if !ids.is_empty() && !ids.iter().any(|id| by_id.contains_key(id)) {
        return Err(SCHEMA_ERROR.into());
    }
    let mut seen = HashSet::new();
    Ok(ids
        .iter()
        .filter(|id| seen.insert((*id).clone()))
        .filter_map(|id| {
            let row = *by_id.get(id)?;
            let mut work = work_with_display(row, display)?;
            localization::enrich(&mut work, &value["body"]["tagTranslation"]);
            (work.id == *id && work.thumbnail_url.is_some()).then_some(work)
        })
        .collect())
}
pub(super) fn hydrate(
    ids: &[String],
    lease: &session::Lease,
    safe_only: bool,
) -> Result<Vec<PixivWork>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut url =
        Url::parse(&format!("{SITE}/ajax/illust/recommend/illusts")).map_err(|_| SCHEMA_ERROR)?;
    for id in ids {
        url.query_pairs_mut().append_pair("illust_ids[]", id);
    }
    url.query_pairs_mut().append_pair("lang", "ko");
    project_batch_with_display(
        &json_with_session(url.as_str(), lease)?,
        ids,
        DisplayPolicy::new(lease.authenticated(), safe_only),
    )
}
fn result(
    pool: &Pool,
    offset: usize,
    items: Vec<PixivWork>,
    home_suggestions: Option<HomeSuggestions>,
) -> PixivSearchPage {
    let end = (offset + BATCH_SIZE).min(pool.ids.len());
    let home_cursor = (end < pool.ids.len()).then(|| format!("{}:{end}", pool.key));
    PixivSearchPage {
        items,
        page: (offset / BATCH_SIZE + 1) as u32,
        has_more: home_cursor.is_some(),
        home_suggestions,
        home_cursor,
    }
}
pub(super) fn read(
    lease: &session::Lease,
    page: u32,
    cursor: Option<&str>,
    safe_only: bool,
) -> Result<PixivSearchPage, String> {
    lease.check()?;
    if let Some(cursor) = cursor {
        let (key, offset) = cursor_parts(cursor, page)?;
        let pool =
            pools()
                .lock()
                .map_err(|_| CURSOR_ERROR)?
                .get(key, lease, safe_only, Instant::now())?;
        if offset >= pool.ids.len() {
            return Err(CURSOR_ERROR.into());
        }
        let items = hydrate(
            &pool.ids[offset..(offset + BATCH_SIZE).min(pool.ids.len())],
            lease,
            safe_only,
        )?;
        lease.check()?;
        return Ok(result(&pool, offset, items, None));
    }
    if page != 1 {
        return Err(CURSOR_ERROR.into());
    }
    // Official mobile DiscoveryFeed initializes once and consumes the ID pool.
    // No invented server offset, ranking substitution or repeated feed poll.
    let value = json_with_session(
        &format!(
            "{SITE}/touch/ajax/recommender/illust?mode={}",
            DisplayPolicy::new(lease.authenticated(), safe_only).query_mode()
        ),
        lease,
    )?;
    let ids = recommendation_ids(&value)?;
    let items = hydrate(&ids[..BATCH_SIZE.min(ids.len())], lease, safe_only)?;
    // Optional tags cannot turn valid recommendations into a failed page.
    let tag_value = json_with_session(
        &format!("{SITE}/ajax/search/suggestion?mode=all&lang=ko"),
        lease,
    )
    .ok();
    retain(lease, safe_only, ids, items, Some(suggestions(tag_value.as_ref(), lease.authenticated())))
}

// Desktop home and related discovery share only the bounded continuation
// transport, never account state or an untrusted renderer-provided ID list.
pub(super) fn retain(
    lease: &session::Lease,
    safe_only: bool,
    ids: Vec<String>,
    items: Vec<PixivWork>,
    suggestions: Option<HomeSuggestions>,
) -> Result<PixivSearchPage, String> {
    if ids.len() > MAX_SOURCE_IDS || ids.iter().any(|id| !valid_id(id)) || items.len() > BATCH_SIZE {
        return Err(SCHEMA_ERROR.into());
    }
    let pool = Pool {
        key: uuid::Uuid::new_v4().simple().to_string(),
        scope: lease.scope(),
        safe_only,
        ids: Arc::new(ids),
        touched: Instant::now(),
    };
    let response = result(
        &pool,
        0,
        items,
        suggestions,
    );
    lease.with_current(|| {
        if response.has_more {
            pools()
                .lock()
                .map_err(|_| CURSOR_ERROR)?
                .insert(pool, Instant::now());
        }
        Ok(response)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn row(id: &str) -> Value {
        json!({"id":id,"title":"Fixture","userId":"77","userName":"Fixture artist","url":"https://i.pximg.net/c/480x480/img-master/img/fixture.jpg","xRestrict":0,"restrict":0,"sl":2,"isMasked":false,"width":640,"height":480,"pageCount":1,"tags":["landscape"]})
    }
    fn fixture() -> Value {
        json!({"error":false,"body":{"illusts":[row("1"),row("2"),row("3"),row("4")]}})
    }
    fn ids() -> Vec<String> {
        ["3", "1", "3", "2", "99"].map(str::to_owned).into()
    }
    fn pool(lease: &session::Lease, count: usize, now: Instant) -> Pool {
        Pool {
            key: uuid::Uuid::new_v4().simple().to_string(),
            scope: lease.scope(),
            safe_only: false,
            ids: Arc::new((1..=count).map(|n| n.to_string()).collect()),
            touched: now,
        }
    }
    #[test]
    fn recommendation_pool_is_bounded_canonical_and_ordered_not_search_suggestions() {
        let value =
            json!({"error":false,"body":{"recommended_work_ids":["3",1,"3","2","01","invalid"]}});
        assert_eq!(recommendation_ids(&value).unwrap(), ["3", "1", "2"]);
        for value in [
            json!({}),
            json!({"error":true,"body":{"recommended_work_ids":[]}}),
            json!({"error":false,"body":{"recommended_work_ids":["bad","01"]}}),
            json!({"error":false,"body":{"recommended_work_ids":vec!["1";1001]}}),
        ] {
            assert!(recommendation_ids(&value).is_err());
        }
        assert!(
            recommendation_ids(&json!({"error":false,"body":{"recommended_work_ids":[]}}))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn hydration_preserves_order_and_rejects_unrequested_aliases() {
        assert_eq!(
            project_batch(&fixture(), &ids(), false)
                .unwrap()
                .iter()
                .map(|work| work.id.as_str())
                .collect::<Vec<_>>(),
            ["3", "1", "2"]
        );
        let mut value = fixture();
        value["body"]["illusts"][0]["illust_id"] = json!("88");
        assert_eq!(project_batch(&value, &ids(), false).unwrap().len(), 2);
        value["body"]["illusts"][0]["illust_id"] = json!("1");
        assert_eq!(project_batch(&value, &ids(), false).unwrap().len(), 3);
    }
    #[test]
    fn missing_core_and_all_joins_are_errors_but_filtered_rows_are_valid_empty() {
        for value in [
            json!({}),
            json!({"error":true,"body":{"illusts":[]}}),
            json!({"error":false,"body":{"illusts":{}}}),
            json!({"error":false,"body":{"illusts":[]}}),
        ] {
            assert!(project_batch(&value, &ids(), false).is_err());
        }
        let mut value = fixture();
        for row in value["body"]["illusts"].as_array_mut().unwrap() {
            row["isMasked"] = json!(true);
        }
        assert!(project_batch(&value, &ids(), true).unwrap().is_empty());
        assert!(project_batch(&fixture(), &vec!["1".into(); 61], false).is_err());
    }
    #[test]
    fn current_access_guards_remain_for_guest_and_authenticated_sessions() {
        for (field, changed) in [
            ("isMasked", json!(true)),
            ("restrict", json!(1)),
            ("url", json!("https://evil.invalid/x.jpg")),
        ] {
            let mut value = fixture();
            value["body"]["illusts"][0][field] = changed;
            for authenticated in [false, true] {
                assert!(
                    !project_batch(&value, &ids(), authenticated)
                        .unwrap()
                        .iter()
                        .any(|work| work.id == "1"),
                    "{field}"
                );
            }
        }
        let mut value = fixture();
        value["body"]["illusts"][0]["isLoginOnly"] = json!(true);
        assert_eq!(project_batch(&value, &ids(), false).unwrap().len(), 2);
        assert_eq!(project_batch(&value, &ids(), true).unwrap().len(), 3);
    }
    #[test]
    fn recommendation_hydration_respects_display_preference() {
        let mut value = fixture();
        value["body"]["illusts"][0]["xRestrict"] = json!(1);
        assert_eq!(
            project_batch_with_display(&value, &ids(), DisplayPolicy::new(true, false))
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            project_batch_with_display(&value, &ids(), DisplayPolicy::new(true, true))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            project_batch_with_display(&value, &ids(), DisplayPolicy::new(false, false))
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn malformed_access_markers_never_become_permissive_defaults() {
        for field in ["xRestrict", "restrict", "sl"] {
            for changed in [
                json!(null),
                json!("0"),
                json!(false),
                json!(-1),
                json!(0.5),
                json!({}),
                json!([]),
                json!(99),
            ] {
                let mut value = fixture();
                value["body"]["illusts"][0][field] = changed;
                for authenticated in [false, true] {
                    assert_eq!(
                        project_batch(&value, &ids(), authenticated).unwrap().len(),
                        2,
                        "{field}"
                    );
                }
            }
        }
        for field in ["isMasked", "isLoginOnly"] {
            for changed in [json!(null), json!("false"), json!(0), json!({}), json!([])] {
                let mut value = fixture();
                value["body"]["illusts"][0][field] = changed;
                assert_eq!(project_batch(&value, &ids(), true).unwrap().len(), 2);
            }
        }
        for changed in [
            json!(null),
            json!(false),
            json!([]),
            json!({"sexual":null}),
            json!({"sexual":"false"}),
            json!({"sexual":0}),
        ] {
            let mut value = fixture();
            value["body"]["illusts"][0]["illust_content"] = changed;
            assert_eq!(project_batch(&value, &ids(), true).unwrap().len(), 2);
        }
    }
    #[test]
    fn optional_tags_are_bounded_deduplicated_and_independent() {
        let value = json!({"error":false,"body":{"recommendTags":{"illust":[{"tag":" sky "},{"tag":"sky"},{"tag":""},{"tag":"line\nbreak"},{"tag":"x".repeat(121)}]},"popularTags":{"illust":(1..=30).map(|n|json!({"tag":format!("tag{n}")})).collect::<Vec<_>>()}}});
        let result = suggestions(Some(&value), false);
        assert_eq!(result.recommended_tags, ["sky"]);
        assert_eq!(result.popular_tags.len(), 20);
        assert!(suggestions(None, false).popular_tags.is_empty());
    }
    #[test]
    fn favorite_search_tags_use_current_authenticated_string_schema_only() {
        let value = json!({"error":false,"body":{"myFavoriteTags":[" sky ","風景","sky","","   ","night sky"],"bookmarkTags":["not-a-search-favorite"],"recommendTags":{"illust":[{"tag":"ordinary-recommendation"}]}}});
        let authenticated = suggestions(Some(&value), true);
        assert_eq!(
            authenticated.favorite_tags.unwrap(),
            ["sky", "風景", "night sky"]
        );
        assert_eq!(authenticated.recommended_tags, ["ordinary-recommendation"]);
        let guest = serde_json::to_value(suggestions(Some(&value), false)).unwrap();
        assert!(guest.get("favoriteTags").is_none());
    }
    #[test]
    fn empty_favorites_are_distinct_from_failed_or_malformed_data() {
        let empty = suggestions(
            Some(&json!({"error":false,"body":{"myFavoriteTags":[]}})),
            true,
        );
        assert_eq!(empty.favorite_tags, Some(vec![]));
        assert_eq!(
            serde_json::to_value(empty).unwrap()["favoriteTags"],
            json!([])
        );
        assert!(suggestions(None, true).favorite_tags.is_none());
        for value in [
            json!({}),
            json!({"error":true,"body":{"myFavoriteTags":["old"]}}),
            json!({"error":false,"body":{"bookmarkTags":["not-favorite"]}}),
        ] {
            assert!(suggestions(Some(&value), true).favorite_tags.is_none());
        }
        for invalid in [
            json!(null),
            json!({}),
            json!("sky"),
            json!([{"tag":"sky"}]),
            json!(["sky", 1]),
            json!(["line\nbreak"]),
            json!(["x".repeat(121)]),
            json!(vec!["sky"; 1001]),
            json!((0..11).map(|n| format!("tag{n}")).collect::<Vec<_>>()),
        ] {
            assert!(favorite_tags(&invalid).is_none());
        }
        assert_eq!(
            favorite_tags(&json!((0..10)
                .map(|n| format!("tag{n}"))
                .collect::<Vec<_>>()))
            .unwrap()
            .len(),
            10
        );
    }
    #[test]
    fn favorite_tag_failure_keeps_public_suggestions_and_valid_feed_items() {
        let suggestions = suggestions(
            Some(
                &json!({"error":false,"body":{"myFavoriteTags":{},"recommendTags":{"illust":[{"tag":"sky"}]},"popularTags":{"illust":[{"tag":"landscape"}]}}}),
            ),
            true,
        );
        assert!(suggestions.favorite_tags.is_none());
        assert_eq!(suggestions.recommended_tags, ["sky"]);
        assert_eq!(suggestions.popular_tags, ["landscape"]);
        let lease = session::Authority::default().lease(session::session_header("fixture"));
        let response = result(
            &pool(&lease, 1000, Instant::now()),
            0,
            project_batch(&fixture(), &ids(), true).unwrap(),
            Some(suggestions),
        );
        assert_eq!(response.items.len(), 3);
        assert!(response.has_more);
    }
    #[test]
    fn favorite_translation_uses_official_korean_then_english_and_canonical_order() {
        let value = json!({"error":false,"body":{
            "myFavoriteTags":["OC","イラスト","創作","オリジナル"],
            "tagTranslation":{
                "オリジナル":{"ko":"창작","en":"original"},
                "イラスト":{"ko":" 일러스트 ","en":"Illustration"},
                "創作":{"ko":"창작","en":"creation"},
                "OC":{"ko":"","en":"original character","romaji":"ignored"},
                "unrelated":{"ko":"not a favorite"}
            }
        }});
        let result = serde_json::to_value(suggestions(Some(&value), true)).unwrap();
        assert_eq!(
            result["favoriteTags"],
            json!(["OC", "イラスト", "創作", "オリジナル"])
        );
        assert_eq!(
            result["favoriteTagTranslations"],
            json!([
                {"tag":"OC","translation":"original character","language":"en"},
                {"tag":"イラスト","translation":"일러스트","language":"ko"},
                {"tag":"創作","translation":"창작","language":"ko"},
                {"tag":"オリジナル","translation":"창작","language":"ko"}
            ])
        );
        let guest = serde_json::to_value(suggestions(Some(&value), false)).unwrap();
        assert!(guest.get("favoriteTags").is_none());
        assert!(guest.get("favoriteTagTranslations").is_none());
    }
    #[test]
    fn invalid_translation_is_optional_and_never_replaces_the_original_tag() {
        for invalid in [
            json!(null),
            json!({}),
            json!(true),
            json!(3),
            json!([]),
            json!(""),
            json!("   "),
            json!("sky"),
            json!("line\nbreak"),
            json!("tab\t"),
            json!("x".repeat(121)),
        ] {
            let value = json!({"error":false,"body":{"myFavoriteTags":["sky"],
                "tagTranslation":{"sky":{"ko":invalid,"romaji":"not a translation"}},
                "recommendTags":{"illust":[{"tag":"landscape"}]}}});
            let result = suggestions(Some(&value), true);
            assert_eq!(result.favorite_tags, Some(vec!["sky".into()]));
            assert_eq!(result.recommended_tags, ["landscape"]);
            assert!(result.favorite_tag_translations.is_none());
        }
        let value = json!({"error":false,"body":{"myFavoriteTags":["sky"],
            "tagTranslation":{"sky":{"ko":false,"en":"Sky translation"}}}});
        assert_eq!(
            suggestions(Some(&value), true)
                .favorite_tag_translations
                .unwrap()[0]
                .language,
            "en"
        );
        for malformed in [
            json!(null),
            json!("bad"),
            json!([]),
            json!({"sky":"untyped"}),
            json!({" sky ":{"ko":"하늘"}}),
            json!((0..1001)
                .map(|n| (format!("tag{n}"), json!({"ko":"하늘"})))
                .collect::<serde_json::Map<_, _>>()),
        ] {
            let value =
                json!({"error":false,"body":{"myFavoriteTags":["sky"],"tagTranslation":malformed}});
            let result = serde_json::to_value(suggestions(Some(&value), true)).unwrap();
            assert_eq!(result["favoriteTags"], json!(["sky"]));
            assert!(result.get("favoriteTagTranslations").is_none());
        }
    }
    #[test]
    fn favorite_translation_requires_verified_favorites_and_respects_unicode_bound() {
        for favorites in [json!(null), json!({}), json!(["sky", 1]), json!([])] {
            let value = json!({"error":false,"body":{"myFavoriteTags":favorites,
                "tagTranslation":{"sky":{"ko":"하늘"}}}});
            assert!(suggestions(Some(&value), true)
                .favorite_tag_translations
                .is_none());
        }
        let favorites: Vec<_> = (0..10).map(|n| format!("tag{n}")).collect();
        let translated: serde_json::Map<_, _> = favorites
            .iter()
            .map(|tag| (tag.clone(), json!({"ko":"가".repeat(120)})))
            .collect();
        let value =
            json!({"error":false,"body":{"myFavoriteTags":favorites,"tagTranslation":translated}});
        assert_eq!(
            suggestions(Some(&value), true)
                .favorite_tag_translations
                .unwrap()
                .len(),
            10
        );
        let mut failed = value;
        failed["error"] = json!(true);
        assert!(suggestions(Some(&failed), true)
            .favorite_tag_translations
            .is_none());
    }
    #[test]
    fn cursor_is_bounded_and_bound_to_batch_number() {
        let key = "0123456789abcdef0123456789abcdef";
        assert_eq!(cursor_parts(&format!("{key}:60"), 2).unwrap().1, 60);
        for (cursor, page) in [
            (format!("{key}:0"), 1),
            (format!("{key}:060"), 2),
            (format!("{key}:61"), 2),
            (format!("{key}:60"), 3),
            (format!("{key}:1020"), 18),
            ("https://evil.invalid/".into(), 2),
            ("x".repeat(97), 2),
        ] {
            assert!(cursor_parts(&cursor, page).is_err());
        }
    }
    #[test]
    fn cache_has_expiry_eviction_and_session_scope_without_cookies() {
        let authority = session::Authority::default();
        let lease = authority.lease(None);
        let now = Instant::now();
        let mut cache = Pools::default();
        let first = pool(&lease, 1000, now);
        let key = first.key.clone();
        cache.insert(first, now);
        assert!(cache
            .get(&key, &session::Lease::default(), false, now)
            .is_err());
        assert!(cache
            .get(
                &key,
                &authority.lease(session::session_header("fixture")),
                false,
                now
            )
            .is_err());
        assert!(cache.get(&key, &lease, true, now).is_err());
        assert!(cache.get(&key, &lease, false, now).is_ok());
        authority.invalidate();
        assert!(cache.get(&key, &authority.lease(None), false, now).is_err());
        assert!(cache.get(&key, &lease, false, now + POOL_TTL).is_err());
        for _ in 0..MAX_POOLS + 1 {
            cache.insert(pool(&lease, 1000, now), now);
        }
        assert_eq!(cache.0.len(), MAX_POOLS);
        assert!(cache.get(&key, &lease, false, now).is_err());
    }
    #[test]
    fn remaining_pool_not_visible_count_controls_continuation() {
        let lease = session::Lease::default();
        let sample = pool(&lease, 125, Instant::now());
        let first = result(&sample, 0, vec![], None);
        assert!(first.has_more);
        assert_eq!(
            cursor_parts(first.home_cursor.as_ref().unwrap(), 2)
                .unwrap()
                .1,
            60
        );
        let last = result(&sample, 120, vec![], None);
        assert!(!last.has_more);
        assert!(last.home_cursor.is_none());
        assert_eq!(last.page, 3);
        assert!(!result(&pool(&lease, 0, Instant::now()), 0, vec![], None).has_more);
    }
    #[test]
    fn stale_lease_and_bad_inputs_fail_without_network() {
        let authority = session::Authority::default();
        let lease = authority.lease(None);
        authority.invalidate();
        assert!(matches!(read(&lease,1,None,false),Err(error) if error==session::STALE));
        assert!(read(&session::Lease::default(), 2, None, false).is_err());
        assert!(read(&session::Lease::default(), 2, Some("unknown"), false).is_err());
        for (query, mode, cursor) in [
            ("landscape", "home", None),
            ("landscape", "latest", Some("forbidden")),
        ] {
            assert!(super::super::search_with_session(
                super::super::PixivSearchRequest {
                    query: query.into(),
                    page: 1,
                    mode: mode.into(),
                    home_cursor: cursor.map(str::to_owned),
                    safe_only: false,
                },
                &session::Lease::default()
            )
            .is_err());
        }
    }
    #[test]
    fn optional_home_fields_do_not_change_other_search_responses() {
        let lease = session::Lease::default();
        let home = serde_json::to_value(result(
            &pool(&lease, 1000, Instant::now()),
            0,
            vec![],
            Some(suggestions(None, false)),
        ))
        .unwrap();
        assert!(home["homeCursor"].is_string());
        assert!(home["homeSuggestions"]["recommendedTags"].is_array());
        let ordinary = serde_json::to_value(PixivSearchPage {
            items: vec![],
            page: 1,
            has_more: false,
            home_suggestions: None,
            home_cursor: None,
        })
        .unwrap();
        assert!(ordinary.get("homeCursor").is_none());
    }
    #[test]
    #[ignore = "Opt-in official public metadata only: no images, credentials or writes"]
    fn live_public_home_metadata_only() {
        let lease = session::Lease::default();
        let first = read(&lease, 1, None, false).unwrap();
        assert!(first.items.len() >= 24);
        assert!(first.has_more);
        let next = read(&lease, 2, first.home_cursor.as_deref(), false).unwrap();
        assert!(!next.items.is_empty());
        assert_eq!(next.page, 2);
        let ids: HashSet<_> = first.items.iter().map(|work| &work.id).collect();
        assert!(next.items.iter().all(|work| !ids.contains(&work.id)));
        println!("Pixiv public recommendation batches: {} + {} eligible works; continuation={}, no images read",first.items.len(),next.items.len(),next.has_more);
    }
}
