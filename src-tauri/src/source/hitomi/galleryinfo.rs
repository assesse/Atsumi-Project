use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::source::SourceContractError;

use super::model::{
    HitomiGalleryMetadata, HitomiPageFile, HitomiTag, HitomiTagKind, RevisionFingerprint,
    SourceRevision,
};

pub fn parse_galleryinfo_script(
    script: &str,
) -> Result<HitomiGalleryMetadata, SourceContractError> {
    let json = extract_galleryinfo_json(script)?;
    let value: Value = serde_json::from_str(json).map_err(|error| {
        SourceContractError::invalid_data("galleryinfo JSON", error.to_string())
    })?;
    let object = value.as_object().ok_or_else(|| {
        SourceContractError::invalid_data("galleryinfo", "root must be a JSON object")
    })?;

    parse_gallery_object(object)
}

fn extract_galleryinfo_json(script: &str) -> Result<&str, SourceContractError> {
    let bytes = script.as_bytes();
    let name = b"galleryinfo";
    let mut search_from = 0;

    while search_from + name.len() <= bytes.len() {
        let Some(relative) = script[search_from..].find("galleryinfo") else {
            break;
        };
        let start = search_from + relative;
        let end = start + name.len();
        search_from = end;

        if !is_identifier_boundary(bytes, start, end) {
            continue;
        }

        let mut cursor = skip_ascii_whitespace(bytes, end);
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor = skip_ascii_whitespace(bytes, cursor + 1);
        if bytes.get(cursor) != Some(&b'{') {
            return Err(SourceContractError::protocol(
                "galleryinfo assignment must contain a JSON object",
            ));
        }

        let mut depth = 0_u32;
        let mut in_string = false;
        let mut escaped = false;
        for index in cursor..bytes.len() {
            let byte = bytes[index];
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                continue;
            }

            match byte {
                b'"' => in_string = true,
                b'{' => depth = depth.saturating_add(1),
                b'}' => {
                    depth = depth.checked_sub(1).ok_or_else(|| {
                        SourceContractError::protocol(
                            "galleryinfo wrapper contains an unmatched closing brace",
                        )
                    })?;
                    if depth == 0 {
                        return Ok(&script[cursor..=index]);
                    }
                }
                _ => {}
            }
        }

        return Err(SourceContractError::protocol(
            "galleryinfo JSON object is not terminated",
        ));
    }

    Err(SourceContractError::protocol(
        "galleryinfo assignment was not found",
    ))
}

fn is_identifier_boundary(bytes: &[u8], start: usize, end: usize) -> bool {
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

fn parse_gallery_object(
    object: &Map<String, Value>,
) -> Result<HitomiGalleryMetadata, SourceContractError> {
    let id = required_positive_integer(object, "id", "galleryinfo.id")?;
    let title = required_string(object, "title", "galleryinfo.title")?;
    let alternate_title = optional_string_aliases(
        object,
        &["japanese_title", "title_jpn"],
        "galleryinfo alternate title",
    )?;
    let gallery_type = optional_string(object, "type", "galleryinfo.type")?;
    let language = optional_string(object, "language", "galleryinfo.language")?;
    let published_at = optional_string(object, "date", "galleryinfo.date")?;
    let gallery_path = parse_gallery_path(object.get("galleryurl"))?;
    let artists = parse_names(object.get("artists"), "artists", "artist")?;
    let groups = parse_names(object.get("groups"), "groups", "group")?;
    let series = parse_names(object.get("parodys"), "parodys", "parody")?;
    let characters = parse_names(object.get("characters"), "characters", "character")?;
    let tags = parse_tags(object.get("tags"))?;
    let related_gallery_ids = parse_related(object.get("related"))?;
    let pages = parse_pages(object.get("files"))?;

    let mut metadata = HitomiGalleryMetadata {
        id,
        title,
        alternate_title,
        gallery_type,
        language,
        published_at,
        gallery_path,
        artists,
        groups,
        series,
        characters,
        tags,
        related_gallery_ids,
        pages,
        source_revision: SourceRevision::gallery(id, 0),
    };
    metadata.source_revision = gallery_revision(&metadata);
    Ok(metadata)
}

fn required_positive_integer(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<u64, SourceContractError> {
    let value = object
        .get(field)
        .ok_or_else(|| SourceContractError::invalid_data(context, "field is required"))?;
    positive_integer(value, context)
}

fn positive_integer(value: &Value, context: &str) -> Result<u64, SourceContractError> {
    let number = value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| SourceContractError::invalid_data(context, "must be a positive integer"))?;
    Ok(number)
}

fn optional_positive_u32(
    value: Option<&Value>,
    context: &str,
) -> Result<Option<u32>, SourceContractError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let number = positive_integer(value, context)?;
    u32::try_from(number).map(Some).map_err(|_| {
        SourceContractError::invalid_data(context, "must fit in an unsigned 32-bit integer")
    })
}

