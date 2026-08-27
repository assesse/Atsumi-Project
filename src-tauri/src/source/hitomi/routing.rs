use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::source::SourceContractError;

use super::galleryinfo::validate_file_hash;
use super::model::{HitomiPageFile, SourceRevision, HITOMI_CONTENT_DOMAIN};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GgRoutingTable {
    base_path: String,
    default_route: u8,
    overrides: BTreeMap<u16, u8>,
}

impl GgRoutingTable {
    pub fn base_path(&self) -> &str {
        &self.base_path
    }

    pub const fn default_route(&self) -> u8 {
        self.default_route
    }

    pub fn overrides(&self) -> &BTreeMap<u16, u8> {
        &self.overrides
    }

    pub fn route_for_hash(&self, hash: &str) -> Result<u8, SourceContractError> {
        let key = route_key(hash)?;
        Ok(self
            .overrides
            .get(&key)
            .copied()
            .unwrap_or(self.default_route))
    }
}

pub fn parse_gg_routing(script: &str) -> Result<GgRoutingTable, SourceContractError> {
    if script.trim().is_empty() {
        return Err(SourceContractError::protocol("gg.js response is empty"));
    }

    let base_path = parse_base_path(script)?;
    let (switch_start, switch_body) = parse_switch_body(script)?;
    let default_route = parse_default_route(&script[..switch_start])?;
    let overrides = parse_route_overrides(switch_body)?;

    Ok(GgRoutingTable {
        base_path,
        default_route,
        overrides,
    })
}

fn parse_base_path(script: &str) -> Result<String, SourceContractError> {
    let bytes = script.as_bytes();
    let mut cursor = 0;
    while let Some(index) = find_identifier(script, "b", cursor) {
        cursor = index + 1;
        let mut value_start = skip_ascii_whitespace(bytes, cursor);
        if bytes.get(value_start) != Some(&b':') {
            continue;
        }
        value_start = skip_ascii_whitespace(bytes, value_start + 1);
        let Some(quote @ (b'\'' | b'"')) = bytes.get(value_start).copied() else {
            return Err(SourceContractError::protocol(
                "gg.js base path must be a quoted string",
            ));
        };
        let content_start = value_start + 1;
        let Some(relative_end) = bytes[content_start..]
            .iter()
            .position(|byte| *byte == quote)
        else {
            return Err(SourceContractError::protocol(
                "gg.js base path string is not terminated",
            ));
        };
        return normalize_base_path(&script[content_start..content_start + relative_end]);
    }

    Err(SourceContractError::protocol(
        "gg.js base path property was not found",
    ))
}

fn normalize_base_path(value: &str) -> Result<String, SourceContractError> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() || value.len() > 128 {
        return Err(SourceContractError::invalid_data(
            "gg.js base path",
            "must contain between 1 and 128 characters",
        ));
    }
    if value.starts_with('/')
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_'))
    {
        return Err(SourceContractError::invalid_data(
            "gg.js base path",
            "contains an unsafe path segment",
        ));
    }
    Ok(format!("{value}/"))
}

fn parse_switch_body(script: &str) -> Result<(usize, &str), SourceContractError> {
    let switch_start = find_identifier(script, "switch", 0)
        .ok_or_else(|| SourceContractError::protocol("gg.js route switch was not found"))?;
    let bytes = script.as_bytes();
    let mut cursor = skip_ascii_whitespace(bytes, switch_start + "switch".len());
    if bytes.get(cursor) != Some(&b'(') {
        return Err(SourceContractError::protocol(
            "gg.js route switch is missing its selector",
        ));
    }
    cursor = skip_ascii_whitespace(bytes, cursor + 1);
    if bytes.get(cursor) != Some(&b'g') {
        return Err(SourceContractError::protocol(
            "gg.js route switch selector must be g",
        ));
    }
    cursor = skip_ascii_whitespace(bytes, cursor + 1);
    if bytes.get(cursor) != Some(&b')') {
        return Err(SourceContractError::protocol(
            "gg.js route switch selector is malformed",
        ));
    }
    cursor = skip_ascii_whitespace(bytes, cursor + 1);
    if bytes.get(cursor) != Some(&b'{') {
        return Err(SourceContractError::protocol(
            "gg.js route switch body was not found",
        ));
    }
    let body_start = cursor + 1;
    let mut depth = 1_u32;
    let mut quote = None;
    let mut escaped = false;
    for index in body_start..bytes.len() {
        let byte = bytes[index];
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'{' => depth = depth.saturating_add(1),
            b'}' => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    SourceContractError::protocol("gg.js route switch has unmatched braces")
                })?;
                if depth == 0 {
                    return Ok((switch_start, &script[body_start..index]));
                }
            }
            _ => {}
        }
    }

    Err(SourceContractError::protocol(
        "gg.js route switch body is not terminated",
    ))
}

