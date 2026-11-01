//! Inert, bounded publisher-declared work/retail facts. Never infer a code
//! from a URL or confuse related products with the primary page entity.
use super::page_evidence::{identifiers, identity, structured_items, tidy, Field};
use dom_query::{Document, Selection};
use serde_json::Value;
use tauri::Url;

#[derive(Clone, Debug, Default)]
pub(super) struct CatalogPage {
    pub title: Option<String>,
    pub title_evidence: Option<Field>,
    pub codes: Vec<Field>,
    pub sale: Vec<Field>,
    pub links: Vec<String>,
    pub relations: Vec<(String, &'static str)>,
    pub kind: &'static str,
    pub ambiguous: bool,
    pub limited: bool,
}
const CODES: [(&str, &str); 11] = [
    ("sku", "SKU"),
    ("productID", "상품 ID"),
    ("mpn", "품번"),
    ("isbn", "ISBN"),
    ("gtin", "GTIN"),
    ("gtin8", "GTIN-8"),
    ("gtin12", "GTIN-12"),
    ("gtin13", "GTIN-13"),
    ("gtin14", "GTIN-14"),
    ("serialNumber", "일련번호"),
    ("catalogNumber", "카탈로그 번호"),
];
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}
fn push(fields: &mut Vec<Field>, label: &str, value: &str, source: &str, limited: &mut bool) {
    let (value, cut) = tidy(value, 256);
    *limited |= cut;
    if cut || value.is_empty() || fields.iter().any(|v| v.label == label && v.value == value) {
        return;
    }
    if fields.len() >= 16 {
        *limited = true;
        return;
    }
    fields.push(Field {
        label: label.into(),
        value,
        source: source.into(),
    });
}
fn text(node: &Selection<'_>) -> String {
    node.attr("content")
        .or_else(|| node.attr("value"))
        .map(|v| v.to_string())
        .unwrap_or_else(|| node.text().to_string())
}
fn own_item(scope: &Selection<'_>, node: &Selection<'_>) -> bool {
    let mut parent = node.parent();
    for _ in 0..32 {
        if parent.is_empty() {
            return false;
        }
        if parent.is("[itemscope]") {
            return parent.is_selection(scope);
        }
        parent = parent.parent();
    }
    false
}
fn link(page: &mut CatalogPage, base: &Url, raw: &str, relation: &'static str) {
    if page.links.len() >= 4 {
        page.limited = true;
        return;
    }
    let Some(url) = super::page_evidence::page_url(raw).or_else(|| {
        base.join(raw)
            .ok()
            .and_then(|v| super::page_evidence::page_url(v.as_str()))
    }) else {
        return;
    };
    if base.scheme() == "https" && url.scheme() != "https" {
        return;
    }
    let path = url.path().to_ascii_lowercase();
    if path.split('/').any(|v| {
        matches!(
            v,
            "cart"
                | "checkout"
                | "basket"
                | "logout"
                | "login"
                | "signin"
                | "subscribe"
                | "purchase"
                | "order"
                | "add-to-cart"
        )
    }) || url.query_pairs().any(|(k, _)| {
        matches!(
            k.to_ascii_lowercase().as_str(),
            "action" | "add-to-cart" | "addtocart" | "checkout"
        )
    }) {
        return;
    }
    if [
        ".mp4", ".jpg", ".png", ".webp", ".gif", ".zip", ".pdf", ".m3u8",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
    {
        return;
    }
    if !page.links.iter().any(|v| v == url.as_str()) {
        page.relations.push((url.to_string(), relation));
        page.links.push(url.into());
    }
}
fn offers(page: &mut CatalogPage, value: &Value, path: &str, base: &Url) {
    let values: Vec<&Value> = value
        .as_array()
        .map(|v| v.iter().take(4).collect())
        .unwrap_or_else(|| vec![value]);
    page.limited |= value.as_array().is_some_and(|v| v.len() > 4);
    for (i, item) in values.into_iter().enumerate() {
        if !item.is_object() {
            continue;
        }
        for (key, label) in [
            ("price", "판매 가격"),
            ("priceCurrency", "통화"),
            ("availability", "판매 상태"),
        ] {
            if let Some(v) = item.get(key).and_then(scalar) {
                push(
                    &mut page.sale,
                    label,
                    &v,
                    &format!("{path}[{i}].{key}"),
                    &mut page.limited,
                );
            }
        }
        if let Some(url) = item.get("url").and_then(Value::as_str) {
            link(page, base, url, "offer");
        }
    }
}
fn detail_label(label: &str) -> Option<&'static str> {
    let normalized = label
        .trim()
        .trim_end_matches([':', '：'])
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "品番" | "製品番号" | "型番" | "メーカー品番" | "メーカー型番" | "품번" | "제품번호"
        | "mpn" => Some("품번"),
        "商品番号" | "商品コード" | "製品コード" | "sku" | "상품번호" | "상품코드" | "商品id"
        | "製品id" | "상품id" | "제품코드" | "productcode" | "productid" | "itemnumber"
        | "itemcode" | "productnumber" => Some("상품 번호"),
        "作品番号" | "作品コード" | "작품번호" | "카탈로그번호" | "catalognumber" => {
            Some("작품 번호")
        }
        "isbn" | "isbn13" | "isbn-13" | "isbn10" | "isbn-10" => Some("ISBN"),
        "jan" | "janコード" => Some("JAN"),
        "ean" | "ean-13" => Some("EAN"),
        "upc" => Some("UPC"),
        "asin" => Some("ASIN"),
        "gtin" => Some("GTIN"),
        _ => None,
    }
}
fn declared_work_label(value: &Value) -> Option<&'static str> {
    let has = |name: &str| {
        value.get("@type").is_some_and(|kind| {
            kind.as_str() == Some(name)
                || kind
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some(name)))
        })
    };
    if has("Article") || has("NewsArticle") {
        return None;
    }
    if ["Book", "Movie", "MusicRecording", "MusicAlbum"]
        .iter()
        .any(|kind| has(kind))
    {
        Some("작품명")
    } else if has("Product") {
        Some("상품명")
    } else if has("VideoObject") || has("ImageObject") {
        Some("업로드 제목")
    } else if has("CreativeWork") {
        Some("페이지 제목")
    } else {
        None
    }
}
fn literal_title(value: &str, label: &str, source: &str, limited: &mut bool) -> Option<Field> {
    let (value, cut) = tidy(value, 512);
    let cut = cut || value.encode_utf16().count() > 512;
    *limited |= cut;
    if cut || value.is_empty() {
        return None;
    }
    Some(Field {
        label: label.into(),
        value,
        source: source.into(),
    })
}
fn description_boundary(label: &str) -> bool {
    matches!(
        label
            .split_whitespace()
            .collect::<String>()
            .to_ascii_lowercase()
            .as_str(),
        "제품분류"
            | "상품분류"
            | "브랜드"
            | "제품"
            | "상품"
            | "제품명"
            | "상품명"
            | "작품명"
            | "제목"
            | "가격"
            | "판매가격"
            | "색상"
            | "크기"
            | "설명"
            | "brand"
            | "category"
            | "product"
            | "productname"
            | "name"
            | "title"
            | "price"
            | "color"
            | "colour"
            | "size"
            | "description"
            | "商品名"
            | "作品名"
            | "製品名"
            | "ブランド"
            | "カテゴリ"
            | "カテゴリー"
            | "価格"
            | "色"
            | "サイズ"
            | "説明"
    )
}
fn description_codes(page: &mut CatalogPage, raw: &str, path: &str) {
    // Some primary product descriptions are literal field lists. Do not
    // search ordinary prose, truncate an identifier, or guess from digits.
    if raw.len() > 16 * 1024 || raw.encode_utf16().count() > 4096 {
        page.limited = true;
        return;
    }
    let raw = raw.trim();
    if raw.contains('<') {
        return;
    }
    let mut fields = Vec::new();
    for (colon, character) in raw.char_indices().filter(|(_, c)| matches!(c, ':' | '：')) {
        if fields.len() >= 64 {
            page.limited = true;
            return;
        }
        let prefix = raw[..colon].trim_end();
        let matched = prefix
            .char_indices()
            .rev()
            .take(64)
            .filter_map(|(start, _)| {
                if start > 0 && !prefix[..start].ends_with(char::is_whitespace) {
                    return None;
                }
                let label = &prefix[start..];
                let code = detail_label(label);
                (code.is_some() || description_boundary(label)).then_some((start, code, label))
            })
            .last();
        if let Some((start, code, label)) = matched {
            fields.push((start, colon + character.len_utf8(), code, label));
        }
    }
    // A field-like phrase embedded in a review is not a product declaration.
    if fields.first().is_none_or(|field| field.0 != 0) {
        return;
    }
    if raw.contains(['\n', '\r']) {
        let mut offset = 0;
        for line in raw.split_inclusive('\n') {
            let start = offset + line.len() - line.trim_start().len();
            if !line.trim().is_empty() && !fields.iter().any(|field| field.0 == start) {
                return;
            }
            offset += line.len();
        }
    } else if fields[0].2.is_none()
        && !matches!(
            fields[0]
                .3
                .split_whitespace()
                .collect::<String>()
                .to_ascii_lowercase()
                .as_str(),
            "제품분류"
                | "상품분류"
                | "브랜드"
                | "brand"
                | "category"
                | "ブランド"
                | "カテゴリ"
                | "カテゴリー"
        )
    {
        // Compact catalogue attributes have an explicit category/brand lead.
        // "Title: a review mentioning SKU: ..." is still prose.
        return;
    }
    let mut declared = Vec::new();
    for (index, (_, end, code, source_label)) in fields.iter().enumerate() {
        let Some(label) = code else { continue };
        let next = fields.get(index + 1).map_or(raw.len(), |field| field.0);
        let value = raw[*end..next].trim();
        // Unknown label boundaries or a continuation paragraph are ambiguous.
        if value.contains([':', '：', '\n', '\r']) {
            page.limited = true;
            continue;
        }
        let grouped_number = matches!(*label, "ISBN" | "JAN" | "EAN" | "UPC" | "GTIN")
            && value
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_whitespace() || matches!(c, '-' | 'X' | 'x'));
        if value.chars().any(char::is_whitespace) && !grouped_number {
            page.limited = true;
            continue;
        }
        if declared
            .iter()
            .any(|(old_label, old_value, _)| old_label == label && *old_value != value)
        {
            page.limited = true;
            return;
        }
        declared.push((*label, value, *source_label));
    }
    for (label, value, source_label) in declared {
        push(
            &mut page.codes,
            label,
            value,
            &format!("{path}:{source_label}"),
            &mut page.limited,
        );
    }
}
fn product_price_metadata(page: &mut CatalogPage, doc: &Document, identities: &[Url]) -> bool {
    if page.ambiguous || page.codes.is_empty() {
        return false;
    }
    let meta = |property: &str| {
        let nodes = doc.select(&format!("head meta[property='{property}']"));
        (nodes.length() == 1)
            .then(|| nodes.attr("content").map(|v| v.to_string()))
            .flatten()
    };
    let Some(title) = meta("og:title") else {
        return false;
    };
    if page.title.as_deref() != Some(tidy(&title, 512).0.as_str()) {
        return false;
    }
    let same_page = meta("og:url")
        .and_then(|v| Url::parse(&v).ok())
        .is_some_and(|mut v| {
            v.set_fragment(None);
            identities.contains(&v)
        });
    if !same_page {
        return false;
    }
    // CMS sites often use og:type=article even on a product page. Require a
    // product-specific identifier that agrees with this page's own code.
    let same_item = [
        "product:isbn",
        "product:retailer_part_no",
        "product:mfr_part_no",
    ]
    .iter()
    .filter_map(|key| meta(key))
    .any(|id| page.codes.iter().any(|field| field.value == id));
    if !same_item {
        return false;
    }
    let Some(price) = meta("product:price:amount") else {
        return false;
    };
    let Some(currency) = meta("product:price:currency") else {
        return false;
    };
    if price.len() > 32
        || !price.bytes().any(|c| c.is_ascii_digit())
        || !price
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b','))
        || currency.len() != 3
        || !currency.bytes().all(|c| c.is_ascii_uppercase())
    {
        return false;
    }
    push(
        &mut page.sale,
        "판매 가격",
        &price,
        "meta:product:price:amount",
        &mut page.limited,
    );
    push(
        &mut page.sale,
        "통화",
        &currency,
        "meta:product:price:currency",
        &mut page.limited,
    );
    true
}
fn purchase_options(page: &mut CatalogPage, scope: &Selection<'_>, base: &Url, identities: &[Url]) {
    let has_price = |value: &str| {
        value
            .chars()
            .any(|c| matches!(c, '$' | '€' | '£' | '₩' | '¥'))
            && value.chars().any(|c| c.is_ascii_digit())
    };
    let forms = scope
        .select("form")
        .iter()
        .filter(|form| {
            let priced_option = form.select("label").iter().any(|label| {
                label.select("input[type='radio']").length() == 1 && has_price(&label.text())
            }) || form
                .select("select option")
                .iter()
                .any(|option| has_price(&option.text()));
            priced_option
                && form
                    .attr("method")
                    .is_some_and(|m| m.eq_ignore_ascii_case("post"))
                && form
                    .attr("action")
                    .and_then(|a| base.join(&a).ok())
                    .is_some_and(|u| identities.contains(&u))
        })
        .collect::<Vec<_>>();
    if forms.len() != 1 {
        page.limited |= forms.len() > 1;
        return;
    }
    let form = &forms[0];
    let labels = form
        .select("label")
        .iter()
        .filter(|label| label.select("input[type='radio']").length() == 1)
        .chain(form.select("select option").iter())
        .collect::<Vec<_>>();
    page.limited |= labels.len() > 8;
    for (i, label) in labels.into_iter().take(8).enumerate() {
        let raw = label.text().to_string();
        if !raw
            .chars()
            .any(|c| matches!(c, '$' | '€' | '£' | '₩' | '¥'))
            || !raw.chars().any(|c| c.is_ascii_digit())
        {
            continue;
        }
        push(
            &mut page.sale,
            "구매 옵션",
            &raw,
            &format!("detail:purchase-option[{i}]"),
            &mut page.limited,
        );
    }
}
pub(super) fn parse(url: &str, html: &str) -> CatalogPage {
    let mut page = CatalogPage {
        kind: "page",
        ..Default::default()
    };
    let Ok(base) = Url::parse(url) else {
        return page;
    };
    let doc = Document::from(html);
    let mut identities = vec![base.clone()];
    let canonical = doc.select("head link[rel=canonical]");
    if canonical.length() == 1 {
        if let Some(v) = canonical
            .attr("href")
            .and_then(|v| base.join(&v).ok())
            .filter(|v| v.origin() == base.origin())
        {
            identities.push(v);
        }
    }
    let mut json = Vec::new();
    let scripts = doc.select("script[type='application/ld+json']");
    page.limited |= scripts.length() > 8;
    for node in scripts.iter().take(8) {
        let value = node.text();
        if value.len() > 128 * 1024 {
            page.limited = true;
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&value) {
            json.push(v);
        }
    }
    let mut items = Vec::new();
    for (i, value) in json.iter().enumerate() {
        structured_items(value, format!("json-ld[{i}]"), 0, &mut items);
    }
    page.limited |= items.len() >= 32;
    let matched = items
        .iter()
        .filter(|(_, value)| {
            ["url", "@id", "mainEntityOfPage"]
                .iter()
                .filter_map(|k| value.get(k))
                .filter_map(identity)
                .any(|v| {
                    base.join(v).ok().is_some_and(|mut u| {
                        u.set_fragment(None);
                        identities.iter().any(|v| v.as_str() == u.as_str())
                    })
                })
        })
        .collect::<Vec<_>>();
    let selected = if matched.len() == 1 {
        Some(matched[0])
    } else if matched.is_empty()
        && items.len() == 1
        && ["url", "@id", "mainEntityOfPage"]
            .iter()
            .all(|k| items[0].1.get(k).is_none())
    {
        Some(&items[0])
    } else {
        page.ambiguous = !items.is_empty();
        None
    };
    if let Some((path, value)) = selected {
        page.kind = match declared_work_label(value) {
            Some("작품명") => "work",
            Some("상품명") => "product",
            Some("업로드 제목") => "upload",
            _ => "page",
        };
        if let Some((key, title)) = value
            .get("name")
            .and_then(Value::as_str)
            .map(|title| ("name", title))
            .or_else(|| {
                value
                    .get("headline")
                    .and_then(Value::as_str)
                    .map(|title| ("headline", title))
            })
        {
            let (display, cut) = tidy(title, 512);
            page.limited |= cut;
            page.title = Some(display).filter(|value| !value.is_empty());
            if let Some(label) = declared_work_label(value) {
                page.title_evidence =
                    literal_title(title, label, &format!("{path}.{key}"), &mut page.limited);
            }
        }
        for (key, label) in CODES {
            if let Some(v) = value.get(key).and_then(scalar) {
                push(
                    &mut page.codes,
                    label,
                    &v,
                    &format!("{path}.{key}"),
                    &mut page.limited,
                );
            }
        }
        if declared_work_label(value).is_some() {
            if let Some(description) = value.get("description").and_then(Value::as_str) {
                description_codes(&mut page, description, &format!("{path}.description"));
            }
        }
        if let Some(v) = value.get("identifier") {
            let mut fields = super::page_evidence::PageEvidence::empty(url);
            identifiers(&mut fields, v, &format!("{path}.identifier"));
            page.limited |= fields.limited;
            for field in fields.fields {
                push(
                    &mut page.codes,
                    &field.label,
                    &field.value,
                    &field.source,
                    &mut page.limited,
                );
            }
        }
        if let Some(v) = value.get("offers") {
            offers(&mut page, v, &format!("{path}.offers"), &base);
        }
        if let Some(properties) = value.get("additionalProperty").and_then(Value::as_array) {
            page.limited |= properties.len() > 24;
            for (index, property) in properties.iter().take(24).enumerate() {
                if property.get("@type").and_then(Value::as_str) != Some("PropertyValue") {
                    continue;
                }
                if let Some((label, value)) = property
                    .get("name")
                    .and_then(Value::as_str)
                    .and_then(detail_label)
                    .zip(property.get("value").and_then(scalar))
                {
                    push(
                        &mut page.codes,
                        label,
                        &value,
                        &format!("{path}.additionalProperty[{index}].value"),
                        &mut page.limited,
                    );
                }
            }
        }
        // These are source-published references; they are never identity proof.
        for key in ["sameAs", "isBasedOn", "encodesCreativeWork"] {
            if let Some(v) = value.get(key) {
                for item in v
                    .as_array()
                    .map(|a| a.iter().take(4).collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![v])
                {
                    if let Some(raw) = item.as_str().or_else(|| identity(item)) {
                        link(
                            &mut page,
                            &base,
                            raw,
                            match key {
                                "sameAs" => "identity",
                                "encodesCreativeWork" => "source",
                                _ => "derived",
                            },
                        );
                    }
                }
            }
        }
    }
    if !page.ambiguous {
        let retailer = doc.select("head meta[property='product:retailer_item_id']");
        if retailer.length() == 1 {
            if let Some(value) = retailer.attr("content") {
                push(
                    &mut page.codes,
                    "판매처 등록 ID",
                    &value,
                    "meta:product:retailer_item_id",
                    &mut page.limited,
                );
            }
        } else {
            page.limited |= retailer.length() > 1;
        }
        if page.title.is_none() && !page.codes.is_empty() {
            let title = doc.select("head meta[property='og:title']");
            if title.length() == 1 {
                if let Some(value) = title.attr("content") {
                    let (title, cut) = tidy(&value, 512);
                    page.limited |= cut;
                    page.title = Some(title);
                }
            }
        }
    }
    doc.select("script,style,template,noscript,svg,nav,aside,footer,blockquote,pre,code,[hidden],[inert],[aria-hidden='true'],[style*='display:none'],[style*='display: none'],[style*='visibility:hidden'],[style*='visibility: hidden'],[class*='recommend'],[id*='recommend'],[class*='related'],[id*='related'],[class*='ranking']").remove();
    let primary=doc.select("[itemscope][itemtype='https://schema.org/Product'],[itemscope][itemtype='http://schema.org/Product'],[itemscope][itemtype='https://schema.org/Book'],[itemscope][itemtype='http://schema.org/Book']");
    let main = doc.select("main,[role=main]");
    let article = doc.select("article");
    let scope = if primary.length() == 1 {
        Some(primary)
    } else if primary.length() > 1 {
        page.ambiguous = true;
        None
    } else if main.length() == 1 {
        Some(main)
    } else if main.length() == 0 && article.length() == 1 {
        Some(article)
    } else if main.length() == 0 && article.length() == 0 && doc.select("h1").length() == 1 {
        Some(doc.select("body"))
    } else {
        None
    };
    if let Some(scope) = scope {
        // A timeline/list main is not one work's detail body. Preserve any
        // explicitly selected structured facts, but never borrow another
        // article's identifier or purchase link into this page's work.
        if scope.select("article").length() > 1 {
            page.limited = true;
            return page;
        }
        if !page.ambiguous {
            // A literal name in the current primary typed item is work
            // evidence; an ordinary heading or a nested item's name is not.
            if page.title_evidence.is_none()
                && selected.is_none_or(|(_, value)| declared_work_label(value).is_some())
            {
                let label = match scope.attr("itemtype").as_deref() {
                    Some("https://schema.org/Book" | "http://schema.org/Book") => Some("작품명"),
                    Some("https://schema.org/Product" | "http://schema.org/Product") => {
                        Some("상품명")
                    }
                    _ => None,
                };
                if let Some(label) = label {
                    let current = scope.attr("itemid").is_none_or(|raw| {
                        base.join(&raw).ok().is_some_and(|mut item| {
                            item.set_fragment(None);
                            identities.iter().any(|identity| *identity == item)
                        })
                    });
                    if current {
                        let names = scope
                            .select("[itemprop~='name']")
                            .iter()
                            .filter(|node| !node.is("[itemscope]") && own_item(&scope, node))
                            .collect::<Vec<_>>();
                        page.limited |= names.len() > 1;
                        if names.len() == 1 {
                            if let Some(evidence) = literal_title(
                                &text(&names[0]),
                                label,
                                "microdata:name",
                                &mut page.limited,
                            ) {
                                page.title = Some(evidence.value.clone());
                                page.kind = if label == "상품명" {
                                    "product"
                                } else {
                                    "work"
                                };
                                page.title_evidence = Some(evidence);
                            }
                        }
                    } else {
                        page.limited = true;
                    }
                }
            }
            if page.title.is_none() {
                let name = scope.select("h1");
                if name.length() == 1 {
                    page.title = Some(tidy(&name.text(), 512).0);
                }
            }
            if scope.attr("itemscope").is_some() {
                for node in scope.select("[itemprop]").iter().take(128) {
                    // Nested item scopes have their own identity. Skip them.
                    if node.is("[itemscope]") || !own_item(&scope, &node) {
                        continue;
                    }
                    if let Some(key) = node.attr("itemprop") {
                        if let Some((_, label)) = CODES.iter().find(|(v, _)| *v == key.as_ref()) {
                            push(
                                &mut page.codes,
                                label,
                                &text(&node),
                                &format!("microdata:{key}"),
                                &mut page.limited,
                            );
                        }
                    }
                }
            }
            for (i, row) in scope.select("tr").iter().take(128).enumerate() {
                let cells = row.children().filter("th,td");
                if cells.length() != 2 {
                    continue;
                }
                let mut values = cells.iter();
                let label = values.next().unwrap();
                let value = values.next().unwrap();
                if let Some(label) = detail_label(&label.text()) {
                    push(
                        &mut page.codes,
                        label,
                        &value.text(),
                        &format!("detail:tr[{i}]"),
                        &mut page.limited,
                    );
                }
            }
            for (i, label) in scope.select("dt").iter().take(128).enumerate() {
                let value = label.next_sibling();
                if value.is("dd") {
                    if let Some(label) = detail_label(&label.text()) {
                        push(
                            &mut page.codes,
                            label,
                            &value.text(),
                            &format!("detail:dt[{i}]"),
                            &mut page.limited,
                        );
                    }
                }
            }
            // Common CMS fields expose a literal label beside a value rather
            // than a table. Require the exact code label and one value group.
            for (i, label) in scope.select(".field-label").iter().take(128).enumerate() {
                let value = label.next_sibling();
                if value.is(".field-items,.field-value")
                    && value.select(".field-item").length() <= 1
                {
                    if let Some(label) = detail_label(&label.text()) {
                        push(
                            &mut page.codes,
                            label,
                            &value.text(),
                            &format!("detail:field[{i}]"),
                            &mut page.limited,
                        );
                    }
                }
            }
            for (i, node) in scope.select("p,li").iter().take(128).enumerate() {
                let line = node.text();
                if line.len() > 512 {
                    continue;
                }
                if let Some((label, value)) = line.split_once([':', '：']) {
                    if let Some(label) = detail_label(label) {
                        push(
                            &mut page.codes,
                            label,
                            value,
                            &format!("detail:line[{i}]"),
                            &mut page.limited,
                        );
                    }
                }
            }
        }
        for anchor in scope.select("a[href]").iter().take(256) {
            let label = tidy(&anchor.text(), 160).0.to_ascii_lowercase();
            let published = matches!(
                label.as_str(),
                "buy"
                    | "buy now"
                    | "purchase"
                    | "shop"
                    | "official store"
                    | "source"
                    | "original"
                    | "購入"
                    | "購入はこちら"
                    | "販売ページ"
                    | "販売サイト"
                    | "公式ストア"
                    | "作品ページ"
                    | "配信ページ"
                    | "원본"
                    | "출처"
                    | "판매 페이지"
                    | "구매"
                    | "구매하기"
            ) || ["buy at ", "buy from ", "購入はこちら ", "販売ページ "]
                .iter()
                .any(|v| label.starts_with(v));
            if published {
                if let Some(raw) = anchor.attr("href") {
                    let relation = if matches!(
                        label.as_str(),
                        "source" | "original" | "원본" | "출처" | "作品ページ"
                    ) {
                        "source"
                    } else {
                        "purchase"
                    };
                    link(&mut page, &base, &raw, relation);
                }
            }
        }
        if product_price_metadata(&mut page, &doc, &identities) {
            purchase_options(&mut page, &scope, &base, &identities);
        }
    } else if selected.is_none() {
        page.limited = true;
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    fn priced_book() -> &'static str {
        r#"<head><meta property="og:type" content="article"><meta property="og:url" content="https://publisher.example/book"><meta property="og:title" content="Exact Book"><meta property="product:isbn" content="9781718504448"><meta property="product:price:amount" content="59.99"><meta property="product:price:currency" content="USD"></head><main><h1>Exact Book</h1><dl><dt>ISBN</dt><dd>9781718504448</dd></dl><form method="post" action="/book"><label><input type="radio" name="format" value="1">Print and ebook, $59.99</label><label><input type="radio" name="format" value="2">Ebook, $47.99</label></form></main>"#
    }
    #[test]
    fn explicit_same_product_price_and_purchase_options_keep_their_own_labels() {
        let p = parse("https://publisher.example/book", priced_book());
        assert!(p
            .sale
            .iter()
            .any(|f| f.source == "meta:product:price:amount" && f.value == "59.99"));
        assert!(p.sale.iter().any(|f| f.label == "통화" && f.value == "USD"));
        assert!(p
            .sale
            .iter()
            .any(|f| f.label == "구매 옵션" && f.value == "Print and ebook, $59.99"));
        assert!(p
            .sale
            .iter()
            .any(|f| f.label == "구매 옵션" && f.value == "Ebook, $47.99"));
        assert!(
            p.links.is_empty(),
            "a form action is never a navigation/crawl link"
        );
    }
    #[test]
    fn a_same_page_site_search_form_does_not_hide_the_unique_purchase_options() {
        let html=priced_book().replace("<main>","<main><form method=\"post\" action=\"/book\"><input type=\"text\" name=\"search\"><button>Search</button></form>");
        let p = parse("https://publisher.example/book", &html);
        assert_eq!(p.sale.iter().filter(|f| f.label == "구매 옵션").count(), 2);
    }
    #[test]
    fn price_metadata_requires_unique_matching_page_and_product_identity() {
        for html in [
            priced_book().replace("content=\"Exact Book\"", "content=\"Other product\""),
            priced_book().replace("content=\"9781718504448\"", "content=\"9781718500440\""),
            priced_book().replace(
                "https://publisher.example/book",
                "https://publisher.example/other",
            ),
            priced_book().replace(
                "</head>",
                "<meta property=\"product:price:amount\" content=\"12\"></head>",
            ),
        ] {
            assert!(parse("https://publisher.example/book", &html)
                .sale
                .is_empty());
        }
        let p = parse(
            "https://publisher.example/book",
            &priced_book().replace("action=\"/book\"", "action=\"/checkout\""),
        );
        assert!(!p.sale.iter().any(|f| f.label == "구매 옵션"));
        let p = parse(
            "https://publisher.example/book",
            &priced_book().replace("<label>", "<label hidden>"),
        );
        assert!(!p.sale.iter().any(|f| f.label == "구매 옵션"));
    }
    #[test]
    fn primary_labelled_description_keeps_literal_product_code_and_retailer_role() {
        let p = parse(
            "https://example.com/products/4494426",
            r#"<meta property="product:retailer_item_id" content="Example Company"><script type="application/ld+json">{"@type":"Product","url":"https://example.com/products/4494426","name":"Declared cardigan","sku":"4494426","description":"제품분류 : 아우터 > 카디건 브랜드 : Example 제품번호 : BM243-KN001 제품 : Declared cardigan - 76,330"}</script>"#,
        );
        let code = p
            .codes
            .iter()
            .find(|field| field.value == "BM243-KN001")
            .expect("explicit product number in primary description");
        assert_eq!(code.label, "품번");
        assert_eq!(code.source, "json-ld[0].description:제품번호");
        let retailer = p
            .codes
            .iter()
            .find(|field| field.value == "Example Company")
            .unwrap();
        assert_eq!(retailer.label, "판매처 등록 ID");
        assert_eq!(retailer.source, "meta:product:retailer_item_id");
        assert!(!p
            .codes
            .iter()
            .any(|field| field.value == "Example" || field.value == "아우터 > 카디건"));
    }
    #[test]
    fn description_code_fields_require_unambiguous_primary_field_list() {
        for description in [
            "A review mentioning SKU: OTHER-0004",
            "Title: A review mentioning SKU: OTHER-0004",
            "Title: Book\nA review mentioning SKU: OTHER-0004",
            "SKU: not assigned",
            "SKU: ABC-0004 unknown: trailing text",
            "SKU: ABC-0004 SKU: OTHER-0004",
            "<p>SKU: ABC-0004</p>",
            "SKU: ABC-0004\nSome unrelated prose",
        ] {
            let value = serde_json::json!({"@type":"Product","description":description});
            let html = format!("<script type=\"application/ld+json\">{value}</script>");
            assert!(
                parse("https://example.com/work", &html).codes.is_empty(),
                "unproved description: {description}"
            );
        }
        for value in [
            serde_json::json!({"@type":"Article","description":"SKU: OTHER-0004"}),
            serde_json::json!({"@type":"Product","url":"https://example.com/other","description":"SKU: OTHER-0004"}),
            serde_json::json!([{"@type":"Product","description":"SKU: OTHER-0004"},{"@type":"Product","name":"Other"}]),
        ] {
            let html = format!("<script type=\"application/ld+json\">{value}</script>");
            assert!(parse("https://example.com/work", &html).codes.is_empty());
        }
    }
    #[test]
    fn description_codes_preserve_leading_zeros_and_refuse_cut_values() {
        let value = serde_json::json!({"@type":"Book","description":"Title: Literal Book\nISBN-13: 978-1-71850-444-8\nSKU: 00042\nPrice: 10.00"});
        let p = parse(
            "https://example.com/work",
            &format!("<script type=\"application/ld+json\">{value}</script>"),
        );
        assert_eq!(
            p.codes.iter().map(|v| v.value.as_str()).collect::<Vec<_>>(),
            ["978-1-71850-444-8", "00042"]
        );
        for description in [
            format!("SKU: {}", "X".repeat(257)),
            format!("SKU: {}", "X".repeat(4097)),
        ] {
            let value = serde_json::json!({"@type":"Product","description":description});
            let p = parse(
                "https://example.com/work",
                &format!("<script type=\"application/ld+json\">{value}</script>"),
            );
            assert!(p.codes.is_empty());
            assert!(p.limited);
        }
    }
    #[test]
    fn declared_primary_title_keeps_the_exact_field_and_does_not_use_the_page_heading() {
        let p = parse(
            "https://example.com/work",
            r#"<script type="application/ld+json">{"@type":"WebPage","mainEntity":{"@type":"Movie","url":"https://example.com/work","name":null,"headline":"Literal & Original <Title>"},"related":{"@type":"Movie","name":"Other work"}}</script><main><h1>Generic page heading</h1></main>"#,
        );
        let title = p.title_evidence.unwrap();
        assert_eq!(title.value, "Literal & Original <Title>");
        assert_eq!(title.source, "json-ld[0].mainEntity.headline");
        assert_eq!(title.label, "작품명");
        assert_eq!(p.title.as_deref(), Some(title.value.as_str()));
        assert!(p.codes.is_empty());
    }
    #[test]
    fn microdata_title_belongs_to_the_current_unique_item_not_a_nested_author() {
        let p = parse(
            "https://example.com/work",
            r##"<main itemscope itemtype="https://schema.org/Book" itemid="#book"><h1>Generic page heading</h1><meta itemprop="name" content="A Literal Book Title"><div itemprop="author" itemscope itemtype="https://schema.org/Person"><span itemprop="name">Another name</span></div></main>"##,
        );
        assert_eq!(p.title.as_deref(), Some("A Literal Book Title"));
        let title = p.title_evidence.unwrap();
        assert_eq!(title.value, "A Literal Book Title");
        assert_eq!(title.source, "microdata:name");
    }
    #[test]
    fn generic_articles_wrong_targets_and_ambiguous_entities_are_not_declared_work_titles() {
        for html in [
            r#"<meta property="og:title" content="Unproved heading"><main><h1>Unproved heading</h1></main>"#,
            r#"<script type="application/ld+json">{"@type":"Article","name":"News heading"}</script><main itemscope itemtype="https://schema.org/Book"><span itemprop="name">A referenced book</span></main>"#,
            r#"<script type="application/ld+json">{"@type":"NewsArticle","headline":"News heading"}</script>"#,
            r#"<script type="application/ld+json">{"@type":"Movie","url":"https://example.com/other","name":"Other film"}</script>"#,
            r#"<script type="application/ld+json">[{"@type":"Book","name":"Book A"},{"@type":"Book","name":"Book B"}]</script>"#,
            r#"<main itemscope itemtype="https://schema.org/Book" itemid="/other"><span itemprop="name">Other book</span></main>"#,
            r#"<main itemscope itemtype="https://schema.org/Book"><span itemprop="name">Book A</span><span itemprop="name">Book B</span></main>"#,
            r#"<main><article><h1>First post</h1></article><article><h1>Second post</h1></article></main>"#,
        ] {
            let p = parse("https://example.com/work", html);
            assert!(
                p.title_evidence.is_none(),
                "unproved work title admitted: {html}"
            );
        }
    }
    #[test]
    fn truncated_or_over_limit_unicode_names_are_not_presented_as_exact_title_evidence() {
        for title in ["X".repeat(513), "🌷".repeat(257)] {
            let html = serde_json::json!({"@type":"Book", "name":title});
            let p = parse(
                "https://example.com/work",
                &format!("<script type=\"application/ld+json\">{html}</script>"),
            );
            assert!(p.title_evidence.is_none());
            assert!(p.limited);
        }
    }
    #[test]
    fn explicit_primary_product_codes_and_offers_keep_values_and_provenance() {
        let html = r#"<script type="application/ld+json">{"@type":"Product","url":"https://example.com/work","name":"Declared Work","sku":"00123","mpn":"CAT-0042","offers":{"price":"12.00","priceCurrency":"JPY","availability":"https://schema.org/InStock","url":"https://shop.example.com/item"},"identifier":{"@type":"PropertyValue","name":"catalogue","value":"0008"}}</script>"#;
        let p = parse("https://example.com/work", html);
        assert_eq!(p.title.as_deref(), Some("Declared Work"));
        assert_eq!(
            p.codes.iter().map(|v| v.value.as_str()).collect::<Vec<_>>(),
            ["00123", "CAT-0042", "0008"]
        );
        assert!(p.codes[2].source.ends_with("identifier.value"));
        assert_eq!(p.sale.len(), 3);
        assert_eq!(p.links, ["https://shop.example.com/item"]);
    }
    #[test]
    fn labelled_rows_are_work_evidence_without_guessing_url_codes() {
        let p = parse(
            "https://example.com/RJ999999",
            r#"<main><h1>Title</h1><dl><dt>品番：</dt><dd>ABC-0004</dd><dt>ISBN-13</dt><dd>978-1-234-56789-0</dd></dl><table><tr><th>商品番号</th><td>00042</td></tr></table><aside><table><tr><th>品番</th><td>NOT-PRIMARY</td></tr></table></aside><blockquote><dl><dt>品番</dt><dd>QUOTE</dd></dl></blockquote><a href="/buy/explicit">購入</a><a href="/cart">Buy</a><a href="/images/pic.jpg">Source</a></main>"#,
        );
        assert_eq!(p.codes.len(), 3);
        assert!(!p
            .codes
            .iter()
            .any(|v| ["RJ999999", "NOT-PRIMARY", "QUOTE"].contains(&v.value.as_str())));
        assert_eq!(p.links, ["https://example.com/buy/explicit"]);
    }
    #[test]
    fn unrelated_and_ambiguous_products_do_not_become_work_identity() {
        for html in [
            r#"<script type="application/ld+json">[{"@type":"Product","name":"A","sku":"A1"},{"@type":"Product","name":"B","sku":"B1"}]</script><main><h1>All products</h1><dl><dt>品番</dt><dd>UNKNOWN</dd></dl></main>"#,
            r#"<script type="application/ld+json">{"@type":"Product","url":"https://example.com/other","sku":"OTHER"}</script>"#,
        ] {
            let p = parse("https://example.com/current", html);
            assert!(p.codes.is_empty());
        }
        let p = parse(
            "https://example.com/current",
            r#"<script type="application/ld+json">{"@type":"WebPage","mainEntity":{"@type":"Book","name":"Primary","isbn":"12345"},"related":{"@type":"Product","sku":"UNRELATED"}}</script>"#,
        );
        assert_eq!(p.codes.len(), 1);
        assert_eq!(p.codes[0].value, "12345");
    }
    #[test]
    fn arbitrary_numbers_and_url_slugs_are_never_identifiers() {
        let p = parse(
            "https://example.com/work/CODE-123",
            r#"<main><h1>Post 2026</h1><p>Score 95.5% #CODE-123</p><dl><dt>Views</dt><dd>99999</dd></dl><a href="javascript:alert(1)">購入</a></main>"#,
        );
        assert!(p.codes.is_empty());
        assert!(p.links.is_empty());
    }
    #[test]
    fn timeline_articles_do_not_supply_another_posts_code_or_buy_link() {
        let p = parse(
            "https://example.com/source",
            r#"<main><article><h1>Selected post</h1></article><article><p>品番: OTHER-0042</p><a href="https://shop.example.com/unrelated">購入</a></article></main>"#,
        );
        assert!(p.codes.is_empty());
        assert!(p.links.is_empty());
        assert!(p.limited);
    }
    #[test]
    fn labelled_cms_fields_and_lines_preserve_codes_and_never_truncate_an_identifier() {
        let p = parse(
            "https://example.com/work",
            &format!(
                r#"<main><h1>Book</h1><div class="field"><div class="field-label">ISBN-13:&nbsp;</div><div class="field-items"><div class="field-item">9781718504448</div></div></div><p>品番： ABC-0042</p><p>SKU: {}</p></main>"#,
                "X".repeat(300)
            ),
        );
        assert_eq!(p.codes.len(), 2);
        assert_eq!(p.codes[0].value, "9781718504448");
        assert_eq!(p.codes[1].value, "ABC-0042");
        assert!(p.limited);
    }
    #[test]
    fn retailer_metadata_and_typed_properties_keep_literal_values_without_promoting_color_or_generic_id(
    ) {
        let p = parse(
            "https://example.com/work",
            r#"<meta property="og:title" content="Declared Work"><meta property="product:retailer_item_id" content="RETAIL-0004"><script type="application/ld+json">{"@type":"Product","additionalProperty":[{"@type":"PropertyValue","name":"メーカー品番","value":"ABC-0004"},{"@type":"PropertyValue","name":"color","value":"red"},{"name":"SKU","value":"UNTYPED"}]}</script><main><h1>Work</h1><p>품번: KO-0042</p><p>商品ID: 00123</p><p>ID: UNKNOWN</p></main>"#,
        );
        assert_eq!(p.title.as_deref(), Some("Declared Work"));
        assert!(p
            .codes
            .iter()
            .any(|v| v.value == "ABC-0004" && v.source.ends_with("additionalProperty[0].value")));
        assert!(p.codes.iter().any(|v| v.value == "RETAIL-0004"));
        assert!(p.codes.iter().any(|v| v.value == "KO-0042"));
        assert!(p.codes.iter().any(|v| v.value == "00123"));
        assert!(!p
            .codes
            .iter()
            .any(|v| matches!(v.value.as_str(), "red" | "UNTYPED" | "UNKNOWN")));
    }
}