fn required_string(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<String, SourceContractError> {
    let value = object
        .get(field)
        .ok_or_else(|| SourceContractError::invalid_data(context, "field is required"))?;
    normalized_string(value, context)?
        .ok_or_else(|| SourceContractError::invalid_data(context, "must be a non-empty string"))
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<Option<String>, SourceContractError> {
    object
        .get(field)
        .map_or(Ok(None), |value| normalized_string(value, context))
}

fn optional_string_aliases(
    object: &Map<String, Value>,
    fields: &[&str],
    context: &str,
) -> Result<Option<String>, SourceContractError> {
    for field in fields {
        if let Some(value) = object.get(*field) {
            return normalized_string(value, context);
        }
    }
    Ok(None)
}

fn normalized_string(value: &Value, context: &str) -> Result<Option<String>, SourceContractError> {
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or_else(|| SourceContractError::invalid_data(context, "must be a string or null"))?;
    let value = value.trim();
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

fn parse_gallery_path(value: Option<&Value>) -> Result<Option<String>, SourceContractError> {
    let Some(path) = value else {
        return Ok(None);
    };
    let Some(path) = normalized_string(path, "galleryinfo.galleryurl")? else {
        return Ok(None);
    };
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains(['\\', '?', '#'])
        || path.split('/').any(|segment| segment == "..")
    {
        return Err(SourceContractError::invalid_data(
            "galleryinfo.galleryurl",
            "must be a safe origin-relative path",
        ));
    }
    Ok(Some(path))
}

fn parse_names(
    value: Option<&Value>,
    collection: &str,
    member: &str,
) -> Result<Vec<String>, SourceContractError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let items = value.as_array().ok_or_else(|| {
        SourceContractError::invalid_data(format!("galleryinfo.{collection}"), "must be an array")
    })?;
    let mut names = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let context = format!("galleryinfo.{collection}[{index}]");
        let candidate = if item.is_string() {
            item
        } else {
            let object = item.as_object().ok_or_else(|| {
                SourceContractError::invalid_data(&context, "must be a string or object")
            })?;
            object
                .get(member)
                .or_else(|| object.get("name"))
                .ok_or_else(|| {
                    SourceContractError::invalid_data(
                        &context,
                        format!("must contain {member:?} or \"name\""),
                    )
                })?
        };
        let name = normalized_string(candidate, &context)?
            .ok_or_else(|| SourceContractError::invalid_data(&context, "name must not be empty"))?;
        names.push(name);
    }
    deduplicate_strings(&mut names);
    Ok(names)
}

fn parse_tags(value: Option<&Value>) -> Result<Vec<HitomiTag>, SourceContractError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let items = value
        .as_array()
        .ok_or_else(|| SourceContractError::invalid_data("galleryinfo.tags", "must be an array"))?;
    let mut tags = Vec::with_capacity(items.len());
    let mut seen = HashSet::new();

    for (index, item) in items.iter().enumerate() {
        let context = format!("galleryinfo.tags[{index}]");
        let (name, female, male) = if item.is_string() {
            (
                normalized_string(item, &context)?.ok_or_else(|| {
                    SourceContractError::invalid_data(&context, "tag name must not be empty")
                })?,
                false,
                false,
            )
        } else {
            let object = item.as_object().ok_or_else(|| {
                SourceContractError::invalid_data(&context, "must be a string or object")
            })?;
            let name = required_string(object, "tag", &format!("{context}.tag"))?;
            let female = flexible_boolean(object.get("female"), &format!("{context}.female"))?;
            let male = flexible_boolean(object.get("male"), &format!("{context}.male"))?;
            (name, female, male)
        };
        if female && male {
            return Err(SourceContractError::invalid_data(
                &context,
                "tag cannot be both female and male",
            ));
        }
        let kind = if female {
            HitomiTagKind::Female
        } else if male {
            HitomiTagKind::Male
        } else {
            HitomiTagKind::General
        };
        if seen.insert((name.to_ascii_lowercase(), kind)) {
            tags.push(HitomiTag { name, kind });
        }
    }
    Ok(tags)
}

