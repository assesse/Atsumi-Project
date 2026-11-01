//! URL entities actually published in the exact source post and its connected
//! same-author reply chain. This is provenance, never reverse-image authorship.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tauri::Url;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PublishedLink {
    pub url: String,
    pub post_url: String,
    pub reply: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Projection {
    pub links: Vec<PublishedLink>,
    pub limited: bool,
}
#[derive(Clone, Debug)]
struct Post {
    id: String,
    author: String,
    handle: String,
    parent: Option<String>,
    reply_author: Option<String>,
    urls: Vec<String>,
    limited: bool,
}
fn id(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && !value.starts_with('0')
        && value.bytes().all(|c| c.is_ascii_digit())
}
fn handle(value: &str) -> bool {
    (1..=15).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
pub(super) fn source(value: &Url) -> Option<(String, String)> {
    if value.scheme() != "https"
        || value.host_str() != Some("x.com")
        || value.port().is_some()
        || value.query().is_some()
        || value.fragment().is_some()
        || !value.username().is_empty()
        || value.password().is_some()
    {
        return None;
    }
    let parts = value
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [who, "status", post] if handle(who) && id(post) => {
            Some((who.to_ascii_lowercase(), (*post).into()))
        }
        _ => None,
    }
}
pub(super) fn detail_response(url: &Url) -> bool {
    if url.scheme() != "https"
        || url.host_str() != Some("x.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let parts = url
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    matches!(parts.as_slice(),["i","api","graphql",query,"TweetDetail"]if (8..=128).contains(&query.len())&&query.bytes().all(|c|c.is_ascii_alphanumeric()||b"_-".contains(&c)))
}
fn external(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.host_str()?.contains('.')
        || matches!(
            url.host_str()?,
            "x.com" | "www.x.com" | "twitter.com" | "www.twitter.com"
        )
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}
fn post(value: &Value) -> Option<Post> {
    let value = if value.get("__typename")?.as_str() == Some("TweetWithVisibilityResults") {
        value.get("tweet")?
    } else {
        value
    };
    if value.get("__typename")?.as_str() != Some("Tweet") {
        return None;
    }
    let legacy = value.get("legacy")?;
    let key = value.get("rest_id")?.as_str()?;
    if !id(key) {
        return None;
    }
    if legacy
        .get("id_str")
        .and_then(Value::as_str)
        .is_some_and(|v| v != key)
    {
        return None;
    }
    let user = value.pointer("/core/user_results/result")?;
    if user
        .get("__typename")
        .and_then(Value::as_str)
        .is_some_and(|v| v != "User")
    {
        return None;
    }
    let author = user.get("rest_id")?.as_str()?;
    let name = user
        .pointer("/core/screen_name")
        .or_else(|| user.pointer("/legacy/screen_name"))?
        .as_str()?;
    if !id(author)
        || !handle(name)
        || legacy
            .get("user_id_str")
            .and_then(Value::as_str)
            .is_some_and(|v| v != author)
    {
        return None;
    }
    let parent = match legacy.get("in_reply_to_status_id_str") {
        Some(Value::Null) | None => None,
        Some(Value::String(v)) if id(v) => Some(v.clone()),
        _ => return None,
    };
    let reply_author = match legacy.get("in_reply_to_user_id_str") {
        Some(Value::Null) | None => None,
        Some(Value::String(v)) if id(v) => Some(v.clone()),
        _ => return None,
    };
    let mut urls = Vec::new();
    let mut limited = false;
    for entities in [
        legacy.pointer("/entities/urls"),
        value.pointer("/note_tweet/note_tweet_results/result/entity_set/urls"),
    ]
    .into_iter()
    .flatten()
    {
        let Some(items) = entities.as_array() else {
            continue;
        };
        limited |= items.len() > 24;
        for entity in items.iter().take(24) {
            if let Some(url) = entity
                .get("expanded_url")
                .and_then(Value::as_str)
                .and_then(external)
            {
                if !urls.contains(&url) {
                    if urls.len() < 8 {
                        urls.push(url);
                    } else {
                        limited = true;
                    }
                }
            }
        }
    }
    Some(Post {
        id: key.into(),
        author: author.into(),
        handle: name.to_ascii_lowercase(),
        parent,
        reply_author,
        urls,
        limited,
    })
}
pub(super) fn parse(bytes: &[u8], expected: &Url) -> Result<Projection, String> {
    let (who, root) = source(expected).ok_or("source")?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("size".into());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "json")?;
    let instructions = value
        .pointer("/data/threaded_conversation_with_injections_v2/instructions")
        .and_then(Value::as_array)
        .ok_or("structure")?;
    if instructions.len() > 32 {
        return Err("instructions".into());
    }
    let mut posts = HashMap::new();
    let mut ambiguous = HashSet::new();
    let mut entries = 0;
    let mut limited = false;
    let mut accept = |item: &Value| {
        let Some(result) = item.pointer("/tweet_results/result").and_then(post) else {
            return;
        };
        if let Some(old) = posts.get(&result.id) {
            let old: &Post = old;
            if old.author != result.author
                || old.parent != result.parent
                || old.reply_author != result.reply_author
                || old.handle != result.handle
                || old.urls != result.urls
            {
                ambiguous.insert(result.id.clone());
            }
        }
        posts.insert(result.id.clone(), result);
    };
    for instruction in instructions {
        // Only actual timeline entries/modules. Never recursively scan quoted
        // tweets, cards, recommendations or arbitrary objects for URL strings.
        let list = instruction
            .get("entries")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if list.len() > 256 {
            return Err("entries".into());
        }
        limited |= instruction.get("moduleItems").is_some();
        for entry in list.iter().take(256) {
            entries += 1;
            if entries > 256 {
                return Err("entries".into());
            }
            let Some(content) = entry.get("content") else {
                continue;
            };
            limited |= content.get("cursorType").and_then(Value::as_str) == Some("Bottom");
            if let Some(item) = content.get("itemContent") {
                accept(item);
            }
            if let Some(items) = content.get("items").and_then(Value::as_array) {
                if items.len() > 64 {
                    return Err("module".into());
                }
                for item in items {
                    entries += 1;
                    if entries > 256 {
                        return Err("entries".into());
                    }
                    if let Some(item) = item.pointer("/item/itemContent") {
                        accept(item);
                    }
                }
            }
        }
    }
    let original = posts
        .get(&root)
        .filter(|p| p.handle == who && !ambiguous.contains(&root))
        .ok_or("root")?;
    let author = &original.author;
    let mut connected = HashSet::from([root.clone()]);
    for _ in 0..16 {
        let before = connected.clone();
        for value in posts.values() {
            if value.author == *author
                && value.handle == who
                && !ambiguous.contains(&value.id)
                && value.reply_author.as_ref().is_none_or(|v| v == author)
            {
                if value.parent.as_ref().is_some_and(|v| before.contains(v)) {
                    connected.insert(value.id.clone());
                }
                if before.contains(&value.id) {
                    if let Some(parent) =
                        value
                            .parent
                            .as_ref()
                            .and_then(|id| posts.get(id))
                            .filter(|p| {
                                p.author == *author && p.handle == who && !ambiguous.contains(&p.id)
                            })
                    {
                        connected.insert(parent.id.clone());
                    }
                }
            }
        }
        if connected.len() == before.len() {
            break;
        }
    }
    limited |= posts.values().any(|post| {
        post.author == *author
            && post.handle == who
            && !ambiguous.contains(&post.id)
            && post.reply_author.as_ref().is_none_or(|v| v == author)
            && (post
                .parent
                .as_ref()
                .is_some_and(|id| connected.contains(id) && !connected.contains(&post.id))
                || connected.contains(&post.id)
                    && post.parent.as_ref().is_some_and(|id| {
                        !connected.contains(id)
                            && posts.get(id).is_some_and(|parent| {
                                parent.author == *author
                                    && parent.handle == who
                                    && !ambiguous.contains(id)
                            })
                    }))
    });
    let mut ordered = posts
        .values()
        .filter(|p| connected.contains(&p.id))
        .collect::<Vec<_>>();
    ordered.sort_by(|a, b| (a.id != root).cmp(&(b.id != root)).then(a.id.cmp(&b.id)));
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    for post in ordered {
        limited |= post.limited;
        for url in &post.urls {
            if seen.insert(url.clone()) {
                if links.len() >= 8 {
                    limited = true;
                    continue;
                }
                links.push(PublishedLink {
                    url: url.clone(),
                    post_url: format!("https://x.com/{who}/status/{}", post.id),
                    reply: post.id != root,
                });
            }
        }
    }
    Ok(Projection { links, limited })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn item(key: &str, author: &str, who: &str, parent: Option<&str>, url: &str) -> Value {
        json!({"tweet_results":{"result":{"__typename":"Tweet","rest_id":key,"core":{"user_results":{"result":{"__typename":"User","rest_id":author,"core":{"screen_name":who}}}},"legacy":{"in_reply_to_status_id_str":parent,"entities":{"urls":[{"expanded_url":url}]}}}}})
    }
    fn response(items: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({"data":{"threaded_conversation_with_injections_v2":{"instructions":[{"entries":items.into_iter().map(|v|json!({"content":{"itemContent":v}})).collect::<Vec<_>>()}]}}})).unwrap()
    }
    #[test]
    fn exact_root_and_connected_same_author_links_keep_reply_provenance() {
        let data = response(vec![
            item(
                "100001",
                "55",
                "author",
                None,
                "https://example.com/original",
            ),
            item(
                "100002",
                "55",
                "author",
                Some("100001"),
                "https://shop.example.com/item",
            ),
            item(
                "100003",
                "55",
                "author",
                Some("100002"),
                "https://other.example.com/item",
            ),
        ]);
        let links = parse(
            &data,
            &"https://x.com/author/status/100001".parse().unwrap(),
        )
        .unwrap()
        .links;
        assert_eq!(links.len(), 3);
        assert!(!links[0].reply);
        assert!(links[1].reply);
        assert_eq!(links[1].post_url, "https://x.com/author/status/100002");
    }
    #[test]
    fn other_authors_disconnected_posts_quotes_and_conflicting_roots_are_not_sources() {
        let mut root = item(
            "100001",
            "55",
            "author",
            None,
            "https://example.com/original",
        );
        root["tweet_results"]["result"]["quoted_status_result"] = item(
            "200001",
            "77",
            "other",
            None,
            "https://unrelated.example.com/quote",
        );
        let data = response(vec![
            root.clone(),
            item(
                "100002",
                "77",
                "other",
                Some("100001"),
                "https://unrelated.example.com/reply",
            ),
            item(
                "100003",
                "55",
                "author",
                Some("200001"),
                "https://unrelated.example.com/disconnected",
            ),
        ]);
        let target = "https://x.com/author/status/100001".parse().unwrap();
        let links = parse(&data, &target).unwrap().links;
        assert_eq!(links.len(), 1);
        assert!(parse(
            &response(vec![
                root,
                item("100001", "77", "other", None, "https://bad.example.com/")
            ]),
            &target
        )
        .is_err());
        assert!(parse(&data, &"https://x.com/wrong/status/100001".parse().unwrap()).is_err());
    }
    #[test]
    fn bounded_protocol_does_not_admit_other_graphql_or_arbitrary_source_paths() {
        assert!(detail_response(
            &"https://x.com/i/api/graphql/abcdefgh123/TweetDetail?variables=opaque"
                .parse()
                .unwrap()
        ));
        for raw in [
            "https://x.com/i/api/graphql/abcdefgh123/CreateTweet",
            "https://x.com.evil.test/i/api/graphql/abcdefgh123/TweetDetail",
            "http://x.com/i/api/graphql/abcdefgh123/TweetDetail",
        ] {
            assert!(!detail_response(&raw.parse().unwrap()));
        }
        let target = "https://x.com/author/status/100001".parse().unwrap();
        assert!(parse(&vec![0; 2 * 1024 * 1024 + 1], &target).is_err());
        assert!(parse(b"{}", &target).is_err());
        assert!(parse(
            &response(vec![item(
                "100002",
                "55",
                "author",
                None,
                "https://example.com/"
            )]),
            &target
        )
        .is_err());
    }
    #[test]
    fn source_reply_can_find_its_same_author_parent_but_not_a_foreign_parent() {
        let target = "https://x.com/author/status/100002".parse().unwrap();
        let own = response(vec![
            item(
                "100001",
                "55",
                "author",
                None,
                "https://shop.example.com/original",
            ),
            item(
                "100002",
                "55",
                "author",
                Some("100001"),
                "https://example.com/source",
            ),
        ]);
        let own = parse(&own, &target).unwrap();
        assert_eq!(own.links.len(), 2);
        assert_eq!(own.links[1].post_url, "https://x.com/author/status/100001");
        let foreign = response(vec![
            item(
                "100001",
                "77",
                "other",
                None,
                "https://unrelated.example.com/",
            ),
            item(
                "100002",
                "55",
                "author",
                Some("100001"),
                "https://example.com/source",
            ),
        ]);
        assert_eq!(parse(&foreign, &target).unwrap().links.len(), 1);
    }

    #[test]
    fn bounded_thread_depth_remains_partial_instead_of_silently_complete() {
        let items = (100001..=100018)
            .map(|key| {
                let parent = (key > 100001).then(|| (key - 1).to_string());
                item(
                    &key.to_string(),
                    "55",
                    "author",
                    parent.as_deref(),
                    "https://shop.example.com/one-item",
                )
            })
            .collect();
        let target = "https://x.com/author/status/100001".parse().unwrap();
        let partial = parse(&response(items), &target).unwrap();
        assert!(partial.limited);
        assert_eq!(partial.links.len(), 1);
        let complete = parse(
            &response(vec![item(
                "100001",
                "55",
                "author",
                None,
                "https://shop.example.com/one-item",
            )]),
            &target,
        )
        .unwrap();
        assert!(!complete.limited);
    }
}
