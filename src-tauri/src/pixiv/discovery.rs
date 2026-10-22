//! Typed projections of the desktop landing and related-work response graphs.
//! Network schemas are evidence-backed adapters, not a claim of live parity.
use super::{
    allowed_url, detail_with_session, home, json_with_session, session, take_text, text, valid_id,
    work_with_display, DisplayPolicy, PixivWork, SITE,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const SCHEMA: &str = "Pixiv 추천 구성을 읽지 못했습니다. 다시 시도해 주세요.";
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    Home,
    Illustration,
    Manga,
    Related,
}
impl Surface {
    fn includes(self, work: &PixivWork) -> bool {
        match self {
            Self::Illustration => work.kind != "manga",
            Self::Manga => work.kind == "manga",
            _ => true,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub surface: Surface,
    pub work_id: Option<String>,
    pub cursor: Option<String>,
    #[serde(default = "first_page")]
    pub page: u32,
    #[serde(default)]
    pub safe_only: bool,
}
fn first_page() -> u32 {
    1
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    pub title: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub items: Vec<PixivWork>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Creator {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    pub works: Vec<PixivWork>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub sections: Vec<Section>,
    pub creators: Vec<Creator>,
    pub suggestions: Option<home::HomeSuggestions>,
    pub cursor: Option<String>,
    pub page: u32,
    pub issues: Vec<String>,
}
fn body(value: &Value) -> Result<&Value, String> {
    if value["error"].as_bool() != Some(false) || !value["body"].is_object() {
        return Err(SCHEMA.into());
    }
    Ok(&value["body"])
}
fn ids(value: &Value) -> Result<Vec<String>, String> {
    let rows = value.as_array().ok_or(SCHEMA)?;
    if rows.len() > 1000 {
        return Err(SCHEMA.into());
    }
    let mut seen = HashSet::new();
    let result: Vec<_> = rows
        .iter()
        .map(text)
        .filter(|id| valid_id(id) && seen.insert(id.clone()))
        .collect();
    if !rows.is_empty() && result.is_empty() {
        return Err(SCHEMA.into());
    }
    Ok(result)
}
fn works(value: &Value, display: DisplayPolicy, translations: &Value) -> Result<HashMap<String, PixivWork>, String> {
    let rows = value.as_array().ok_or(SCHEMA)?;
    if rows.len() > 1000 {
        return Err(SCHEMA.into());
    }
    Ok(rows
        .iter()
        .filter_map(|row| work_with_display(row, display))
        .map(|mut work| { super::localization::enrich(&mut work, translations); work })
        .map(|work| (work.id.clone(), work))
        .collect())
}
fn project(
    source: &[String],
    table: &HashMap<String, PixivWork>,
    surface: Surface,
    limit: usize,
) -> Vec<PixivWork> {
    source
        .iter()
        .take(limit)
        .filter_map(|id| table.get(id))
        .filter(|work| surface.includes(work))
        .cloned()
        .collect()
}
fn section(
    id: &str,
    title: &str,
    kind: &'static str,
    items: Vec<PixivWork>,
    tag: Option<String>,
) -> Section {
    Section {
        id: id.into(),
        title: title.into(),
        kind,
        tag,
        items,
    }
}
fn creators(
    value: &Value,
    surface: Surface,
    display: DisplayPolicy,
) -> Result<Vec<Creator>, String> {
    let value = body(value)?;
    let rows = value
        .get("recommendUsers")
        .or_else(|| value.get("recommendedUsers"))
        .or_else(|| value.pointer("/page/recommendUser"))
        .and_then(Value::as_array)
        .ok_or(SCHEMA)?;
    let users = value["users"].as_array().ok_or(SCHEMA)?;
    if rows.len() > 100 || users.len() > 1000 {
        return Err(SCHEMA.into());
    }
    let table = works(&value["thumbnails"]["illust"], display, &value["tagTranslation"])?;
    let mut seen = HashSet::new();
    Ok(rows
        .iter()
        .filter_map(|row| {
            let id = text(row.get("userId").unwrap_or(&row["id"]));
            if !valid_id(&id) || !seen.insert(id.clone()) {
                return None;
            }
            let user = users.iter().find(|user| text(&user["userId"]) == id)?;
            if user["isBlocking"].as_bool() == Some(true) {
                return None;
            }
            let work_ids = ids(row.get("illustIds").unwrap_or(&row["recentIllustIds"])).ok()?;
            let name = take_text(&user["name"], 120);
            if name.trim().is_empty() {
                return None;
            }
            let image_url = ["imageBig", "image"]
                .iter()
                .filter_map(|key| user[*key].as_str())
                .find(|url| allowed_url(url, true).is_ok())
                .map(str::to_owned);
            Some(Creator {
                id,
                name,
                image_url,
                works: project(&work_ids, &table, surface, 3),
            })
        })
        .take(12)
        .collect())
}
fn landing(
    value: &Value,
    surface: Surface,
    display: DisplayPolicy,
) -> Result<(Vec<Section>, Vec<String>), String> {
    let body = body(value)?;
    let table = works(&body["thumbnails"]["illust"], display, &body["tagTranslation"])?;
    let page = &body["page"];
    let recommended = ids(&page["recommend"]["ids"])?;
    let followed = ids(&page["follow"])?;
    let mut sections = vec![
        section(
            "followed",
            "팔로우 신작",
            "followed",
            project(&followed, &table, surface, 24),
            None,
        ),
        section(
            "recommended",
            "당신을 위한 추천 작품",
            "recommended",
            project(&recommended, &table, surface, 60),
            None,
        ),
    ];
    let tag_rows = page["recommendByTag"].as_array().ok_or(SCHEMA)?;
    for (index, row) in tag_rows.iter().take(8).enumerate() {
        let tag = take_text(&row["tag"], 120);
        if tag.trim().is_empty() || tag.chars().any(char::is_control) {
            continue;
        }
        let items = project(&ids(&row["ids"])?, &table, surface, 12);
        if !items.is_empty() {
            sections.push(section(
                &format!("tag-{index}"),
                &format!("#{tag}"),
                "tag",
                items,
                Some(tag),
            ));
        }
    }
    if let Ok(newest) = ids(&page["newPost"]) {
        let items = project(&newest, &table, surface, 12);
        if !items.is_empty() {
            sections.push(section("newest", "새로 올라온 작품", "newest", items, None));
        }
    }
    Ok((sections, recommended))
}

pub(super) fn read(request: Request, lease: &session::Lease) -> Result<Response, String> {
    lease.check()?;
    if request.surface == Surface::Related {
        if !request.work_id.as_deref().is_some_and(valid_id) {
            return Err("올바른 작품 번호가 아닙니다.".into());
        }
    } else if request.work_id.is_some() {
        return Err(SCHEMA.into());
    }
    if let Some(cursor) = request.cursor {
        let page = home::read(lease, request.page, Some(&cursor), request.safe_only)?;
        let related = request.surface == Surface::Related;
        return Ok(Response {
            sections: vec![section(
                if related { "related" } else { "recommended" },
                if related {
                    "관련 작품"
                } else {
                    "당신을 위한 추천 작품"
                },
                if related { "related" } else { "recommended" },
                page.items
                    .into_iter()
                    .filter(|work| request.surface.includes(work))
                    .collect(),
                None,
            )],
            creators: vec![],
            suggestions: None,
            cursor: page.home_cursor,
            page: page.page,
            issues: vec![],
        });
    }
    if request.page != 1 {
        return Err(SCHEMA.into());
    }
    let display = DisplayPolicy::new(lease.authenticated(), request.safe_only);
    if request.surface == Surface::Related {
        return related(
            request.work_id.as_deref().unwrap(),
            lease,
            display,
            request.safe_only,
        );
    }
    if !lease.authenticated() {
        let page = home::read(lease, 1, None, request.safe_only)?;
        return Ok(Response {
            sections: vec![section(
                "recommended",
                "추천 작품",
                "recommended",
                page.items
                    .into_iter()
                    .filter(|work| request.surface.includes(work))
                    .collect(),
                None,
            )],
            creators: vec![],
            suggestions: page.home_suggestions,
            cursor: page.home_cursor,
            page: 1,
            issues: vec![],
        });
    }
    // The desktop endpoint returns a single relational artwork graph. Category
    // views project its server-reported illustType; do not invent /top/manga.
    let value = json_with_session(
        &format!(
            "{SITE}/ajax/top/illust?mode={}&lang=ko",
            display.query_mode()
        ),
        lease,
    )?;
    let (mut sections, recommended_ids) = landing(&value, request.surface, display)?;
    let first_ids = &recommended_ids[..recommended_ids.len().min(60)];
    // Sparse manifests can omit work rows. Hydrate the exact server ID set, not
    // an unrelated ranking. The official access filter remains authoritative.
    let present: HashSet<_> = value["body"]["thumbnails"]["illust"]
        .as_array()
        .ok_or(SCHEMA)?
        .iter()
        .map(|row| text(&row["id"]))
        .collect();
    // A filtered/blocked row is present, not missing. Do not re-request it.
    if first_ids.iter().any(|id| !present.contains(id)) {
        sections[1].items = home::hydrate(first_ids, lease, request.safe_only)?
            .into_iter()
            .filter(|work| request.surface.includes(work))
            .collect();
    }
    let page = home::retain(
        lease,
        request.safe_only,
        recommended_ids,
        sections[1].items.clone(),
        None,
    )?;
    let mut issues = vec![];
    let creators = match creators(&value, request.surface, display) {
        Ok(rows) => rows,
        Err(_) => match json_with_session(
            &format!("{SITE}/ajax/discovery/users?limit=12&lang=ko"),
            lease,
        )
        .and_then(|value| creators(&value, request.surface, display))
        {
            Ok(rows) => rows,
            Err(_) => {
                issues.push("추천 작가를 불러오지 못했습니다.".into());
                vec![]
            }
        },
    };
    let suggestion_value = json_with_session(
        &format!("{SITE}/ajax/search/suggestion?mode=all&lang=ko"),
        lease,
    )
    .ok();
    if suggestion_value.is_none() {
        issues.push("검색 태그를 불러오지 못했습니다.".into());
    }
    lease.check()?;
    Ok(Response {
        sections,
        creators,
        suggestions: Some(home::suggestions(suggestion_value.as_ref(), true)),
        cursor: page.home_cursor,
        page: 1,
        issues,
    })
}
fn related(
    id: &str,
    lease: &session::Lease,
    display: DisplayPolicy,
    safe_only: bool,
) -> Result<Response, String> {
    // Derive the creator from freshly authorized metadata, never an arbitrary
    // renderer URL. An unavailable work cannot be used to probe account data.
    let detail = detail_with_session(id, lease)?;
    let value = json_with_session(
        &format!("{SITE}/ajax/illust/{id}/recommend/init?limit=60&lang=ko"),
        lease,
    )?;
    let source = body(&value)?;
    let table = works(&source["illusts"], display, &source["tagTranslation"])?;
    let mut seen = HashSet::new();
    let mut order: Vec<_> = source["illusts"]
        .as_array()
        .ok_or(SCHEMA)?
        .iter()
        .map(|row| text(&row["id"]))
        .filter(|value| valid_id(value) && value != id && seen.insert(value.clone()))
        .collect();
    for next in ids(&source["nextIds"])? {
        if next != id && seen.insert(next.clone()) {
            order.push(next);
        }
    }
    order.truncate(1000);
    // Some responses give fewer than limit initial rows. Fill the first page
    // before issuing an offset60 cursor so no returned IDs are skipped.
    let first = &order[..order.len().min(60)];
    let items = if first.iter().all(|id| table.contains_key(id)) {
        project(first, &table, Surface::Related, 60)
    } else {
        home::hydrate(first, lease, safe_only)?
    };
    let page = home::retain(lease, safe_only, order, items.clone(), None)?;
    let mut issues = vec![];
    let creators = if valid_id(&detail.work.artist_id) {
        let url = format!(
            "{SITE}/ajax/user/{}/recommends?userNum=8&workNum=3&isR18={}&lang=ko",
            detail.work.artist_id,
            lease.authenticated() && !safe_only
        );
        match json_with_session(&url, lease)
            .and_then(|value| creators(&value, Surface::Related, display))
        {
            Ok(rows) => rows,
            Err(_) => {
                issues.push("관련 작가를 불러오지 못했습니다.".into());
                vec![]
            }
        }
    } else {
        vec![]
    };
    lease.check()?;
    Ok(Response {
        sections: vec![section("related", "관련 작품", "related", items, None)],
        creators,
        suggestions: None,
        cursor: page.home_cursor,
        page: 1,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn work(id: u32, kind: u32) -> Value {
        json!({"id":id.to_string(),"illustType":kind,"title":"Fixture","userId":"9","userName":"Artist","url":"https://i.pximg.net/img-master/fixture.jpg","pageCount":1,"xRestrict":0,"sl":2})
    }
    fn fixture() -> Value {
        json!({"error":false,"body":{"page":{"follow":[2,1],"recommend":{"ids":["1","2","1"]},"recommendByTag":[{"tag":"sky","ids":["2","1"]}],"recommendUser":[{"id":9,"illustIds":["1","2"]}]},"users":[{"userId":"9","name":"Artist","imageBig":"https://i.pximg.net/user-profile/fixture.jpg"}],"thumbnails":{"illust":[work(1,0),work(2,1)]}}})
    }
    #[test]
    fn ordered_sections_and_categories_use_server_types() {
        let (home, ids) =
            landing(&fixture(), Surface::Home, DisplayPolicy::new(true, false)).unwrap();
        assert_eq!(ids, ["1", "2"]);
        assert_eq!(
            home[0]
                .items
                .iter()
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            ["2", "1"]
        );
        let (manga, _) =
            landing(&fixture(), Surface::Manga, DisplayPolicy::new(true, false)).unwrap();
        assert_eq!(manga[1].items.len(), 1);
        assert_eq!(manga[1].items[0].id, "2");
        assert_eq!(manga[2].tag.as_deref(), Some("sky"));
    }
    #[test]
    fn creators_join_server_order_and_validate_images() {
        let mut value = fixture();
        let users = creators(
            &value,
            Surface::Illustration,
            DisplayPolicy::new(true, false),
        )
        .unwrap();
        assert_eq!(users[0].works.len(), 1);
        assert_eq!(users[0].id, "9");
        assert!(users[0].image_url.is_some());
        value["body"]["users"][0]["imageBig"] = json!("https://evil.test/private");
        assert!(
            creators(&value, Surface::Home, DisplayPolicy::new(true, false)).unwrap()[0]
                .image_url
                .is_none()
        );
        value["body"]["users"][0]["isBlocking"] = json!(true);
        assert!(
            creators(&value, Surface::Home, DisplayPolicy::new(true, false))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn malformed_and_restricted_data_are_not_accepted_as_a_valid_empty_feed() {
        assert!(landing(
            &json!({"error":false,"body":{}}),
            Surface::Home,
            DisplayPolicy::new(true, false)
        )
        .is_err());
        assert!(ids(&json!(vec!["1"; 1001])).is_err());
        let mut value = fixture();
        value["body"]["thumbnails"]["illust"][0]["isMasked"] = json!(true);
        let (sections, _) =
            landing(&value, Surface::Home, DisplayPolicy::new(true, false)).unwrap();
        assert!(sections
            .iter()
            .all(|section| section.items.iter().all(|work| work.id != "1")));
    }
    #[test]
    fn stale_session_and_invalid_requests_stop_before_network() {
        let authority = session::Authority::default();
        let lease = authority.lease(None);
        authority.invalidate();
        let request = || Request {
            surface: Surface::Related,
            work_id: Some("123".into()),
            cursor: None,
            page: 1,
            safe_only: false,
        };
        assert_eq!(read(request(), &lease).unwrap_err(), session::STALE);
        let mut invalid = request();
        invalid.work_id = Some("https://evil.test".into());
        assert!(read(invalid, &session::Lease::default()).is_err());
    }
}
