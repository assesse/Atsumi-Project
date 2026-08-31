use std::{collections::BTreeSet, sync::LazyLock};

use serde::{Deserialize, Serialize};

use super::{
    normalize_search_tags, validate_folder_name_template, ValidationError,
    DEFAULT_FOLDER_NAME_TEMPLATE,
};

pub const DEFAULT_MAX_COLUMNS: u32 = 3;
pub const DEFAULT_PREVIEW_WIDTH: u32 = 220;
/// The related-gallery strip lives inside the floating detail view, so it has
/// its own size preference instead of inheriting the dense Explore card size.
pub const DEFAULT_RELATED_PREVIEW_WIDTH: u32 = 240;
pub const DEFAULT_EXPLORE_PAGE_SIZE: u32 = 50;
pub const DEFAULT_CACHE_LIMIT_GB: u32 = 10;
pub const DEFAULT_CONCURRENT_IMAGE_REQUESTS: u32 = 5;
pub const DEFAULT_REQUEST_START_INTERVAL_MS: u64 = 25;
pub const MAX_COLLAPSED_GROUP_KEYS: usize = 2_048;
pub const MAX_COLLAPSED_GROUP_KEY_BYTES: usize = 256;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewPresetDefinition {
    width: u32,
    legacy_max: Option<u32>,
}

static PREVIEW_PRESETS: LazyLock<Vec<PreviewPresetDefinition>> = LazyLock::new(|| {
    let definitions: Vec<PreviewPresetDefinition> =
        serde_json::from_str(include_str!("../../../shared/gallery-preview-presets.json"))
            .expect("shared gallery preview presets must be valid JSON");
    let widths = definitions
        .iter()
        .map(|definition| definition.width)
        .collect::<Vec<_>>();
    assert_eq!(
        widths.len(),
        7,
        "exactly seven gallery preview presets are required"
    );
    assert!(
        widths.windows(2).all(|pair| pair[0] < pair[1]),
        "gallery preview presets must be strictly increasing"
    );
    assert!(
        widths.contains(&DEFAULT_PREVIEW_WIDTH),
        "default gallery preview width must be a preset"
    );
    definitions
});

static PREVIEW_PRESET_WIDTHS: LazyLock<Vec<u32>> =
    LazyLock::new(|| PREVIEW_PRESETS.iter().map(|preset| preset.width).collect());

pub fn gallery_preview_preset_widths() -> &'static [u32] {
    PREVIEW_PRESET_WIDTHS.as_slice()
}

pub fn normalize_gallery_preview_width(value: u32) -> u32 {
    PREVIEW_PRESETS
        .iter()
        .find(|preset| preset.legacy_max.is_some_and(|maximum| value <= maximum))
        .or_else(|| PREVIEW_PRESETS.last())
        .map(|preset| preset.width)
        .expect("preview preset source of truth is non-empty")
}

pub fn is_gallery_preview_width(value: u32) -> bool {
    PREVIEW_PRESET_WIDTHS.contains(&value)
}

pub fn is_related_preview_width(value: u32) -> bool {
    matches!(value, 180 | 200 | 220 | 240 | 260 | 280 | 300 | 320)
}

/// Converts only well-formed Windows verbatim filesystem paths to the form a
/// user normally reads and edits. Device paths and malformed prefixes are
/// deliberately left unchanged.
pub fn windows_path_for_display(value: &str) -> String {
    const VERBATIM_PREFIX: &str = "\\\\?\\";
    const VERBATIM_UNC_PREFIX: &str = "\\\\?\\UNC\\";

    if let Some(rest) = value.strip_prefix(VERBATIM_UNC_PREFIX) {
        let mut components = rest.split('\\');
        if components.next().is_some_and(|server| !server.is_empty())
            && components.next().is_some_and(|share| !share.is_empty())
        {
            return format!(r"\\{rest}");
        }
        return value.to_owned();
    }

    if let Some(rest) = value.strip_prefix(VERBATIM_PREFIX) {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return rest.to_owned();
        }
    }

    value.to_owned()
}