fn parse_related(value: Option<&Value>) -> Result<Vec<u64>, SourceContractError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let items = value.as_array().ok_or_else(|| {
        SourceContractError::invalid_data("galleryinfo.related", "must be an array")
    })?;
    let mut ids = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        ids.push(positive_integer(
            item,
            &format!("galleryinfo.related[{index}]"),
        )?);
    }
    let mut seen = HashSet::new();
    ids.retain(|id| seen.insert(*id));
    Ok(ids)
}

fn parse_pages(value: Option<&Value>) -> Result<Vec<HitomiPageFile>, SourceContractError> {
    let value = value.ok_or_else(|| {
        SourceContractError::invalid_data("galleryinfo.files", "field is required")
    })?;
    let items = value.as_array().ok_or_else(|| {
        SourceContractError::invalid_data("galleryinfo.files", "must be an array")
    })?;
    if items.is_empty() {
        return Err(SourceContractError::invalid_data(
            "galleryinfo.files",
            "must contain at least one page",
        ));
    }
    if items.len() > u32::MAX as usize {
        return Err(SourceContractError::invalid_data(
            "galleryinfo.files",
            "contains too many pages",
        ));
    }

    items
        .iter()
        .enumerate()
        .map(|(index, item)| parse_page(item, index as u32 + 1))
        .collect()
}

fn parse_page(value: &Value, source_page: u32) -> Result<HitomiPageFile, SourceContractError> {
    let context = format!("galleryinfo.files[{}]", source_page - 1);
    let object = value
        .as_object()
        .ok_or_else(|| SourceContractError::invalid_data(&context, "must be an object"))?;
    let name = required_string(object, "name", &format!("{context}.name"))?;
    if name.contains(['/', '\\', '\0']) {
        return Err(SourceContractError::invalid_data(
            format!("{context}.name"),
            "must be a file name, not a path",
        ));
    }

    let hash = required_string(object, "hash", &format!("{context}.hash"))?.to_ascii_lowercase();
    validate_file_hash(&hash, &format!("{context}.hash"))?;

    Ok(HitomiPageFile {
        source_page,
        name,
        hash: hash.clone(),
        width: optional_positive_u32(object.get("width"), &format!("{context}.width"))?,
        height: optional_positive_u32(object.get("height"), &format!("{context}.height"))?,
        has_webp: flexible_optional_boolean(object.get("haswebp"), &format!("{context}.haswebp"))?,
        has_avif: flexible_boolean(object.get("hasavif"), &format!("{context}.hasavif"))?,
        has_jxl: flexible_boolean(object.get("hasjxl"), &format!("{context}.hasjxl"))?,
        source_revision: SourceRevision::page(&hash),
    })
}

