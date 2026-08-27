use std::collections::BTreeMap;

use crate::{
    domain::{canonical_tag_token, normalize_tag_name, TagCatalogEntry, TagNamespace},
    source::SourceContractError,
};

pub const ALL_TAGS_PAGE_COUNT: usize = 27;
pub const ALL_CATALOG_PAGE_COUNT: usize = ALL_TAGS_PAGE_COUNT * 3;

pub fn all_tags_urls() -> Vec<String> {
    catalog_index_urls("alltags")
}

pub fn all_catalog_pages() -> Vec<(TagNamespace, String)> {
    [
        (TagNamespace::Tag, "alltags"),
        (TagNamespace::Artist, "allartists"),
        (TagNamespace::Group, "allgroups"),
    ]
    .into_iter()
    .flat_map(|(namespace, prefix)| {
        catalog_index_urls(prefix)
            .into_iter()
            .map(move |url| (namespace, url))
    })
    .collect()
}

fn catalog_index_urls(prefix: &str) -> Vec<String> {
    std::iter::once(format!("https://hitomi.la/{prefix}-123.html"))
        .chain(('a'..='z').map(|letter| format!("https://hitomi.la/{prefix}-{letter}.html")))
        .collect()
}

pub fn parse_all_tags_page(html: &str) -> Result<Vec<TagCatalogEntry>, SourceContractError> {
    parse_catalog_page(html, TagNamespace::Tag)
}

pub fn parse_catalog_page(
    html: &str,
    page_namespace: TagNamespace,
) -> Result<Vec<TagCatalogEntry>, SourceContractError> {
    let mut entries = Vec::new();
    let mut cursor = html;
    while let Some(start) = cursor.find("<a") {
        cursor = &cursor[start + 2..];
        let Some(end) = cursor.find('>') else {
            return Err(SourceContractError::invalid_data(
                "metadata catalog",
                "unterminated anchor",
            ));
        };
        let attrs = &cursor[..end];
        let rest = &cursor[end + 1..];
        let Some(close) = rest.find("</a>") else {
            return Err(SourceContractError::invalid_data(
                "metadata catalog",
                "anchor is missing closing tag",
            ));
        };
        let text = strip_html(&rest[..close]);
        let after_anchor = &rest[close + 4..];
        cursor = after_anchor;
        let Some(href) = attribute(attrs, "href") else {
            continue;
        };
        let Some((namespace, encoded_name)) = catalog_href(&href, page_namespace)? else {
            continue;
        };
        let name = percent_decode(&encoded_name)?;
        let name = normalize_tag_name(&name);
        let count = anchor_count(&text, after_anchor)?;
        let canonical_token = canonical_tag_token(namespace, &name).map_err(|error| {
            SourceContractError::invalid_data("metadata catalog", error.to_string())
        })?;
        entries.push(TagCatalogEntry {
            namespace,
            normalized_name: name.clone(),
            name,
            canonical_token,
            gallery_count: count,
        });
    }
    if entries.is_empty() {
        return Err(SourceContractError::invalid_data(
            "metadata catalog",
            "page contains no valid catalog anchors",
        ));
    }
    Ok(entries)
}

pub fn merge_catalog(
    entries: impl IntoIterator<Item = TagCatalogEntry>,
) -> Result<Vec<TagCatalogEntry>, SourceContractError> {
    let mut result = BTreeMap::<(String, String), TagCatalogEntry>::new();
    let mut tokens = BTreeMap::<String, (String, String)>::new();
    for entry in entries {
        let key = (entry.namespace.as_str().to_owned(), entry.name.clone());
        if let Some(existing) = tokens.insert(entry.canonical_token.clone(), key.clone()) {
            if existing != key {
                return Err(SourceContractError::invalid_data(
                    "metadata catalog",
                    "canonical token collision",
                ));
            }
        }
        result
            .entry(key)
            .and_modify(|current| {
                current.gallery_count = current.gallery_count.max(entry.gallery_count)
            })
            .or_insert(entry);
    }
    if result.len() < 1_000 {
        return Err(SourceContractError::invalid_data(
            "metadata catalog",
            "catalog contains fewer than 1000 entries",
        ));
    }
    Ok(result.into_values().collect())
}