pub fn download_root_for_display(value: &str) -> String {
    #[cfg(windows)]
    {
        windows_path_for_display(value)
    }
    #[cfg(not(windows))]
    {
        value.to_owned()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoFindHistoryMode {
    #[default]
    IncludeAllHistory,
    NewerThanOldestDownloaded,
}

impl AutoFindHistoryMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IncludeAllHistory => "include_all_history",
            Self::NewerThanOldestDownloaded => "newer_than_oldest_downloaded",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "include_all_history" => Some(Self::IncludeAllHistory),
            "newer_than_oldest_downloaded" => Some(Self::NewerThanOldestDownloaded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadOverlapAutoMode {
    #[default]
    Off,
    Recommend,
    StrictQuarantine,
}

impl DownloadOverlapAutoMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Recommend => "recommend",
            Self::StrictQuarantine => "strict_quarantine",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "recommend" => Some(Self::Recommend),
            "strict_quarantine" => Some(Self::StrictQuarantine),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GalleryGroupingMode {
    #[default]
    All,
    Day,
    Artist,
}

impl GalleryGroupingMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Day => "day",
            Self::Artist => "artist",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "day" => Some(Self::Day),
            "artist" => Some(Self::Artist),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GalleryDisplayMode {
    #[default]
    Detail,
    Compact,
}

impl GalleryDisplayMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Detail => "detail",
            Self::Compact => "compact",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "detail" => Some(Self::Detail),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub revision: u64,
    pub download_root: String,
    pub folder_name_template: String,
    pub explore_page_size: u32,
    pub max_columns: u32,
    pub preview_width: u32,
    pub related_preview_width: u32,
    pub privacy_mode: bool,
    pub cache_limit_gb: u32,
    pub concurrent_image_requests: u32,
    pub request_start_interval_ms: u64,
    pub auto_find_history_mode: AutoFindHistoryMode,
    pub download_overlap_auto_mode: DownloadOverlapAutoMode,
    pub auto_find_grouping: GalleryGroupingMode,
    pub downloads_grouping: GalleryGroupingMode,
    pub explore_display_mode: GalleryDisplayMode,
    pub auto_find_display_mode: GalleryDisplayMode,
    pub downloads_display_mode: GalleryDisplayMode,
    pub collapsed_group_keys: Vec<String>,
    pub search_include_tags: Vec<String>,
    pub search_exclude_tags: Vec<String>,
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            download_root: String::new(),
            folder_name_template: DEFAULT_FOLDER_NAME_TEMPLATE.to_owned(),
            explore_page_size: DEFAULT_EXPLORE_PAGE_SIZE,
            max_columns: DEFAULT_MAX_COLUMNS,
            preview_width: DEFAULT_PREVIEW_WIDTH,
            related_preview_width: DEFAULT_RELATED_PREVIEW_WIDTH,
            privacy_mode: false,
            cache_limit_gb: DEFAULT_CACHE_LIMIT_GB,
            concurrent_image_requests: DEFAULT_CONCURRENT_IMAGE_REQUESTS,
            request_start_interval_ms: DEFAULT_REQUEST_START_INTERVAL_MS,
            auto_find_history_mode: AutoFindHistoryMode::default(),
            download_overlap_auto_mode: DownloadOverlapAutoMode::default(),
            auto_find_grouping: GalleryGroupingMode::default(),
            downloads_grouping: GalleryGroupingMode::default(),
            explore_display_mode: GalleryDisplayMode::default(),
            auto_find_display_mode: GalleryDisplayMode::default(),
            downloads_display_mode: GalleryDisplayMode::default(),
            collapsed_group_keys: Vec::new(),
            search_include_tags: Vec::new(),
            search_exclude_tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub download_root: Option<String>,
    pub folder_name_template: Option<String>,
    pub explore_page_size: Option<u32>,
    pub max_columns: Option<u32>,
    pub preview_width: Option<u32>,
    pub related_preview_width: Option<u32>,
    pub privacy_mode: Option<bool>,
    pub cache_limit_gb: Option<u32>,
    pub concurrent_image_requests: Option<u32>,
    pub request_start_interval_ms: Option<u64>,
    pub auto_find_history_mode: Option<AutoFindHistoryMode>,
    pub download_overlap_auto_mode: Option<DownloadOverlapAutoMode>,
    pub auto_find_grouping: Option<GalleryGroupingMode>,
    pub downloads_grouping: Option<GalleryGroupingMode>,
    pub explore_display_mode: Option<GalleryDisplayMode>,
    pub auto_find_display_mode: Option<GalleryDisplayMode>,
    pub downloads_display_mode: Option<GalleryDisplayMode>,
    pub collapsed_group_keys: Option<Vec<String>>,
    pub search_include_tags: Option<Vec<String>>,
    pub search_exclude_tags: Option<Vec<String>>,
}

pub fn normalize_collapsed_group_keys(values: Vec<String>) -> Result<Vec<String>, ValidationError> {
    if values.len() > MAX_COLLAPSED_GROUP_KEYS {
        return Err(ValidationError::new(
            "collapsedGroupKeys",
            "must contain at most 2048 group keys",
        ));
    }
    let mut normalized = BTreeSet::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || value.len() > MAX_COLLAPSED_GROUP_KEY_BYTES {
            return Err(ValidationError::new(
                "collapsedGroupKeys",
                "each group key must be between 1 and 256 bytes",
            ));
        }
        normalized.insert(value.to_owned());
    }
    Ok(normalized.into_iter().collect())
}

impl SettingsSnapshot {
    pub fn apply_patch(&self, patch: SettingsPatch) -> Result<Self, ValidationError> {
        let mut next = self.clone();

        if let Some(value) = patch.download_root {
            next.download_root = value;
        }
        if let Some(value) = patch.folder_name_template {
            next.folder_name_template = value;
        }
        if let Some(value) = patch.explore_page_size {
            next.explore_page_size = value;
        }
        if let Some(value) = patch.max_columns {
            next.max_columns = value;
        }
        if let Some(value) = patch.preview_width {
            next.preview_width = value;
        }
        if let Some(value) = patch.related_preview_width {
            next.related_preview_width = value;
        }
        if let Some(value) = patch.privacy_mode {
            next.privacy_mode = value;
        }
        if let Some(value) = patch.cache_limit_gb {
            next.cache_limit_gb = value;
        }
        if let Some(value) = patch.concurrent_image_requests {
            next.concurrent_image_requests = value;
        }
        if let Some(value) = patch.request_start_interval_ms {
            next.request_start_interval_ms = value;
        }
        if let Some(value) = patch.auto_find_history_mode {
            next.auto_find_history_mode = value;
        }
        if let Some(value) = patch.download_overlap_auto_mode {
            next.download_overlap_auto_mode = value;
        }
        if let Some(value) = patch.auto_find_grouping {
            next.auto_find_grouping = value;
        }
        if let Some(value) = patch.downloads_grouping {
            next.downloads_grouping = value;
        }
        if let Some(value) = patch.explore_display_mode {
            next.explore_display_mode = value;
        }
        if let Some(value) = patch.auto_find_display_mode {
            next.auto_find_display_mode = value;
        }
        if let Some(value) = patch.downloads_display_mode {
            next.downloads_display_mode = value;
        }
        if let Some(value) = patch.collapsed_group_keys {
            next.collapsed_group_keys = normalize_collapsed_group_keys(value)?;
        }
        if let Some(value) = patch.search_include_tags {
            next.search_include_tags = normalize_search_tags(value, "searchIncludeTags")?;
        }
        if let Some(value) = patch.search_exclude_tags {
            next.search_exclude_tags = normalize_search_tags(value, "searchExcludeTags")?;
        }

        // Windows canonicalization is still used at filesystem boundaries,
        // but settings remain human-readable and editable.
        next.download_root = download_root_for_display(&next.download_root);

        next.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| ValidationError::new("revision", "cannot be incremented"))?;
        next.validate()?;
        Ok(next)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_folder_name_template(&self.folder_name_template)?;
        if !(10..=200).contains(&self.explore_page_size) {
            return Err(ValidationError::new(
                "explorePageSize",
                "must be between 10 and 200",
            ));
        }
        if !(1..=4).contains(&self.max_columns) {
            return Err(ValidationError::new(
                "maxColumns",
                "must be between 1 and 4",
            ));
        }
        if !is_gallery_preview_width(self.preview_width) {
            return Err(ValidationError::new(
                "previewWidth",
                "must be one of 160, 190, 220, 250, 280, 320 or 360",
            ));
        }
        if !is_related_preview_width(self.related_preview_width) {
            return Err(ValidationError::new(
                "relatedPreviewWidth",
                "must be one of 180, 200, 220, 240, 260, 280, 300 or 320",
            ));
        }
        if !(1..=30).contains(&self.cache_limit_gb) {
            return Err(ValidationError::new(
                "cacheLimitGb",
                "must be between 1 and 30",
            ));
        }
        if !(1..=30).contains(&self.concurrent_image_requests) {
            return Err(ValidationError::new(
                "concurrentImageRequests",
                "must be between 1 and 30",
            ));
        }
        if self.request_start_interval_ms > 5_000 {
            return Err(ValidationError::new(
                "requestStartIntervalMs",
                "must be at most 5000",
            ));
        }
        if normalize_collapsed_group_keys(self.collapsed_group_keys.clone())?
            != self.collapsed_group_keys
        {
            return Err(ValidationError::new(
                "collapsedGroupKeys",
                "must be normalized, unique and sorted",
            ));
        }
        if normalize_search_tags(self.search_include_tags.clone(), "searchIncludeTags")?
            != self.search_include_tags
        {
            return Err(ValidationError::new(
                "searchIncludeTags",
                "must be normalized, unique and sorted",
            ));
        }
        if normalize_search_tags(self.search_exclude_tags.clone(), "searchExcludeTags")?
            != self.search_exclude_tags
        {
            return Err(ValidationError::new(
                "searchExcludeTags",
                "must be normalized, unique and sorted",
            ));
        }
        if self
            .search_include_tags
            .iter()
            .any(|tag| self.search_exclude_tags.binary_search(tag).is_ok())
        {
            return Err(ValidationError::new(
                "searchIncludeTags",
                "must not overlap searchExcludeTags",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_display_paths_remove_only_safe_verbatim_prefixes() {
        assert_eq!(windows_path_for_display(r"\\?\D:\AD"), r"D:\AD");
        assert_eq!(
            windows_path_for_display(r"\\?\UNC\server\share\AD"),
            r"\\server\share\AD"
        );
        assert_eq!(windows_path_for_display(r"D:\AD"), r"D:\AD");
        assert_eq!(
            windows_path_for_display(r"\\server\share\AD"),
            r"\\server\share\AD"
        );
        assert_eq!(
            windows_path_for_display(r"\\.\PhysicalDrive0"),
            r"\\.\PhysicalDrive0"
        );
        assert_eq!(
            windows_path_for_display(r"\\?\UNC\server"),
            r"\\?\UNC\server"
        );
        assert_eq!(windows_path_for_display(r"\\?\D:AD"), r"\\?\D:AD");
        assert_eq!(
            windows_path_for_display(r"\\?\Volume{1234}\AD"),
            r"\\?\Volume{1234}\AD"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_download_roots_are_not_rewritten() {
        assert_eq!(download_root_for_display(r"\\?\D:\AD"), r"\\?\D:\AD");
    }

    #[test]
    fn shared_preview_presets_normalize_legacy_values_with_lower_ties() {
        assert_eq!(
            gallery_preview_preset_widths(),
            &[160, 190, 220, 250, 280, 320, 360]
        );
        assert_eq!(normalize_gallery_preview_width(221), 220);
        assert_eq!(normalize_gallery_preview_width(235), 220);
        assert_eq!(normalize_gallery_preview_width(236), 250);
        assert_eq!(normalize_gallery_preview_width(305), 280);
        assert_eq!(normalize_gallery_preview_width(306), 320);
    }

    #[test]
    fn accordion_group_keys_are_normalized_before_they_are_persisted() {
        assert_eq!(
            normalize_collapsed_group_keys(vec![
                " downloads\u{1f}day\u{1f}2026-08-25 ".into(),
                "auto-find\u{1f}artist\u{1f}mizuno".into(),
                "auto-find\u{1f}artist\u{1f}mizuno".into(),
            ])
            .expect("valid keys"),
            vec![
                "auto-find\u{1f}artist\u{1f}mizuno".to_owned(),
                "downloads\u{1f}day\u{1f}2026-08-25".to_owned(),
            ]
        );
        assert!(normalize_collapsed_group_keys(vec![" ".into()]).is_err());
    }

    #[test]
    fn global_search_tags_are_normalized_and_cannot_overlap() {
        let next = SettingsSnapshot::default()
            .apply_patch(SettingsPatch {
                search_include_tags: Some(vec![" Female:Glasses ".into(), "female:glasses".into()]),
                search_exclude_tags: Some(vec!["male:glasses".into()]),
                ..SettingsPatch::default()
            })
            .expect("valid global search tags");
        assert_eq!(next.search_include_tags, vec!["female:glasses"]);
        assert_eq!(next.search_exclude_tags, vec!["male:glasses"]);

        assert!(next
            .apply_patch(SettingsPatch {
                search_exclude_tags: Some(vec!["female:glasses".into()]),
                ..SettingsPatch::default()
            })
            .is_err());
    }

    #[test]
    fn gallery_grouping_defaults_to_all_and_accepts_persisted_modes() {
        let defaults = SettingsSnapshot::default();
        assert_eq!(defaults.auto_find_grouping, GalleryGroupingMode::All);
        assert_eq!(defaults.downloads_grouping, GalleryGroupingMode::All);

        let next = defaults
            .apply_patch(SettingsPatch {
                auto_find_grouping: Some(GalleryGroupingMode::Artist),
                downloads_grouping: Some(GalleryGroupingMode::Day),
                ..SettingsPatch::default()
            })
            .expect("valid gallery grouping modes");
        assert_eq!(next.auto_find_grouping, GalleryGroupingMode::Artist);
        assert_eq!(next.downloads_grouping, GalleryGroupingMode::Day);
    }
}
