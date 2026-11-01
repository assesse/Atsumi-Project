//! Candidate agreement is distinct from proof that a page describes the same
//! edition/product. Fields always stay on the page that actually declared them.
use super::super::{Candidate, Provider, SearchLink};
use super::{PageEvidence, Root, Task, Work};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Claim {
    pub provider: &'static str,
    pub position: usize,
    pub title: String,
    pub url: String,
    pub creator: Option<String>,
    pub similarity: Option<f64>,
    pub metadata: Vec<ClaimField>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct ClaimField {
    label: String,
    value: String,
}
fn bounded(value: &str, max: usize) -> String {
    let mut used = 0;
    value
        .chars()
        .filter(|c| !c.is_control())
        .take_while(|c| {
            used += c.len_utf16();
            used <= max
        })
        .collect()
}
impl Claim {
    pub(super) fn new(
        provider: Provider,
        index: usize,
        candidate: &Candidate,
        link: &SearchLink,
    ) -> Self {
        Self {
            provider: if provider == Provider::Lens {
                "lens"
            } else {
                "saucenao"
            },
            position: index + 1,
            title: bounded(&candidate.title, 512),
            url: link.url.clone(),
            creator: candidate.creator.as_deref().map(|v| bounded(v, 256)),
            similarity: candidate.similarity,
            metadata: candidate
                .source_metadata
                .iter()
                .take(8)
                .map(|v| ClaimField {
                    label: bounded(&v.label, 128),
                    value: bounded(&v.value, 512),
                })
                .collect(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Context {
    pub group: usize,
    pub status: &'static str,
    pub kind: &'static str,
    pub association: &'static str,
    pub page_status: &'static str,
    pub page_title: Option<String>,
    pub claims: Vec<Claim>,
    pub reasons: Vec<String>,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            group: 0,
            status: "page",
            kind: "page",
            association: "unverified",
            page_status: "not_read",
            page_title: None,
            claims: vec![],
            reasons: vec![],
        }
    }
}
fn add_claims(target: &mut Vec<Claim>, claims: &[Claim]) {
    for claim in claims {
        if target.len() < 16 && !target.contains(claim) {
            target.push(claim.clone());
        }
    }
    target.sort_by(|a, b| {
        (a.provider != "saucenao")
            .cmp(&(b.provider != "saucenao"))
            .then(a.position.cmp(&b.position))
            .then(a.url.cmp(&b.url))
    });
}
pub(super) fn merge_roots(roots: &mut Vec<Root>) {
    let mut merged: Vec<Root> = Vec::new();
    for root in roots.drain(..) {
        if let Some(existing) = merged.iter_mut().find(|v| v.url == root.url) {
            add_claims(&mut existing.claims, &root.claims);
        } else {
            merged.push(root);
        }
    }
    *roots = merged;
}
fn words(value: &str) -> Vec<String> {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}
// Prefix/annotation agreement groups search hypotheses only. It must never be
// used alone to copy a code or offer from another URL into a work.
fn title_agreement(a: &str, b: &str) -> bool {
    let a = words(a);
    let b = words(b);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return a.iter().map(String::len).sum::<usize>() >= 4;
    }
    let prefix = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let meaningful = a
        .iter()
        .take(prefix)
        .filter(|v| v.chars().any(char::is_alphabetic))
        .count();
    let length = a.iter().take(prefix).map(String::len).sum::<usize>();
    // A trailing edition/episode number is significant even if titles share a
    // long prefix. Date/resolution tails are not converted into identifiers.
    let number = |v: &[String]| {
        v.iter()
            .take(prefix + 3)
            .find(|v| v.chars().all(|c| c.is_ascii_digit()) && v.len() < 4)
            .cloned()
    };
    if let (Some(x), Some(y)) = (number(&a), number(&b)) {
        if x != y {
            return false;
        }
    }
    prefix >= 3
        && meaningful >= 2
        && length >= 10
        && (prefix == a.len().min(b.len())
            || a.get(prefix).is_some_and(|v| annotation(v))
            || b.get(prefix).is_some_and(|v| annotation(v)))
}
fn annotation(value: &str) -> bool {
    matches!(
        value,
        "ep" | "episode" | "episodes" | "ova" | "hd" | "edition" | "vol" | "volume"
    ) || value.chars().all(|c| c.is_ascii_digit())
}
fn resource(value: &str) -> String {
    let Ok(mut url) = tauri::Url::parse(value) else {
        return value.into();
    };
    url.set_fragment(None);
    // Never strip product/edition query parameters or change hosts.
    url.to_string()
}
fn global_codes(work: &Work) -> HashMap<String, HashSet<String>> {
    let mut result: HashMap<String, HashSet<String>> = HashMap::new();
    for field in &work.codes {
        let label = field.label.to_ascii_uppercase();
        let family = if label == "ISBN" {
            "ISBN"
        } else if label.starts_with("GTIN") || matches!(label.as_str(), "JAN" | "EAN" | "UPC") {
            "GTIN"
        } else {
            continue;
        };
        let code = field
            .value
            .chars()
            .filter(|c| *c != '-' && !c.is_whitespace())
            .collect::<String>();
        if let Some(code) = normalized_identifier(family, &code) {
            result.entry(family.into()).or_default().insert(code);
        }
    }
    result
}
fn normalized_identifier(family: &str, code: &str) -> Option<String> {
    if family == "ISBN" && code.len() == 10 {
        let digits: Vec<_> = code
            .chars()
            .enumerate()
            .map(|(i, c)| {
                if i == 9 && matches!(c, 'x' | 'X') {
                    Some(10)
                } else {
                    c.to_digit(10)
                }
            })
            .collect::<Option<Vec<_>>>()?;
        if digits
            .iter()
            .enumerate()
            .map(|(i, n)| (10 - i) as u32 * n)
            .sum::<u32>()
            % 11
            != 0
        {
            return None;
        }
        let prefix = format!("978{}", &code[..9]);
        let sum = prefix
            .bytes()
            .enumerate()
            .map(|(i, n)| u32::from(n - b'0') * if i % 2 == 0 { 1 } else { 3 })
            .sum::<u32>();
        return Some(format!("{prefix}{}", (10 - sum % 10) % 10));
    }
    if !code.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if family == "ISBN"
        && (code.len() != 13 || !(code.starts_with("978") || code.starts_with("979")))
    {
        return None;
    }
    if !matches!(code.len(), 8 | 12 | 13 | 14) {
        return None;
    }
    let sum = code
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, n)| u32::from(n - b'0') * if i % 2 == 0 { 1 } else { 3 })
        .sum::<u32>();
    if sum % 10 != 0 || code.bytes().all(|c| c == b'0') {
        return None;
    }
    Some(if family == "GTIN" {
        format!("{code:0>14}")
    } else {
        code.into()
    })
}
fn code_relation(a: &Work, b: &Work) -> (bool, bool) {
    let a = global_codes(a);
    let b = global_codes(b);
    let same = a
        .iter()
        .any(|(k, v)| b.get(k).is_some_and(|other| !v.is_disjoint(other)));
    let conflict = a
        .iter()
        .any(|(k, v)| b.get(k).is_some_and(|other| v.is_disjoint(other)));
    (same, conflict)
}
fn work(root: &Root) -> Work {
    Work {
        title: root.claims.first().map(|c| c.title.clone()),
        title_evidence: None,
        url: root.url.clone(),
        root_url: root.via.clone().unwrap_or_else(|| root.url.clone()),
        origin: root.label.clone(),
        observed_at: chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
            .to_rfc3339(),
        codes: vec![],
        sale: vec![],
        ambiguous: false,
        context: Context {
            status: if root.claims.is_empty() {
                "page"
            } else {
                "candidate"
            },
            claims: root.claims.clone(),
            ..Default::default()
        },
    }
}
fn note(work: &mut Work, text: &str) {
    if work.context.reasons.len() < 6 && !work.context.reasons.iter().any(|v| v == text) {
        work.context.reasons.push(text.into());
    }
}
fn conflict(work: &mut Work, text: &str) {
    work.context.status = "conflict";
    work.context.association = "conflict";
    note(work, text);
}