pub(crate) fn validate_file_hash(hash: &str, context: &str) -> Result<(), SourceContractError> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SourceContractError::invalid_data(
            context,
            "must be a 64-character hexadecimal digest",
        ));
    }
    Ok(())
}

fn flexible_boolean(value: Option<&Value>, context: &str) -> Result<bool, SourceContractError> {
    let Some(value) = value else {
        return Ok(false);
    };
    match value {
        Value::Null => Ok(false),
        Value::Bool(value) => Ok(*value),
        Value::Number(value) => value
            .as_u64()
            .filter(|value| *value <= 1)
            .map(|value| value == 1)
            .ok_or_else(|| {
                SourceContractError::invalid_data(context, "numeric flag must be 0 or 1")
            }),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Ok(true),
            "0" | "false" | "no" | "" => Ok(false),
            _ => Err(SourceContractError::invalid_data(
                context,
                "string flag must be true/false or 1/0",
            )),
        },
        _ => Err(SourceContractError::invalid_data(
            context,
            "flag must be boolean, 1/0, string, or null",
        )),
    }
}

fn flexible_optional_boolean(
    value: Option<&Value>,
    context: &str,
) -> Result<Option<bool>, SourceContractError> {
    value
        .map(|value| flexible_boolean(Some(value), context).map(Some))
        .transpose()
        .map(Option::flatten)
}

fn deduplicate_strings(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.to_ascii_lowercase()));
}

fn gallery_revision(metadata: &HitomiGalleryMetadata) -> SourceRevision {
    let mut fingerprint = RevisionFingerprint::new();
    fingerprint.field("hitomi-gallery-metadata-v1");
    fingerprint.number(metadata.id);
    fingerprint.field(&metadata.title);
    fingerprint.optional_string(metadata.alternate_title.as_deref());
    fingerprint.optional_string(metadata.gallery_type.as_deref());
    fingerprint.optional_string(metadata.language.as_deref());
    fingerprint.optional_string(metadata.published_at.as_deref());
    fingerprint.optional_string(metadata.gallery_path.as_deref());
    fingerprint.strings(&metadata.artists);
    fingerprint.strings(&metadata.groups);
    fingerprint.strings(&metadata.series);
    fingerprint.strings(&metadata.characters);
    fingerprint.number(metadata.tags.len() as u64);
    for tag in &metadata.tags {
        fingerprint.field(tag.kind.as_str());
        fingerprint.field(&tag.name);
    }
    fingerprint.number(metadata.related_gallery_ids.len() as u64);
    for id in &metadata.related_gallery_ids {
        fingerprint.number(*id);
    }
    fingerprint.number(metadata.pages.len() as u64);
    for page in &metadata.pages {
        fingerprint.number(u64::from(page.source_page));
        fingerprint.field(&page.name);
        fingerprint.field(&page.hash);
        fingerprint.optional_number(page.width.map(u64::from));
        fingerprint.optional_number(page.height.map(u64::from));
        fingerprint.optional_boolean(page.has_webp);
        fingerprint.boolean(page.has_avif);
        fingerprint.boolean(page.has_jxl);
    }
    SourceRevision::gallery(metadata.id, fingerprint.finish())
}

trait RevisionFingerprintExt {
    fn optional_string(&mut self, value: Option<&str>);
    fn optional_number(&mut self, value: Option<u64>);
    fn optional_boolean(&mut self, value: Option<bool>);
    fn strings(&mut self, values: &[String]);
}

impl RevisionFingerprintExt for RevisionFingerprint {
    fn optional_string(&mut self, value: Option<&str>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.field(value);
        }
    }

    fn optional_number(&mut self, value: Option<u64>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.number(value);
        }
    }

    fn optional_boolean(&mut self, value: Option<bool>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.boolean(value);
        }
    }

    fn strings(&mut self, values: &[String]) {
        self.number(values.len() as u64);
        for value in values {
            self.field(value);
        }
    }
}