fn parse_default_route(prefix: &str) -> Result<u8, SourceContractError> {
    let bytes = prefix.as_bytes();
    let mut cursor = 0;
    let mut result = None;
    while cursor < bytes.len() {
        let Some(relative) = prefix[cursor..].find('o') else {
            break;
        };
        let index = cursor + relative;
        cursor = index + 1;
        if !identifier_boundary(bytes, index, index + 1) {
            continue;
        }
        let mut assignment = skip_ascii_whitespace(bytes, cursor);
        if bytes.get(assignment) != Some(&b'=') {
            continue;
        }
        assignment = skip_ascii_whitespace(bytes, assignment + 1);
        if let Some(route) = parse_binary_route(bytes.get(assignment).copied()) {
            result = Some(route);
        }
    }
    result.ok_or_else(|| SourceContractError::protocol("gg.js default route was not found"))
}

fn parse_route_overrides(body: &str) -> Result<BTreeMap<u16, u8>, SourceContractError> {
    let mut overrides = BTreeMap::new();
    let mut block_start = 0;
    let mut saw_case = false;
    while block_start < body.len() {
        let next_break = find_identifier(body, "break", block_start);
        let block_end = next_break.unwrap_or(body.len());
        let block = &body[block_start..block_end];
        let cases = parse_case_values(block)?;
        if !cases.is_empty() {
            saw_case = true;
            let route = parse_block_route(block)?.ok_or_else(|| {
                SourceContractError::protocol("gg.js case block has no binary route assignment")
            })?;
            for case in cases {
                if let Some(previous) = overrides.insert(case, route) {
                    if previous != route {
                        return Err(SourceContractError::invalid_data(
                            "gg.js route table",
                            format!("case {case} is assigned to conflicting routes"),
                        ));
                    }
                }
            }
        }
        let Some(next_break) = next_break else {
            break;
        };
        block_start = next_break + "break".len();
    }

    if !saw_case {
        return Err(SourceContractError::protocol(
            "gg.js route switch contains no cases",
        ));
    }
    Ok(overrides)
}

fn parse_case_values(block: &str) -> Result<Vec<u16>, SourceContractError> {
    let bytes = block.as_bytes();
    let mut cases = Vec::new();
    let mut cursor = 0;
    while let Some(index) = find_identifier(block, "case", cursor) {
        cursor = index + "case".len();
        let number_start = skip_ascii_whitespace(bytes, cursor);
        let number_end = bytes[number_start..]
            .iter()
            .position(|byte| !byte.is_ascii_digit())
            .map_or(bytes.len(), |offset| number_start + offset);
        if number_start == number_end {
            return Err(SourceContractError::protocol(
                "gg.js case is missing a numeric route key",
            ));
        }
        let delimiter = skip_ascii_whitespace(bytes, number_end);
        if bytes.get(delimiter) != Some(&b':') {
            return Err(SourceContractError::protocol(
                "gg.js case is missing its colon",
            ));
        }
        let value = block[number_start..number_end]
            .parse::<u16>()
            .map_err(|_| {
                SourceContractError::invalid_data(
                    "gg.js route key",
                    "must fit in an unsigned 12-bit integer",
                )
            })?;
        if value > 0x0fff {
            return Err(SourceContractError::invalid_data(
                "gg.js route key",
                format!("case {value} exceeds the 12-bit hash suffix space"),
            ));
        }
        cases.push(value);
        cursor = delimiter + 1;
    }
    Ok(cases)
}

fn parse_block_route(block: &str) -> Result<Option<u8>, SourceContractError> {
    let bytes = block.as_bytes();
    let mut cursor = 0;
    let mut route = None;
    while cursor < bytes.len() {
        let Some(relative) = block[cursor..].find('o') else {
            break;
        };
        let index = cursor + relative;
        cursor = index + 1;
        if !identifier_boundary(bytes, index, index + 1) {
            continue;
        }
        let mut assignment = skip_ascii_whitespace(bytes, cursor);
        if bytes.get(assignment) != Some(&b'=') {
            continue;
        }
        assignment = skip_ascii_whitespace(bytes, assignment + 1);
        let parsed = parse_binary_route(bytes.get(assignment).copied()).ok_or_else(|| {
            SourceContractError::invalid_data(
                "gg.js route assignment",
                "route must be binary (0 or 1)",
            )
        })?;
        if route
            .replace(parsed)
            .is_some_and(|previous| previous != parsed)
        {
            return Err(SourceContractError::invalid_data(
                "gg.js case block",
                "contains conflicting route assignments",
            ));
        }
    }
    Ok(route)
}