pub(super) fn reconcile(
    roots: &[Root],
    pages: &[(Task, PageEvidence)],
    failed: &HashSet<String>,
) -> Vec<Work> {
    let mut works: Vec<Work> = roots
        .iter()
        .filter(|r| !r.claims.is_empty())
        .take(24)
        .map(|root| {
            let mut value = work(root);
            if failed.contains(&root.url) {
                value.context.page_status = "unavailable";
                note(&mut value, "페이지를 읽지 못해 검색 후보를 유지했습니다.");
            }
            value
        })
        .collect();
    let mut order: Vec<_> = pages.iter().collect();
    order.sort_by(|(a, _), (b, _)| a.depth.cmp(&b.depth).then(a.url.cmp(&b.url)));
    let mut trails = Vec::new();
    for (task, page) in order {
        let catalog = &page.catalog;
        let existing = works
            .iter()
            .position(|w| w.url == task.url)
            .or_else(|| works.iter().position(|w| w.url == page.url));
        let useful = !catalog.codes.is_empty()
            || !catalog.sale.is_empty()
            || catalog.title_evidence.is_some();
        if existing.is_none() && (!useful || works.len() >= 48) {
            continue;
        }
        let index = existing.unwrap_or_else(|| {
            works.push(work(&task.root));
            let i = works.len() - 1;
            works[i].url = page.url.clone();
            works[i].context.claims.clear();
            i
        });
        let value = &mut works[index];
        value.context.page_status = "read";
        value.context.kind = if catalog.kind.is_empty() {
            "page"
        } else {
            catalog.kind
        };
        value.context.page_title = catalog.title.as_deref().map(|title| bounded(title, 512));
        value.observed_at = page.observed_at.clone();
        value.url = page.url.clone();
        value.ambiguous = catalog.ambiguous;
        if value.context.claims.is_empty() {
            value.title = value.context.page_title.clone();
            value.title_evidence = catalog
                .title_evidence
                .clone()
                .filter(|_| !catalog.ambiguous);
        } else if value.title == catalog.title && !catalog.ambiguous {
            value.title_evidence = catalog.title_evidence.clone();
        }
        value.codes = catalog.codes.clone();
        value.sale = catalog.sale.clone();
        let agrees = value.context.claims.iter().any(|c| {
            catalog
                .title
                .as_deref()
                .is_some_and(|title| title_agreement(&c.title, title))
        });
        if catalog.ambiguous || global_codes(value).values().any(|ids| ids.len() > 1) {
            conflict(
                value,
                "페이지에 여러 항목이 있어 품번·판매정보를 연결하지 않았습니다.",
            );
        } else if task.root.via.is_some() && !task.trail.iter().any(|(_, _, r)| *r == "derived") {
            value.context.status = "source_link";
            value.context.association = "source_link";
            value.title = value
                .context
                .page_title
                .clone()
                .or_else(|| value.title.clone());
            value.title_evidence = catalog.title_evidence.clone();
            note(value, "원 게시물·게시자 타래에 직접 등록된 링크입니다.");
            if !value.context.claims.is_empty() && !agrees {
                note(value,"검색 제목과 출처 제목이 다릅니다. 게시자가 연결한 페이지의 원문 정보를 표시합니다.");
            }
        } else if !value.context.claims.is_empty() && !agrees && catalog.title.is_some() && useful {
            conflict(
                value,
                "검색 제목과 페이지 제목이 달라 품번·판매정보 연결을 보류했습니다.",
            );
        } else if agrees && matches!(catalog.kind, "work" | "product") {
            value.context.association = "same_page";
            note(
                value,
                "검색 결과가 가리키는 동일 페이지의 명시된 정보입니다.",
            );
        } else if value.context.claims.is_empty()
            && task.root.claims.is_empty()
            && task.depth > 0
            && !task.trail.iter().any(|(_, _, r)| *r == "derived")
        {
            value.context.association = "source_link";
            note(value, "출처 페이지에 명시된 연결 링크입니다. 검색 이미지와의 일치는 별도 확인이 필요합니다.");
        }
        if catalog.kind == "upload" {
            note(
                value,
                "업로드 페이지 제목이며 원본 작품명으로 확정하지 않았습니다.",
            );
        }
        if !task.trail.is_empty() {
            trails.push((page.url.clone(), task.trail.clone()));
        }
    }
    // Multiple search providers often use the same destination. Keep all
    // claims but do not count two mirrors from one provider as two providers.
    let mut dedup: Vec<Work> = Vec::new();
    for value in works {
        if let Some(existing) = dedup
            .iter_mut()
            .find(|w| resource(&w.url) == resource(&value.url))
        {
            add_claims(&mut existing.context.claims, &value.context.claims);
        } else {
            dedup.push(value);
        }
    }
    let mut works = dedup;
    for (i, w) in works.iter_mut().enumerate() {
        w.context.group = i;
    }
    let mut edges = Vec::new();
    for i in 0..works.len() {
        for j in i + 1..works.len() {
            let (same_code, incompatible_code) = code_relation(&works[i], &works[j]);
            let a = &works[i];
            let b = &works[j];
            let title_match = a
                .title
                .as_deref()
                .zip(b.title.as_deref())
                .is_some_and(|(a, b)| title_agreement(a, b));
            let explicit = trails.iter().any(|(target, trail)| {
                target == &b.url
                    && trail.iter().any(|(from, to, r)| {
                        from == &a.url
                            && to == target
                            && matches!(*r, "identity" | "source" | "offer" | "purchase")
                    })
            }) || trails.iter().any(|(target, trail)| {
                target == &a.url
                    && trail.iter().any(|(from, to, r)| {
                        from == &b.url
                            && to == target
                            && matches!(*r, "identity" | "source" | "offer" | "purchase")
                    })
            });
            if incompatible_code && (title_match || explicit) {
                conflict(
                    &mut works[i],
                    "출처의 식별자가 달라 다른 판본·상품일 수 있습니다.",
                );
                conflict(
                    &mut works[j],
                    "출처의 식별자가 달라 다른 판본·상품일 수 있습니다.",
                );
                continue;
            }
            if works[i].context.status == "conflict" || works[j].context.status == "conflict" {
                continue;
            }
            if explicit
                && !title_match
                && a.title.is_some()
                && b.title.is_some()
                && !matches!(a.context.kind, "upload" | "page")
                && !matches!(b.context.kind, "upload" | "page")
            {
                conflict(
                    &mut works[j],
                    "연결된 페이지의 제목이 달라 같은 작품으로 묶지 않았습니다.",
                );
                continue;
            }
            if same_code || explicit {
                edges.push((i, j));
                for k in [i, j] {
                    works[k].context.association = if same_code {
                        "identifier"
                    } else {
                        "explicit_link"
                    };
                    note(
                        &mut works[k],
                        if same_code {
                            "출처에 명시된 ISBN·GTIN 식별자가 일치합니다."
                        } else {
                            "작품 페이지의 명시된 출처·판매 링크로 연결됩니다."
                        },
                    );
                }
            } else if title_match && !a.context.claims.is_empty() && !b.context.claims.is_empty() {
                edges.push((i, j));
                note(&mut works[i], "검색 제목이 일치하는 후보입니다. 제목만으로 다른 페이지의 판매정보를 연결하지 않습니다.");
                note(&mut works[j], "검색 제목이 일치하는 후보입니다. 제목만으로 다른 페이지의 판매정보를 연결하지 않습니다.");
            }
        }
    }
    // Union only non-conflicting groups; an intermediate title cannot bridge
    // two incompatible editions through transitivity.
    for (a, b) in edges {
        let ga = works[a].context.group;
        let gb = works[b].context.group;
        let left: Vec<_> = works.iter().filter(|w| w.context.group == ga).collect();
        let right: Vec<_> = works.iter().filter(|w| w.context.group == gb).collect();
        if left
            .iter()
            .chain(right.iter())
            .any(|w| w.context.status == "conflict")
            || left
                .iter()
                .any(|x| right.iter().any(|y| code_relation(x, y).1))
        {
            continue;
        }
        for w in &mut works {
            if w.context.group == gb {
                w.context.group = ga;
            }
        }
    }
    let mut providers: HashMap<usize, HashSet<&str>> = HashMap::new();
    for w in &works {
        for c in &w.context.claims {
            providers
                .entry(w.context.group)
                .or_default()
                .insert(c.provider);
        }
    }
    for w in &mut works {
        if !matches!(w.context.status, "conflict" | "source_link")
            && providers.get(&w.context.group).is_some_and(|p| p.len() > 1)
        {
            w.context.status = "supported";
            note(
                w,
                "두 검색 서비스에서 같은 후보가 나타났습니다. 원본 확정은 아닙니다.",
            );
        }
    }
    let rank = |w: &Work| {
        if w.context.status == "source_link" {
            0
        } else if w.context.status == "supported" {
            1
        } else if w.context.claims.iter().any(|c| c.provider == "saucenao") {
            2
        } else if !w.context.claims.is_empty() {
            3
        } else {
            4
        }
    };
    works.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(
                a.context
                    .claims
                    .first()
                    .map(|v| v.position)
                    .unwrap_or(99)
                    .cmp(&b.context.claims.first().map(|v| v.position).unwrap_or(99)),
            )
            .then(a.url.cmp(&b.url))
    });
    works
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editions_and_short_common_titles_are_not_title_agreement() {
        assert!(!title_agreement(
            "The Complete Example 2",
            "The Complete Example 3"
        ));
        assert!(!title_agreement("A", "A"));
        assert!(!title_agreement(
            "The Complete Example Red",
            "The Complete Example Blue"
        ));
        assert!(title_agreement(
            "Aurora Garden Story",
            "Aurora Garden Story ep 2 HD"
        ));
    }
    #[test]
    fn global_identity_requires_checksum_and_normalizes_equivalent_representations() {
        assert_eq!(
            normalized_identifier("ISBN", "1718500440"),
            normalized_identifier("ISBN", "9781718500440")
        );
        assert!(normalized_identifier("ISBN", "9781718504448").is_some());
        assert!(normalized_identifier("ISBN", "9781718504449").is_none());
        assert!(normalized_identifier("GTIN", "0000000000000").is_none());
        assert_eq!(
            normalized_identifier("GTIN", "036000291452"),
            normalized_identifier("GTIN", "0036000291452")
        );
    }
}
