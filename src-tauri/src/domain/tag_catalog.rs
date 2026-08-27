use serde::{Deserialize, Serialize};

use super::ValidationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagNamespace {
    Artist,
    Group,
    Tag,
    Female,
    Male,
}

impl TagNamespace {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Group => "group",
            Self::Tag => "tag",
            Self::Female => "female",
            Self::Male => "male",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        match value {
            "artist" => Ok(Self::Artist),
            "group" => Ok(Self::Group),
            "tag" => Ok(Self::Tag),
            "female" => Ok(Self::Female),
            "male" => Ok(Self::Male),
            _ => Err(ValidationError::new(
                "namespace",
                "must be artist, group, tag, female, or male",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCatalogEntry {
    pub namespace: TagNamespace,
    pub name: String,
    pub normalized_name: String,
    pub canonical_token: String,
    pub gallery_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCatalogStatus {
    pub revision: u64,
    pub entry_count: u64,
    pub neutral_count: u64,
    pub female_count: u64,
    pub male_count: u64,
    pub artist_count: u64,
    pub group_count: u64,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TagSuggestionRequest {
    pub query: String,
    pub namespace: Option<TagNamespace>,
    pub limit: u32,
}

impl TagSuggestionRequest {
    pub fn normalized(mut self) -> Result<Self, ValidationError> {
        self.query = normalize_tag_name(&self.query);
        if self.query.len() > 200 {
            return Err(ValidationError::new("query", "must be at most 200 bytes"));
        }
        self.limit = self.limit.min(8);
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSuggestion {
    pub namespace: TagNamespace,
    pub name: String,
    pub token: String,
    pub gallery_count: u64,
    pub favorite: bool,
}

pub fn normalize_tag_name(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn canonical_tag_token(namespace: TagNamespace, name: &str) -> Result<String, ValidationError> {
    let name = normalize_tag_name(name);
    if name.is_empty() || name.len() > 200 {
        return Err(ValidationError::new(
            "tagName",
            "must be between 1 and 200 bytes",
        ));
    }
    Ok(format!("{}:{}", namespace.as_str(), name.replace(' ', "_")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artist_and_group_are_valid_catalog_namespaces() {
        assert_eq!(TagNamespace::parse("artist").unwrap(), TagNamespace::Artist);
        assert_eq!(TagNamespace::parse("group").unwrap(), TagNamespace::Group);
        assert_eq!(
            canonical_tag_token(TagNamespace::Artist, " Mizuno  Tooru ").unwrap(),
            "artist:mizuno_tooru"
        );
        assert_eq!(
            canonical_tag_token(TagNamespace::Group, "Circle Energy").unwrap(),
            "group:circle_energy"
        );
    }
}