fn attribute(attrs: &str, wanted: &str) -> Option<String> {
    let mut rest = attrs;
    while !rest.trim_start().is_empty() {
        rest = rest.trim_start();
        let equals = rest.find('=')?;
        let key = rest[..equals].trim();
        rest = &rest[equals + 1..];
        let quote = rest.chars().next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        rest = &rest[1..];
        let end = rest.find(quote)?;
        let value = &rest[..end];
        rest = &rest[end + 1..];
        if key.eq_ignore_ascii_case(wanted) {
            return Some(value.to_owned());
        }
    }
    None
}
fn catalog_href(
    href: &str,
    page_namespace: TagNamespace,
) -> Result<Option<(TagNamespace, String)>, SourceContractError> {
    match page_namespace {
        TagNamespace::Tag => tag_href(href),
        TagNamespace::Artist => namespaced_href(href, "/artist/", TagNamespace::Artist),
        TagNamespace::Group => namespaced_href(href, "/group/", TagNamespace::Group),
        TagNamespace::Female | TagNamespace::Male => Err(SourceContractError::invalid_data(
            "metadata catalog",
            "gender namespaces do not own index pages",
        )),
    }
}

fn namespaced_href(
    href: &str,
    prefix: &str,
    namespace: TagNamespace,
) -> Result<Option<(TagNamespace, String)>, SourceContractError> {
    let Some(value) = href.strip_prefix(prefix) else {
        return Ok(None);
    };
    let Some(value) = value.strip_suffix("-all.html") else {
        return Ok(None);
    };
    if value.is_empty() {
        return Err(SourceContractError::invalid_data(
            "metadata catalog",
            "catalog href has empty name",
        ));
    }
    Ok(Some((namespace, value.to_owned())))
}

fn tag_href(href: &str) -> Result<Option<(TagNamespace, String)>, SourceContractError> {
    let Some(value) = href.strip_prefix("/tag/") else {
        return Ok(None);
    };
    let Some(value) = value.strip_suffix("-all.html") else {
        return Ok(None);
    };
    if value.is_empty() {
        return Err(SourceContractError::invalid_data(
            "metadata catalog",
            "tag href has empty name",
        ));
    }
    if let Some(name) = value
        .strip_prefix("female%3A")
        .or_else(|| value.strip_prefix("female%3a"))
    {
        return Ok(Some((TagNamespace::Female, name.to_owned())));
    }
    if let Some(name) = value
        .strip_prefix("male%3A")
        .or_else(|| value.strip_prefix("male%3a"))
    {
        return Ok(Some((TagNamespace::Male, name.to_owned())));
    }
    // A colon inside any other percent-encoded name is part of the neutral
    // tag itself (for example `circle: honey maple chicken`), not a namespace.
    // The catalog intentionally owns only the two special gender namespaces.
    Ok(Some((TagNamespace::Tag, value.to_owned())))
}
fn percent_decode(value: &str) -> Result<String, SourceContractError> {
    let mut bytes = Vec::new();
    let raw = value.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%' {
            if index + 2 >= raw.len() {
                return Err(SourceContractError::invalid_data(
                    "metadata catalog",
                    "invalid percent encoding",
                ));
            }
            let nibble = |byte: u8| -> Option<u8> {
                match byte {
                    b'0'..=b'9' => Some(byte - b'0'),
                    b'a'..=b'f' => Some(byte - b'a' + 10),
                    b'A'..=b'F' => Some(byte - b'A' + 10),
                    _ => None,
                }
            };
            let (Some(a), Some(b)) = (nibble(raw[index + 1]), nibble(raw[index + 2])) else {
                return Err(SourceContractError::invalid_data(
                    "metadata catalog",
                    "invalid percent encoding",
                ));
            };
            bytes.push(a * 16 + b);
            index += 3;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| SourceContractError::invalid_data("metadata catalog", "href is not UTF-8"))
}
fn strip_html(value: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in value.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out
}
fn anchor_count(text: &str, after_anchor: &str) -> Result<u64, SourceContractError> {
    // Hitomi's catalog pages render the count after the closing anchor:
    // `<a ...>tag</a> (123)`. Keep support for the inline form used by older
    // pages/fixtures, but never scan past the next HTML element.
    count_in_text(text)
        .or_else(|| count_in_text(after_anchor.split('<').next().unwrap_or_default()))
        .ok_or_else(|| {
            SourceContractError::invalid_data("metadata catalog", "anchor is missing gallery count")
        })
}