fn parse_binary_route(value: Option<u8>) -> Option<u8> {
    match value {
        Some(b'0') => Some(0),
        Some(b'1') => Some(1),
        _ => None,
    }
}

fn route_key(hash: &str) -> Result<u16, SourceContractError> {
    validate_file_hash(hash, "Hitomi file hash")?;
    let suffix = &hash[hash.len() - 3..];
    let rotated = format!("{}{}", &suffix[2..], &suffix[..2]);
    u16::from_str_radix(&rotated, 16)
        .map_err(|error| SourceContractError::invalid_data("Hitomi file hash", error.to_string()))
}

fn real_path(hash: &str) -> String {
    format!(
        "{}/{}/{}",
        &hash[hash.len() - 1..],
        &hash[hash.len() - 3..hash.len() - 1],
        hash
    )
}

fn full_path(hash: &str, routing: &GgRoutingTable) -> Result<String, SourceContractError> {
    Ok(format!(
        "{}{}/{}",
        routing.base_path,
        route_key(hash)?,
        hash
    ))
}

fn tn_host(route: u8) -> &'static str {
    if route == 0 {
        "atn"
    } else {
        "btn"
    }
}

fn webp_host(route: u8) -> &'static str {
    if route == 0 {
        "w1"
    } else {
        "w2"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThumbnailSize {
    Large,
    Small,
}

impl ThumbnailSize {
    const fn path(self) -> &'static str {
        match self {
            Self::Large => "webpbigtn",
            Self::Small => "webpsmalltn",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitomiImageKind {
    ThumbnailLarge,
    ThumbnailSmall,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitomiImageFormat {
    Webp,
    Jpeg,
    Png,
    Avif,
    Jxl,
}

impl HitomiImageFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Webp => "webp",
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Avif => "avif",
            Self::Jxl => "jxl",
        }
    }

    const fn content_type(self) -> &'static str {
        match self {
            Self::Webp => "image/webp",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Avif => "image/avif",
            Self::Jxl => "image/jxl",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitomiImageCandidate {
    pub url: String,
    pub kind: HitomiImageKind,
    pub content_type: String,
    pub format: HitomiImageFormat,
    pub source_revision: SourceRevision,
}

pub fn webp_thumbnail_candidates(
    file: &HitomiPageFile,
    routing: &GgRoutingTable,
    size: ThumbnailSize,
) -> Result<Vec<HitomiImageCandidate>, SourceContractError> {
    validate_file_hash(&file.hash, "Hitomi page file hash")?;
    if file.has_webp == Some(false) {
        return Ok(Vec::new());
    }
    let route = routing.route_for_hash(&file.hash)?;
    let real_path = real_path(&file.hash);
    let full_path = full_path(&file.hash, routing)?;
    let kind = match size {
        ThumbnailSize::Large => HitomiImageKind::ThumbnailLarge,
        ThumbnailSize::Small => HitomiImageKind::ThumbnailSmall,
    };
    Ok(deduplicate_candidates(vec![
        candidate(
            format!(
                "https://{}.{HITOMI_CONTENT_DOMAIN}/{}/{real_path}.webp",
                tn_host(route),
                size.path()
            ),
            kind,
            &file.source_revision,
        ),
        candidate(
            format!(
                "https://{}.{HITOMI_CONTENT_DOMAIN}/webp/{real_path}.webp",
                tn_host(route)
            ),
            kind,
            &file.source_revision,
        ),
        candidate(
            format!(
                "https://{}.{HITOMI_CONTENT_DOMAIN}/{full_path}.webp",
                webp_host(route)
            ),
            kind,
            &file.source_revision,
        ),
    ]))
}

pub fn webp_full_candidates(
    file: &HitomiPageFile,
    routing: &GgRoutingTable,
) -> Result<Vec<HitomiImageCandidate>, SourceContractError> {
    validate_file_hash(&file.hash, "Hitomi page file hash")?;
    if file.has_webp == Some(false) {
        return Ok(Vec::new());
    }
    let route = routing.route_for_hash(&file.hash)?;
    let real_path = real_path(&file.hash);
    let full_path = full_path(&file.hash, routing)?;
    let primary_host = webp_host(route);
    let mut urls = Vec::new();
    for host in [primary_host, "w1", "w2"] {
        urls.push(candidate(
            format!("https://{host}.{HITOMI_CONTENT_DOMAIN}/{full_path}.webp"),
            HitomiImageKind::Full,
            &file.source_revision,
        ));
        urls.push(candidate(
            format!("https://{host}.{HITOMI_CONTENT_DOMAIN}/webp/{real_path}.webp"),
            HitomiImageKind::Full,
            &file.source_revision,
        ));
    }
    Ok(deduplicate_candidates(urls))
}

/// Full-size candidates used by the artifact downloader. WebP derivatives are
/// preferred so already-valid WebP bytes can be preserved. The original JPEG
/// or PNG endpoints are retained as a conversion fallback.
pub fn download_full_candidates(
    file: &HitomiPageFile,
    routing: &GgRoutingTable,
) -> Result<Vec<HitomiImageCandidate>, SourceContractError> {
    validate_file_hash(&file.hash, "Hitomi page file hash")?;
    let mut candidates = webp_full_candidates(file, routing)?;
    let route = routing.route_for_hash(&file.hash)?;
    let real_path = real_path(&file.hash);
    let full_path = full_path(&file.hash, routing)?;
    if file.has_avif {
        for host in [webp_host(route), "a", "b"] {
            candidates.push(candidate_for_format(
                format!("https://{host}.{HITOMI_CONTENT_DOMAIN}/avif/{real_path}.avif"),
                HitomiImageKind::Full,
                HitomiImageFormat::Avif,
                &file.source_revision,
            ));
            candidates.push(candidate_for_format(
                format!("https://{host}.{HITOMI_CONTENT_DOMAIN}/{full_path}.avif"),
                HitomiImageKind::Full,
                HitomiImageFormat::Avif,
                &file.source_revision,
            ));
        }
    }
    if file.has_jxl {
        for host in ["a", "b"] {
            candidates.push(candidate_for_format(
                format!("https://{host}.{HITOMI_CONTENT_DOMAIN}/jxl/{real_path}.jxl"),
                HitomiImageKind::Full,
                HitomiImageFormat::Jxl,
                &file.source_revision,
            ));
        }
    }
    let extension = file
        .name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .ok_or_else(|| {
            SourceContractError::invalid_data(
                "Hitomi page filename",
                "does not contain a supported extension",
            )
        })?;
    let (extension, format) = match extension.as_str() {
        "jpg" | "jpeg" => (extension, HitomiImageFormat::Jpeg),
        "png" => (extension, HitomiImageFormat::Png),
        "webp" => (extension, HitomiImageFormat::Webp),
        "avif" if file.has_avif => (extension, HitomiImageFormat::Avif),
        "jxl" if file.has_jxl => (extension, HitomiImageFormat::Jxl),
        _ => {
            return if candidates.is_empty() {
                Err(SourceContractError::invalid_data(
                    "Hitomi page filename",
                    "does not identify a supported downloadable image format",
                ))
            } else {
                Ok(candidates)
            }
        }
    };
    for host in ["a", "b"] {
        candidates.push(candidate_for_format(
            format!("https://{host}.{HITOMI_CONTENT_DOMAIN}/images/{full_path}.{extension}"),
            HitomiImageKind::Full,
            format,
            &file.source_revision,
        ));
        candidates.push(candidate_for_format(
            format!("https://{host}.{HITOMI_CONTENT_DOMAIN}/{extension}/{real_path}.{extension}"),
            HitomiImageKind::Full,
            format,
            &file.source_revision,
        ));
    }
    Ok(deduplicate_candidates(candidates))
}

fn candidate(
    url: String,
    kind: HitomiImageKind,
    source_revision: &SourceRevision,
) -> HitomiImageCandidate {
    candidate_for_format(url, kind, HitomiImageFormat::Webp, source_revision)
}

fn candidate_for_format(
    url: String,
    kind: HitomiImageKind,
    format: HitomiImageFormat,
    source_revision: &SourceRevision,
) -> HitomiImageCandidate {
    HitomiImageCandidate {
        url,
        kind,
        content_type: format.content_type().to_owned(),
        format,
        source_revision: source_revision.clone(),
    }
}

fn deduplicate_candidates(candidates: Vec<HitomiImageCandidate>) -> Vec<HitomiImageCandidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.url.clone()))
        .collect()
}

fn find_identifier(text: &str, identifier: &str, mut cursor: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    while cursor <= text.len() {
        let relative = text[cursor..].find(identifier)?;
        let start = cursor + relative;
        let end = start + identifier.len();
        if identifier_boundary(bytes, start, end) {
            return Some(start);
        }
        cursor = end;
    }
    None
}

fn identifier_boundary(bytes: &[u8], start: usize, end: usize) -> bool {
    let identifier = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$');
    start
        .checked_sub(1)
        .and_then(|index| bytes.get(index))
        .is_none_or(|byte| !identifier(*byte))
        && bytes.get(end).is_none_or(|byte| !identifier(*byte))
}

fn skip_ascii_whitespace(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    cursor
}
