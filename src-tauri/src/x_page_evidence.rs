//! Anonymous, bounded evidence from the exact user-selected result page.
//! Metadata is a publisher's declaration, not an inferred original identity.
use dom_query::Document;
use serde::Serialize;
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::HashSet,
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tauri::Url;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_HTML: usize = 4 * MAX_BYTES;
const UNAVAILABLE: &str = "페이지 정보를 읽지 못했습니다. 원본 페이지에서 확인해 주세요.";
const CANCELLED: &str = "검색 대상이 바뀌어 페이지 정보 확인을 중단했습니다.";
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static DNS_ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageEvidence {
    pub requested_url: String,
    pub url: String,
    pub observed_at: String,
    pub fields: Vec<Field>,
    pub limited: bool,
    pub ambiguous: bool,
    #[serde(skip)]
    pub(super) catalog: super::catalog_parse::CatalogPage,
}
impl PageEvidence {
    pub(super) fn empty(url: &str) -> Self {
        Self {
            requested_url: url.into(),
            url: url.into(),
            observed_at: "1970-01-01T00:00:00Z".into(),
            fields: Vec::new(),
            limited: false,
            ambiguous: false,
            catalog: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Field {
    pub label: String,
    pub value: String,
    pub source: String,
}
pub(super) fn busy() -> bool {
    ACTIVE.load(Ordering::Acquire) != 0
}
struct Permit(&'static AtomicUsize);
impl Permit {
    fn acquire(counter: &'static AtomicUsize, max: usize) -> Result<Self, String> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < max).then_some(n + 1)
            })
            .map(|_| Self(counter))
            .map_err(|_| UNAVAILABLE.into())
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || a == 100 && (64..=127).contains(&b)
                || a == 192 && b == 0 && c == 0
                || a == 198 && matches!(b, 18 | 19))
        }
        IpAddr::V6(ip) => {
            let p = ip.segments();
            (p[0] & 0xe000) == 0x2000
                && p[0] != 0x2002
                && !(p[0] == 0x2001 && p[1] < 0x0200)
                && !(p[0] == 0x2001 && p[1] == 0x0db8)
                && !(p[0] == 0x3fff && p[1] < 0x1000)
        }
    }
}
pub(super) fn page_url(value: &str) -> Option<Url> {
    if value.is_empty()
        || value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return None;
    }
    let mut url = Url::parse(value).ok()?;
    let host = url.host_str()?;
    if !matches!(url.scheme(), "https" | "http")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !host.contains('.')
        || host.ends_with('.')
        || host.ends_with(".local")
        || host.ends_with(".localhost")
        || host.ends_with(".internal")
        || host.parse::<IpAddr>().is_ok_and(|ip| !public_ip(ip))
    {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}