fn count_in_text(text: &str) -> Option<u64> {
    let start = text.rfind('(')?;
    let end = text[start + 1..]
        .find(')')
        .map(|offset| start + 1 + offset)?;
    text[start + 1..end].trim().replace(',', "").parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_href_and_count() {
        let rows=parse_all_tags_page("<a href=\"/tag/female%3Abig%20balls-all.html\">big balls</a> (4,822)<a href='/tag/webtoon-all.html'>webtoon (3)</a><a href='/tag/circle%3A%20honey-all.html'>circle: honey</a> (2)").unwrap();
        assert_eq!(rows[0].canonical_token, "female:big_balls");
        assert_eq!(rows[0].gallery_count, 4822);
        assert_eq!(rows[1].canonical_token, "tag:webtoon");
        assert_eq!(rows[2].canonical_token, "tag:circle:_honey");
    }
    #[test]
    fn has_all_urls() {
        assert_eq!(all_tags_urls().len(), ALL_TAGS_PAGE_COUNT);
        let pages = all_catalog_pages();
        assert_eq!(pages.len(), ALL_CATALOG_PAGE_COUNT);
        assert!(pages.contains(&(
            TagNamespace::Artist,
            "https://hitomi.la/allartists-123.html".to_owned()
        )));
        assert!(pages.contains(&(
            TagNamespace::Group,
            "https://hitomi.la/allgroups-z.html".to_owned()
        )));
    }

    #[test]
    fn parses_artist_and_group_catalog_pages_into_search_tokens() {
        let artists = parse_catalog_page(
            "<a href='/artist/mizuno%20tooru-all.html'>mizuno tooru</a> (142)",
            TagNamespace::Artist,
        )
        .unwrap();
        let groups = parse_catalog_page(
            "<a href='/group/circle%20energy-all.html'>circle energy</a> (76)",
            TagNamespace::Group,
        )
        .unwrap();

        assert_eq!(artists[0].canonical_token, "artist:mizuno_tooru");
        assert_eq!(artists[0].gallery_count, 142);
        assert_eq!(groups[0].canonical_token, "group:circle_energy");
        assert_eq!(groups[0].gallery_count, 76);
    }

    #[test]
    fn rejects_cross_namespace_links_in_catalog_pages() {
        assert!(parse_catalog_page(
            "<a href='/group/not-an-artist-all.html'>wrong</a> (1)",
            TagNamespace::Artist,
        )
        .is_err());
    }

    #[test]
    fn merge_rejects_an_incomplete_catalog_and_keeps_maximum_count() {
        assert!(
            merge_catalog(parse_all_tags_page("<a href='/tag/a-all.html'>a (1)</a>").unwrap())
                .is_err()
        );
        let entries = (0..1_000).map(|index| TagCatalogEntry {
            namespace: TagNamespace::Tag,
            name: format!("tag {index}"),
            normalized_name: format!("tag {index}"),
            canonical_token: format!("tag:tag_{index}"),
            gallery_count: index,
        });
        assert_eq!(merge_catalog(entries).unwrap().len(), 1_000);
    }
}
