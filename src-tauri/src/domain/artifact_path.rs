use super::{ArtifactRelativePath, Gallery, ValidationError};

pub const DEFAULT_FOLDER_NAME_TEMPLATE: &str = "[{artist}] {title} [{group}] {id}";
pub const MAX_FOLDER_TEMPLATE_BYTES: usize = 512;
pub const MAX_FOLDER_COMPONENT_UTF16: usize = 180;
pub const MAX_MANAGED_ABSOLUTE_PATH_UTF16: usize = 240;

const TOKENS: [&str; 4] = ["artist", "title", "group", "id"];

pub fn validate_folder_name_template(template: &str) -> Result<(), ValidationError> {
    if template.trim().is_empty() {
        return Err(ValidationError::new(
            "folderNameTemplate",
            "must not be empty",
        ));
    }
    if template.len() > MAX_FOLDER_TEMPLATE_BYTES {
        return Err(ValidationError::new(
            "folderNameTemplate",
            "must be at most 512 bytes",
        ));
    }
    if template.chars().any(char::is_control) {
        return Err(ValidationError::new(
            "folderNameTemplate",
            "must not contain control characters",
        ));
    }

    let mut cursor = 0;
    let mut has_id = false;
    while cursor < template.len() {
        let character = template[cursor..]
            .chars()
            .next()
            .expect("cursor is within the template");
        match character {
            '{' => {
                let remainder = &template[cursor + 1..];
                let Some(relative_end) = remainder.find('}') else {
                    return Err(ValidationError::new(
                        "folderNameTemplate",
                        "contains an unbalanced token",
                    ));
                };
                let end = cursor + 1 + relative_end;
                let token = &template[cursor + 1..end];
                if !TOKENS.contains(&token) {
                    return Err(ValidationError::new(
                        "folderNameTemplate",
                        "contains an unknown token",
                    ));
                }
                has_id |= token == "id";
                cursor = end + 1;
            }
            '}' => {
                return Err(ValidationError::new(
                    "folderNameTemplate",
                    "contains an unbalanced token",
                ));
            }
            _ => cursor += character.len_utf8(),
        }
    }
    if !has_id {
        return Err(ValidationError::new(
            "folderNameTemplate",
            "must contain the {id} token",
        ));
    }
    Ok(())
}

pub fn plan_artifact_relative_directory(
    template: &str,
    gallery: &Gallery,
) -> Result<ArtifactRelativePath, ValidationError> {
    validate_folder_name_template(template)?;
    let id = gallery.id.get().to_string();
    let artist = gallery
        .metadata
        .primary_artist
        .as_deref()
        .unwrap_or_default();
    let group = gallery
        .metadata
        .primary_group
        .as_deref()
        .unwrap_or_default();

    let mut rendered = template.to_owned();
    if artist.trim().is_empty() {
        rendered = rendered.replace("[{artist}]", "");
    }
    if group.trim().is_empty() {
        rendered = rendered.replace("[{group}]", "");
    }
    rendered = rendered
        .replace("{artist}", artist)
        .replace("{title}", &gallery.metadata.title)
        .replace("{group}", group)
        .replace("{id}", &id);

    let sanitized = sanitize_windows_component(&rendered, &id)?;
    ArtifactRelativePath::new(sanitized)
}

fn sanitize_windows_component(value: &str, id: &str) -> Result<String, ValidationError> {
    let mut sanitized = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
        {
            sanitized.push('_');
        } else {
            sanitized.push(character);
        }
    }
    sanitized = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    sanitized = sanitized.trim_end_matches([' ', '.']).trim().to_owned();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return Err(ValidationError::new(
            "folderNameTemplate",
            "renders an empty or relative folder name",
        ));
    }

    if is_dos_device_name(&sanitized) || sanitized.to_ascii_lowercase().starts_with(".atsumi-") {
        sanitized.insert(0, '_');
    }

    if sanitized.encode_utf16().count() > MAX_FOLDER_COMPONENT_UTF16 {
        let suffix = format!(" {id}");
        let suffix_units = suffix.encode_utf16().count();
        let prefix_budget = MAX_FOLDER_COMPONENT_UTF16.saturating_sub(suffix_units);
        let mut prefix = String::new();
        let mut units = 0;
        for character in sanitized.chars() {
            let next = character.len_utf16();
            if units + next > prefix_budget {
                break;
            }
            prefix.push(character);
            units += next;
        }
        sanitized = format!("{}{}", prefix.trim_end_matches([' ', '.']), suffix);
    }
    if !sanitized.contains(id) {
        return Err(ValidationError::new(
            "folderNameTemplate",
            "renders a folder name without its gallery ID",
        ));
    }
    Ok(sanitized)
}

fn is_dos_device_name(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GalleryId, GalleryMetadata};

    fn gallery(title: &str, artist: Option<&str>, group: Option<&str>, id: i64) -> Gallery {
        Gallery::new(
            GalleryId::new(id).unwrap(),
            0,
            GalleryMetadata::new(
                title,
                artist.map(str::to_owned),
                group.map(str::to_owned),
                1,
            )
            .unwrap(),
        )
    }

    #[test]
    fn validates_known_tokens_and_requires_id() {
        assert!(validate_folder_name_template(DEFAULT_FOLDER_NAME_TEMPLATE).is_ok());
        for invalid in [
            "{title}",
            "{unknown} {id}",
            "{title {id}",
            "{title}} {id}",
            "x\ny {id}",
        ] {
            assert!(
                validate_folder_name_template(invalid).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn default_template_removes_exact_empty_optional_wrappers() {
        let planned = plan_artifact_relative_directory(
            DEFAULT_FOLDER_NAME_TEMPLATE,
            &gallery("Quiet Book", None, None, 42),
        )
        .unwrap();
        assert_eq!(planned.as_str(), "Quiet Book 42");
    }

    #[test]
    fn golden_template_rendering_is_unicode_safe_and_collapses_whitespace() {
        let planned = plan_artifact_relative_directory(
            "  [{artist}]   {title}   [{group}]   {id}. ",
            &gallery("작품：제목", Some(" 작가 "), None, 77),
        )
        .unwrap();
        assert_eq!(planned.as_str(), "[작가] 작품：제목 77");
        assert!(validate_folder_name_template("日本語 {title} {id}").is_ok());
    }

    #[test]
    fn only_exact_empty_square_wrappers_are_removed() {
        let planned = plan_artifact_relative_directory(
            "({artist}) [{group}] {title} {id}",
            &gallery("Book", None, None, 9),
        )
        .unwrap();
        assert_eq!(planned.as_str(), "() Book 9");
    }

    #[test]
    fn sanitizes_windows_names_and_preserves_id_when_truncated() {
        let planned = plan_artifact_relative_directory(
            "{title} {id}",
            &gallery(&format!("CON: {}", "긴".repeat(200)), None, None, 4113714),
        )
        .unwrap();
        assert!(!planned.as_str().contains(':'));
        assert!(planned.as_str().contains("4113714"));
        assert!(planned.as_str().encode_utf16().count() <= MAX_FOLDER_COMPONENT_UTF16);
    }

    #[test]
    fn guards_dos_devices_and_internal_namespaces() {
        assert_eq!(sanitize_windows_component("CON", "CON").unwrap(), "_CON");
        assert_eq!(
            sanitize_windows_component(".atsumi-quarantine 7", "7").unwrap(),
            "_.atsumi-quarantine 7"
        );
    }
}