fn addresses(url: &Url, stop: Instant) -> Result<Vec<SocketAddr>, String> {
    let permit = Permit::acquire(&DNS_ACTIVE, 4)?;
    let host = url.host_str().ok_or(UNAVAILABLE)?.to_owned();
    let port = url.port_or_known_default().ok_or(UNAVAILABLE)?;
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("source-page-dns".into())
        .spawn(move || {
            // Retain the permit until uncancellable OS DNS exits, even on timeout.
            let _permit = permit;
            let result = (host.as_str(), port)
                .to_socket_addrs()
                .map(|ips| ips.take(16).collect::<Vec<_>>());
            let _ = tx.send(result);
        })
        .map_err(|_| UNAVAILABLE)?;
    let found = rx
        .recv_timeout(
            stop.saturating_duration_since(Instant::now())
                .min(Duration::from_secs(1)),
        )
        .map_err(|_| UNAVAILABLE)?
        .map_err(|_| UNAVAILABLE)?;
    if found.is_empty() || !found.iter().all(|ip| public_ip(ip.ip())) {
        return Err(UNAVAILABLE.into());
    }
    Ok(found)
}
fn alive(cancel: &AtomicBool, stop: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELLED.into())
    } else if Instant::now() >= stop {
        Err(UNAVAILABLE.into())
    } else {
        Ok(())
    }
}
fn read_body(reader: impl Read, length: Option<u64>) -> Result<Vec<u8>, String> {
    if length.is_some_and(|n| n > MAX_BYTES as u64) {
        return Err(UNAVAILABLE.into());
    }
    let mut bytes = Vec::new();
    reader
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| UNAVAILABLE)?;
    if bytes.len() > MAX_BYTES {
        return Err(UNAVAILABLE.into());
    }
    Ok(bytes)
}
pub(super) fn read_result(value: &str, cancel: &AtomicBool) -> Result<PageEvidence, String> {
    read_result_until(value, cancel, Instant::now() + Duration::from_secs(14))
}
pub(super) fn read_result_until(
    value: &str,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<PageEvidence, String> {
    let _permit = Permit::acquire(&ACTIVE, 2)?;
    alive(cancel, deadline)?;
    let target = super::provider::resolve_result_url_until(
        value,
        &|| !cancel.load(Ordering::Acquire) && Instant::now() < deadline,
        deadline,
    )?;
    let mut evidence = fetch_until(&target, cancel, deadline)?;
    evidence.requested_url = value.into();
    Ok(evidence)
}
fn fetch(value: &str, cancel: &AtomicBool) -> Result<PageEvidence, String> {
    fetch_until(value, cancel, Instant::now() + Duration::from_secs(6))
}
fn fetch_until(
    value: &str,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<PageEvidence, String> {
    let requested = value.to_owned();
    let mut url = page_url(value).ok_or(UNAVAILABLE)?;
    let stop = deadline.min(Instant::now() + Duration::from_secs(6));
    let mut visited = HashSet::new();
    for hop in 0..=3 {
        alive(cancel, stop)?;
        if !visited.insert(url.to_string()) {
            return Err(UNAVAILABLE.into());
        }
        let pinned = addresses(&url, stop)?;
        alive(cancel, stop)?;
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .no_proxy()
            .resolve_to_addrs(url.host_str().ok_or(UNAVAILABLE)?, &pinned)
            .connect_timeout(Duration::from_secs(1))
            .timeout(stop.saturating_duration_since(Instant::now()))
            .user_agent(concat!("Atsumi/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| UNAVAILABLE)?;
        let response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
            .send()
            .map_err(|_| UNAVAILABLE)?;
        alive(cancel, stop)?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if hop == 3 {
                return Err(UNAVAILABLE.into());
            }
            let headers = response.headers().get_all(reqwest::header::LOCATION);
            if headers.iter().count() != 1 {
                return Err(UNAVAILABLE.into());
            }
            let next = headers
                .iter()
                .next()
                .and_then(|v| v.to_str().ok())
                .and_then(|v| url.join(v).ok())
                .and_then(|v| page_url(v.as_str()))
                .ok_or(UNAVAILABLE)?;
            if url.scheme() == "https" && next.scheme() != "https" {
                return Err(UNAVAILABLE.into());
            }
            url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(UNAVAILABLE.into());
        }
        let kind = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime = kind.split(';').next().unwrap_or("").trim();
        if !matches!(mime, "text/html" | "application/xhtml+xml") {
            return Err(UNAVAILABLE.into());
        }
        let length = response.content_length();
        let bytes = read_body(response, length)?;
        alive(cancel, stop)?;
        let html = decode_html(&bytes, &kind)?;
        let output = parse(&requested, url.as_str(), &html)?;
        alive(cancel, stop)?;
        return Ok(output);
    }
    Err(UNAVAILABLE.into())
}
fn decode_html<'a>(bytes: &'a [u8], content_type: &str) -> Result<Cow<'a, str>, String> {
    use encoding_rs::{Encoding, UTF_8};
    let (encoding, offset) = if let Some((encoding, offset)) = Encoding::for_bom(bytes) {
        (encoding, offset)
    } else {
        let header = content_type.split(';').skip(1).find_map(|part| {
            part.trim()
                .split_once('=')
                .filter(|(k, _)| k.trim().eq_ignore_ascii_case("charset"))
                .map(|(_, v)| v.trim().trim_matches(['\"', '\'']).to_owned())
        });
        let label = if header.is_some() {
            header
        } else {
            let prefix = bytes
                .iter()
                .take(1024)
                .map(|v| if v.is_ascii() { *v as char } else { '?' })
                .collect::<String>();
            let doc = Document::from(prefix.as_str());
            let meta = doc.select("meta[charset]");
            if meta.length() == 1 {
                meta.attr("charset").map(|v| v.to_string())
            } else {
                doc.select("meta[http-equiv]").iter().find_map(|node| {
                    if !node
                        .attr("http-equiv")
                        .is_some_and(|v| v.eq_ignore_ascii_case("content-type"))
                    {
                        return None;
                    }
                    let value = node.attr("content")?;
                    value.split(';').find_map(|v| {
                        v.trim()
                            .split_once('=')
                            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("charset"))
                            .map(|(_, v)| v.trim().trim_matches(['\"', '\'']).to_owned())
                    })
                })
            }
        };
        (
            match label {
                Some(v) => Encoding::for_label(v.as_bytes()).ok_or(UNAVAILABLE)?,
                None => UTF_8,
            },
            0,
        )
    };
    let text = encoding
        .decode_without_bom_handling_and_without_replacement(&bytes[offset..])
        .ok_or(UNAVAILABLE)?;
    if text.len() > MAX_HTML {
        return Err(UNAVAILABLE.into());
    }
    Ok(text)
}
pub(super) fn tidy(value: &str, max: usize) -> (String, bool) {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = String::new();
    let mut length = 0;
    for c in text.chars() {
        if c.is_control() {
            continue;
        }
        if length + c.len_utf16() > max {
            return (output, true);
        }
        output.push(c);
        length += c.len_utf16();
    }
    (output, false)
}
fn add(result: &mut PageEvidence, label: &str, value: &str, source: &str) {
    let (value, limited) = tidy(value, 2048);
    result.limited |= limited;
    if value.is_empty()
        || result
            .fields
            .iter()
            .any(|f| f.label == label && f.value == value && f.source == source)
    {
        return;
    }
    if result.fields.len() >= 24 {
        result.limited = true;
        return;
    }
    result.fields.push(Field {
        label: label.into(),
        value,
        source: source.into(),
    });
}
pub(super) fn identity(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value.get("@id")?.as_str())
}
fn named(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value.get("name")?.as_str())
}
fn identifier_value(output: &mut PageEvidence, value: &Value, path: &str) {
    if let Some(text) = value.as_str() {
        add(output, "식별자", text, path);
        return;
    }
    let known = |kind: &str| {
        matches!(
            kind,
            "PropertyValue"
                | "https://schema.org/PropertyValue"
                | "http://schema.org/PropertyValue"
        )
    };
    let property = value.get("@type").is_some_and(|kind| {
        kind.as_str().is_some_and(known)
            || kind
                .as_array()
                .is_some_and(|types| types.iter().filter_map(Value::as_str).any(known))
    });
    if !property {
        return;
    }
    let text = match value.get("value") {
        Some(Value::String(text)) => Cow::Borrowed(text.as_str()),
        Some(Value::Number(number)) => Cow::Owned(number.to_string()),
        _ => return,
    };
    // PropertyValue.name names the property, never its identifier value.
    let kind = value
        .get("propertyID")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .or_else(|| value.get("name").and_then(Value::as_str));
    let (kind, limited) = kind.map(|text| tidy(text, 64)).unwrap_or_default();
    output.limited |= limited;
    let label = if kind.is_empty() {
        "식별자".into()
    } else {
        format!("식별자 · {kind}")
    };
    add(output, &label, &text, &format!("{path}.value"));
}
pub(super) fn identifiers(output: &mut PageEvidence, value: &Value, path: &str) {
    if let Some(values) = value.as_array() {
        output.limited |= values.len() > 24;
        for (index, value) in values.iter().take(24).enumerate() {
            identifier_value(output, value, &format!("{path}[{index}]"));
        }
    } else {
        identifier_value(output, value, path);
    }
}
pub(super) fn structured_items<'a>(
    value: &'a Value,
    path: String,
    depth: usize,
    items: &mut Vec<(String, &'a Value)>,
) {
    if depth > 4 || items.len() >= 32 {
        return;
    }
    if let Some(list) = value.as_array() {
        for (i, item) in list.iter().take(32).enumerate() {
            structured_items(item, format!("{path}[{i}]"), depth + 1, items);
        }
        return;
    }
    if !value.is_object() {
        return;
    }
    // Follow explicit main entities and graph roots, never reviews, ItemLists,
    // recommendations or arbitrary nested Product objects.
    if let Some(main) = value.get("mainEntity") {
        structured_items(main, format!("{path}.mainEntity"), depth + 1, items);
    }
    if let Some(graph) = value.get("@graph") {
        structured_items(graph, format!("{path}.@graph"), depth + 1, items);
    }
    let accepted = |v: &str| {
        matches!(
            v,
            "Product"
                | "VideoObject"
                | "ImageObject"
                | "Article"
                | "NewsArticle"
                | "CreativeWork"
                | "Book"
                | "Movie"
                | "MusicRecording"
                | "MusicAlbum"
        )
    };
    if items.len() < 32
        && value.get("@type").is_some_and(|kind| {
            kind.as_str().is_some_and(accepted)
                || kind
                    .as_array()
                    .is_some_and(|a| a.iter().filter_map(Value::as_str).any(accepted))
        })
    {
        items.push((path, value));
    }
}
fn parse(requested: &str, url: &str, html: &str) -> Result<PageEvidence, String> {
    if html.len() > MAX_HTML {
        return Err(UNAVAILABLE.into());
    }
    let document = Document::from(html);
    if document.select("iframe[src*=captcha],iframe[src*=challenges],input[name=cf-turnstile-response],#challenge-running").exists() { return Err(UNAVAILABLE.into()); }
    let title = document.select("head title").text().to_string();
    if ["just a moment", "attention required", "access denied"]
        .iter()
        .any(|v| title.to_ascii_lowercase().starts_with(v))
    {
        return Err(UNAVAILABLE.into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| UNAVAILABLE)?;
    let observed = chrono::DateTime::<chrono::Utc>::from_timestamp(
        i64::try_from(now.as_secs()).map_err(|_| UNAVAILABLE)?,
        now.subsec_nanos(),
    )
    .ok_or(UNAVAILABLE)?;
    let mut output = PageEvidence {
        requested_url: requested.into(),
        url: url.into(),
        observed_at: observed.to_rfc3339(),
        fields: Vec::new(),
        limited: false,
        ambiguous: false,
        catalog: super::catalog_parse::parse(url, html),
    };
    for (i, node) in document.select("head title").iter().take(4).enumerate() {
        add(
            &mut output,
            "문서 제목",
            &node.text(),
            &format!("title[{i}]"),
        );
    }
    for (i, node) in document.select("head meta").iter().take(192).enumerate() {
        let key = node
            .attr("property")
            .or_else(|| node.attr("name"))
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_default();
        let label = match key.as_str() {
            "og:title" => "페이지 제목",
            "twitter:title" => "공유 제목",
            "description" | "og:description" | "twitter:description" => "설명",
            "author" | "article:author" => "작성자",
            "article:published_time" => "게시일",
            "product:retailer_item_id" => "판매처 등록 ID",
            _ => continue,
        };
        if let Some(value) = node.attr("content") {
            add(&mut output, label, &value, &format!("meta[{i}]:{key}"));
        }
    }
    let current = Url::parse(url).map_err(|_| UNAVAILABLE)?;
    let mut identities = vec![current.clone()];
    let canonical = document.select("head link[rel=canonical]");
    if canonical.length() == 1 {
        if let Some(target) = canonical
            .attr("href")
            .and_then(|v| current.join(&v).ok())
            .filter(|v| v.origin() == current.origin())
        {
            identities.push(target);
        }
    }
    let scripts = document.select("script[type='application/ld+json']");
    output.limited |= scripts.length() > 8;
    let mut json = Vec::new();
    for node in scripts.iter().take(8) {
        let text = node.text();
        if text.len() > 128 * 1024 {
            output.limited = true;
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            json.push(value);
        }
    }
    let mut items = Vec::new();
    for (i, value) in json.iter().enumerate() {
        structured_items(value, format!("json-ld[{i}]"), 0, &mut items);
    }
    output.limited |= items.len() >= 32;
    let matching = |value: &Value| {
        [
            value.get("url"),
            value.get("@id"),
            value.get("mainEntityOfPage"),
        ]
        .into_iter()
        .flatten()
        .filter_map(identity)
        .any(|id| {
            current.join(id).ok().is_some_and(|mut id| {
                id.set_fragment(None);
                identities.iter().any(|v| v.as_str() == id.as_str())
            })
        })
    };
    let explicit = items
        .iter()
        .filter(|(_, v)| matching(v))
        .collect::<Vec<_>>();
    let selected = if !explicit.is_empty() {
        explicit
    } else if items.len() == 1
        && ["url", "@id", "mainEntityOfPage"]
            .iter()
            .all(|key| items[0].1.get(key).is_none())
    {
        items.iter().collect()
    } else {
        output.ambiguous = items.len() > 1;
        Vec::new()
    };
    for (path, value) in selected {
        for (key, label) in [
            ("name", "이름"),
            ("headline", "제목"),
            ("description", "설명"),
            ("sku", "상품 번호"),
            ("productID", "상품 ID"),
            ("brand", "브랜드"),
            ("author", "작성자"),
            ("creator", "제작자"),
            ("datePublished", "발행일"),
            ("dateCreated", "제작일"),
        ] {
            if let Some(text) = value.get(key).and_then(named) {
                add(&mut output, label, text, &format!("{path}.{key}"));
            }
        }
        if let Some(identifier) = value.get("identifier") {
            identifiers(&mut output, identifier, &format!("{path}.identifier"));
        }
    }
    // A semantic body excerpt remains quoted page text, never an inferred
    // creator/product field. Static HTML cannot establish computed visibility.
    document.select("script,style,template,noscript,svg,nav,aside,header,footer,[hidden],[inert],[aria-hidden='true'],[style*='display:none'],[style*='display: none'],[style*='visibility:hidden'],[style*='visibility: hidden']").remove();
    let main = document.select("main,[role='main']");
    if main.length() == 1 {
        add(
            &mut output,
            "본문 일부",
            &main.formatted_text(),
            "main:text",
        );
    } else if main.length() == 0 {
        let article = document.select("article");
        if article.length() == 1 {
            add(
                &mut output,
                "본문 일부",
                &article.formatted_text(),
                "article:text",
            );
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifier_property_names_are_not_identifier_values() {
        let info=parse("https://example.org/p","https://example.org/p",r#"<script type="application/ld+json">{"@type":"Book","identifier":[{"@type":"PropertyValue","name":"Catalogue number","propertyID":"CAT","value":"001-042"},{"@type":["Thing","https://schema.org/PropertyValue"],"name":"Accession","value":0},"https://example.org/explicit-id"]}</script>"#).unwrap();
        let ids = info
            .fields
            .iter()
            .map(|f| (f.label.as_str(), f.value.as_str(), f.source.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                ("식별자 · CAT", "001-042", "json-ld[0].identifier[0].value"),
                ("식별자 · Accession", "0", "json-ld[0].identifier[1].value"),
                (
                    "식별자",
                    "https://example.org/explicit-id",
                    "json-ld[0].identifier[2]"
                )
            ]
        );
        assert!(!info.limited);
    }
    #[test]
    fn identifier_extraction_does_not_promote_untyped_names_or_other_page_entities() {
        for identifier in [
            serde_json::json!({"name":"NOT_A_CODE"}),
            serde_json::json!({"name":"NOT_A_CODE","value":"UNTYPED"}),
            serde_json::json!({"@type":"Other","name":"NOT_A_CODE","value":"OTHER"}),
            serde_json::json!({"@type":"https://evil.test/PropertyValue","value":"FOREIGN"}),
            serde_json::json!({"@type":"PropertyValue","name":"NOT_A_CODE","@id":"https://example.org/not-a-value"}),
            serde_json::json!({"@type":"PropertyValue","value":{"name":"NOT_A_CODE"}}),
        ] {
            let source = serde_json::json!({"@type":"Product","identifier":identifier});
            let info = parse(
                "https://example.org/p",
                "https://example.org/p",
                &format!("<script type='application/ld+json'>{source}</script>"),
            )
            .unwrap();
            assert!(info.fields.is_empty());
        }
        let unrelated=parse("https://example.org/p","https://example.org/p",r#"<script type="application/ld+json">{"@graph":[{"@type":"Product","url":"/p","identifier":{"@type":"PropertyValue","value":"OWN"}},{"@type":"Product","url":"/other","identifier":{"@type":"PropertyValue","value":"OTHER"}}]}</script>"#).unwrap();
        assert_eq!(unrelated.fields.len(), 1);
        assert_eq!(unrelated.fields[0].value, "OWN");
    }
    #[test]
    fn identifier_arrays_and_property_labels_remain_bounded() {
        let values=(0..30).map(|i|serde_json::json!({"@type":"PropertyValue","propertyID":"🌷".repeat(80),"value":format!("CODE-{i:02}")})).collect::<Vec<_>>();
        let source = serde_json::json!({"@type":"Product","identifier":values});
        let info = parse(
            "https://example.org/p",
            "https://example.org/p",
            &format!("<script type='application/ld+json'>{source}</script>"),
        )
        .unwrap();
        assert_eq!(info.fields.len(), 24);
        assert!(info.limited);
        assert_eq!(info.fields[0].value, "CODE-00");
        assert_eq!(info.fields[23].value, "CODE-23");
        assert!(info
            .fields
            .iter()
            .all(|f| f.label.encode_utf16().count() <= 128 && f.source.len() <= 128));
    }
    #[test]
    fn semantic_body_preserves_source_phrases_and_excludes_non_content_subtrees() {
        let info=parse("https://example.org/p","https://example.org/p",r#"<main><h1>실제 문구 &amp; 정보</h1><p>첫 문단.</p><p>다음 <b>문구</b> 유지.</p><nav>메뉴 제외</nav><aside>관련 항목 제외</aside><p hidden>숨김 제외</p><p aria-hidden="true">숨김 둘 제외</p><div style="display: none">숨김 셋 제외</div><script>danger()</script><template>템플릿 제외</template></main>"#).unwrap();
        let body = info
            .fields
            .iter()
            .find(|f| f.source == "main:text")
            .unwrap();
        assert!(body.value.contains("실제 문구 & 정보"));
        assert!(body.value.contains("첫 문단. 다음 문구 유지."));
        for text in ["제외", "danger"] {
            assert!(!body.value.contains(text));
        }
        let ambiguous = parse(
            "https://example.org/p",
            "https://example.org/p",
            "<article>one</article><article>two</article>",
        )
        .unwrap();
        assert!(ambiguous.fields.is_empty());
        let hidden = parse(
            "https://example.org/p",
            "https://example.org/p",
            "<main hidden>never</main>",
        )
        .unwrap();
        assert!(hidden.fields.is_empty());
        let long = parse(
            "https://example.org/p",
            "https://example.org/p",
            &format!("<article>{}</article>", "🌷".repeat(1100)),
        )
        .unwrap();
        assert!(long.limited);
        assert_eq!(long.fields[0].value.encode_utf16().count(), 2048);
    }
    #[test]
    fn metadata_retains_exact_page_fields_without_executing_html() {
        let info=parse("https://example.org/p","https://example.org/p",r#"<title>A &amp; B</title><meta property="og:title" content="Different &quot;title&quot;"><meta name="description" content="한글 설명 🌷"><meta property="product:retailer_item_id" content="Example Company"><script type="application/ld+json">{"@type":"Product","name":"Item","sku":"ITEM-042","brand":{"name":"Brand"}}</script><script>fetch('/never')</script><img src="https://invalid.test/never">"#).unwrap();
        assert!(info
            .fields
            .iter()
            .any(|f| f.value == "A & B" && f.source == "title[0]"));
        assert!(info.fields.iter().any(|f| f.value == "Different \"title\""));
        assert!(info.fields.iter().any(|f| f.value == "한글 설명 🌷"));
        assert!(info.fields.iter().any(|f| f.label == "판매처 등록 ID"
            && f.value == "Example Company"
            && f.source.ends_with(":product:retailer_item_id")));
        assert!(info
            .fields
            .iter()
            .any(|f| f.value == "ITEM-042" && f.source.ends_with(".sku")));
        assert!(!info.fields.iter().any(|f| f.value.contains("fetch")));
    }
    #[test]
    fn structured_data_does_not_borrow_recommendations_or_unknown_product_identity() {
        let a=parse("https://example.org/p","https://example.org/p",r#"<script type="application/ld+json">{"@type":"ItemList","itemListElement":[{"@type":"Product","sku":"RELATED"}]}</script>"#).unwrap();
        assert!(a.fields.is_empty());
        let b=parse("https://example.org/p","https://example.org/p",r#"<script type="application/ld+json">{"@graph":[{"@type":"Product","sku":"ONE"},{"@type":"Product","sku":"TWO"}]}</script>"#).unwrap();
        assert!(b.fields.is_empty());
        assert!(b.ambiguous);
        let c=parse("https://example.org/p","https://example.org/p",r#"<script type="application/ld+json">{"@graph":[{"@type":"Product","url":"/p","sku":"OWN"},{"@type":"Product","url":"/other","sku":"OTHER"}]}</script>"#).unwrap();
        assert_eq!(c.fields.len(), 1);
        assert_eq!(c.fields[0].value, "OWN");
        let d=parse("https://example.org/p","https://example.org/p",r#"<script type="application/ld+json">{"@type":"Product","url":"https://other.test/p","sku":"FOREIGN"}</script>"#).unwrap();
        assert!(d.fields.is_empty());
    }
    #[test]
    fn source_limits_and_challenges_never_become_successful_facts() {
        assert!(parse(
            "https://example.org",
            "https://example.org",
            &"x".repeat(MAX_HTML + 1)
        )
        .is_err());
        assert!(parse(
            "https://example.org",
            "https://example.org",
            "<title>Just a moment...</title>"
        )
        .is_err());
        assert!(decode_html(
            b"<title>Invalid charset</title>",
            "text/html; charset=unknown-encoding"
        )
        .is_err());
        assert!(read_body(std::io::Cursor::new(vec![0; MAX_BYTES + 1]), None).is_err());
        assert!(read_body(std::io::Cursor::new([]), Some(MAX_BYTES as u64 + 1)).is_err());
        let (text, limited) = tidy(&"🌷".repeat(1025), 2048);
        assert_eq!(text.encode_utf16().count(), 2048);
        assert!(limited);
    }
    #[test]
    fn declared_legacy_encodings_preserve_literal_codes_and_header_or_bom_precedence() {
        let html="<meta charset='shift_jis'><main><h1>作品</h1><dl><dt>品番</dt><dd>ABC-００４２</dd></dl></main>";
        let (bytes, _, errors) = encoding_rs::SHIFT_JIS.encode(html);
        assert!(!errors);
        let decoded = decode_html(&bytes, "text/html").unwrap();
        assert_eq!(decoded, html);
        let p = parse(
            "https://example.org/work",
            "https://example.org/work",
            &decoded,
        )
        .unwrap();
        assert_eq!(p.catalog.codes[0].value, "ABC-００４２");
        assert_eq!(
            decode_html(html.as_bytes(), "text/html; charset=UTF-8").unwrap(),
            html
        );
        let mut bom = vec![0xef, 0xbb, 0xbf];
        bom.extend_from_slice(html.as_bytes());
        assert_eq!(
            decode_html(&bom, "text/html; charset=shift_jis").unwrap(),
            html
        );
        assert!(decode_html(&[0x82], "text/html; charset=shift_jis").is_err());
        assert!(decode_html(&[0xff], "text/html; charset=utf-8").is_err());
    }
    #[test]
    #[ignore = "Explicit neutral publisher retail HTTP check; no account or media"]
    fn neutral_public_retail_code_smoke() {
        let page = fetch(
            "https://nostarch.com/Rust-programming-language-3e",
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(page
            .catalog
            .title
            .as_deref()
            .is_some_and(|v| v.contains("Rust Programming Language")));
        assert!(page
            .catalog
            .codes
            .iter()
            .any(|v| v.label == "ISBN" && v.value == "9781718504448"));
        assert!(page
            .catalog
            .sale
            .iter()
            .any(|v| v.source == "meta:product:price:amount" && v.value == "59.99"));
        assert!(page
            .catalog
            .sale
            .iter()
            .any(|v| v.label == "통화" && v.value == "USD"));
        assert!(page
            .catalog
            .sale
            .iter()
            .any(|v| v.label == "구매 옵션" && v.value.contains("$47.99")));
    }
    #[test]
    #[ignore = "Explicit public-domain book title HTTP check; no account or media"]
    fn neutral_public_book_title_smoke() {
        let page = fetch(
            "https://www.gutenberg.org/ebooks/1342",
            &AtomicBool::new(false),
        )
        .unwrap();
        let title = page.catalog.title_evidence.as_ref().unwrap();
        assert_eq!(title.label, "작품명");
        assert_eq!(title.value, "Pride and Prejudice by Jane Austen");
        assert_eq!(title.source, "microdata:name");
        assert_eq!(page.catalog.title.as_deref(), Some(title.value.as_str()));
        assert!(page.catalog.codes.is_empty());
        assert!(!page.catalog.ambiguous);
    }
    #[test]
    #[ignore = "Explicit neutral retail description HTTP check; no account or media"]
    fn neutral_public_description_code_smoke() {
        let page = fetch(
            "https://www.musinsa.com/products/4494426",
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(page.catalog.codes.iter().any(|field| {
            field.label == "품번"
                && field.value == "BM243-KN001"
                && field.source.ends_with(".description:제품번호")
        }));
        assert!(page
            .catalog
            .codes
            .iter()
            .any(|field| { field.label == "SKU" && field.value == "4494426" }));
    }
    #[test]
    fn public_destination_and_cancellation_guards_reject_local_or_foreign_protocols() {
        for url in [
            "http://127.0.0.1/",
            "http://localhost/",
            "http://private.local/",
            "http://100.64.0.1/",
            "https://example.org:8443/",
            "file:///secret",
            "https://user:secret@example.org/",
        ] {
            assert!(page_url(url).is_none(), "{url}");
        }
        for address in [
            "::1",
            "::ffff:127.0.0.1",
            "fd00::1",
            "2001:db8::1",
            "192.168.1.2",
            "169.254.169.254",
        ] {
            assert!(!public_ip(address.parse().unwrap()));
        }
        assert!(page_url("https://example.org/a#part").is_some());
        assert!(fetch("https://example.org/", &AtomicBool::new(true)).is_err());
    }
    #[test]
    #[ignore = "Explicit neutral public HTTP smoke; no browser, media or account session"]
    fn neutral_public_metadata_smoke() {
        let info = fetch("https://www.rust-lang.org/", &AtomicBool::new(false)).unwrap();
        assert!(!info.fields.is_empty());
        assert!(info.url.starts_with("https://"));
        assert!(info.fields.iter().any(|f| f.label.contains("제목")));
    }
}
