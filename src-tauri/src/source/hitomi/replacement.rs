//! Explicit, bounded replacement proof. Ordinary requested-ID validation stays strict.
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::HitomiGalleryMetadata;
use crate::source::SourceContractError;

/// A process-local proof produced only after comparing two independently fetched
/// metadata documents. It is not deserializable from IPC or a saved alias table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryReplacementProof {
    requested_id: u64,
    canonical_id: u64,
    content_sha256: String,
    requested_revision: String,
    canonical_revision: String,
    page_revisions: Vec<String>,
}

impl GalleryReplacementProof {
    pub fn requested_id(&self) -> u64 {
        self.requested_id
    }
    pub fn canonical_id(&self) -> u64 {
        self.canonical_id
    }
    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }
    pub fn canonical_revision(&self) -> &str {
        &self.canonical_revision
    }
    pub fn page_revisions(&self) -> &[String] {
        &self.page_revisions
    }
}

pub fn verify_gallery_replacement(
    requested_id: u64,
    response: &HitomiGalleryMetadata,
    canonical_request_id: u64,
    canonical: &HitomiGalleryMetadata,
) -> Result<GalleryReplacementProof, SourceContractError> {
    let reject = || {
        SourceContractError::invalid_data(
            "gallery replacement",
            "The replacement identity and ordered source pages could not be independently verified",
        )
    };
    // IDs also cross the JavaScript number boundary in the existing IPC contract.
    const MAX_SAFE_ID: u64 = (1_u64 << 53) - 1;
    if requested_id == 0
        || requested_id > MAX_SAFE_ID
        || canonical_request_id == 0
        || canonical_request_id > MAX_SAFE_ID
        || requested_id == canonical_request_id
        || response.id != canonical_request_id
        || canonical.id != canonical_request_id
        || response.pages.is_empty()
        || response.pages.len() != canonical.pages.len()
        || response.title != canonical.title
        || response.alternate_title != canonical.alternate_title
        || response.gallery_type != canonical.gallery_type
        || response.language != canonical.language
        || response.published_at != canonical.published_at
        || response.gallery_path != canonical.gallery_path
        || response.artists != canonical.artists
        || response.groups != canonical.groups
        || response.series != canonical.series
        || response.characters != canonical.characters
        || response.tags != canonical.tags
        || response.related_gallery_ids != canonical.related_gallery_ids
    {
        return Err(reject());
    }
    for (index, (old, new)) in response.pages.iter().zip(&canonical.pages).enumerate() {
        if old.source_page as usize != index + 1
            || new.source_page != old.source_page
            || old.name != new.name
            || old.hash != new.hash
            || old.width != new.width
            || old.height != new.height
            || old.source_revision != new.source_revision
        {
            return Err(reject());
        }
        super::galleryinfo::validate_file_hash(&new.hash, "replacement page hash")?;
    }
    // Availability flags are routing hints, not content identity. Normalizing a
    // clone for this digest does NOT change the canonical metadata used to fetch.
    let mut identity = canonical.clone();
    for page in &mut identity.pages {
        page.has_webp = None;
        page.has_avif = false;
        page.has_jxl = false;
    }
    identity.source_revision = super::SourceRevision::gallery(canonical.id, 0);
    let bytes = serde_json::to_vec(&identity).map_err(|_| reject())?;
    Ok(GalleryReplacementProof {
        requested_id,
        canonical_id: canonical.id,
        content_sha256: format!("{:x}", Sha256::digest(bytes)),
        requested_revision: response.source_revision.to_string(),
        canonical_revision: canonical.source_revision.to_string(),
        page_revisions: canonical
            .pages
            .iter()
            .map(|page| page.source_revision.to_string())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::hitomi::parse_galleryinfo_script;

    pub(crate) fn metadata(id: u64) -> HitomiGalleryMetadata {
        parse_galleryinfo_script(&format!(
            r#"var galleryinfo = {{"id":{id},"title":"Replacement fixture","artists":[{{"artist":"fixture"}}],"files":[{{"name":"1.png","hash":"{}","width":2,"height":3,"hasavif":1}},{{"name":"2.png","hash":"{}","width":3,"height":2}}]}};"#,
            "a".repeat(64), "b".repeat(64),
        )).unwrap()
    }

    #[test]
    fn same_ordered_content_ignores_only_rendition_availability() {
        let mut old = metadata(20);
        let canonical = metadata(20);
        old.pages[0].has_avif = false;
        old.pages[1].has_jxl = true;
        old.pages[1].has_webp = Some(true);
        let proof = verify_gallery_replacement(10, &old, 20, &canonical).unwrap();
        assert_eq!(proof.requested_id(), 10);
        assert_eq!(proof.canonical_id(), 20);
        assert_eq!(proof.page_revisions().len(), 2);
        assert_eq!(proof.content_sha256().len(), 64);
        assert_eq!(
            proof.content_sha256(),
            verify_gallery_replacement(10, &canonical, 20, &canonical)
                .unwrap()
                .content_sha256()
        );
    }

    #[test]
    fn identity_order_owner_dimensions_and_chain_must_match() {
        let original = metadata(20);
        let mut cases = Vec::new();
        let mut changed = original.clone();
        changed.id = 30;
        cases.push(changed);
        let mut changed = original.clone();
        changed.title.push('!');
        cases.push(changed);
        let mut changed = original.clone();
        changed.artists = vec!["different".into()];
        cases.push(changed);
        let mut changed = original.clone();
        changed.pages.swap(0, 1);
        cases.push(changed);
        let mut changed = original.clone();
        changed.pages[0].hash = "c".repeat(64);
        cases.push(changed);
        let mut changed = original.clone();
        changed.pages[0].width = Some(99);
        cases.push(changed);
        let mut changed = original.clone();
        changed.pages[0].name = "different.png".into();
        cases.push(changed);
        let mut changed = original.clone();
        changed.pages.pop();
        cases.push(changed);
        for changed in cases {
            assert!(verify_gallery_replacement(10, &original, 20, &changed).is_err());
        }
        for requested in [0, 20, 1_u64 << 53, u64::MAX] {
            assert!(verify_gallery_replacement(requested, &original, 20, &original).is_err());
        }
        assert!(verify_gallery_replacement(10, &original, 30, &original).is_err());
        assert!(verify_gallery_replacement(10, &original, 20, &metadata(10)).is_err());
    }
}
