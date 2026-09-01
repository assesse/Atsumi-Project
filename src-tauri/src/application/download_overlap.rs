use std::collections::BTreeSet;

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::domain::{
    ArtifactBundle, DownloadOverlapCandidate, DownloadOverlapGalleryRef, DownloadOverlapPagePair,
    DownloadOverlapRelation, DuplicateCandidateRecord, DuplicatePageHash, DuplicateRelation,
    HashProfile, PageArtifact, PageArtifactState, DOWNLOAD_OVERLAP_POLICY_VERSION,
};

use super::duplicate_analyzer::{analyze_artifact_pair, HashedArtifact};

pub(crate) fn normalize_overlap_artist(value: &str) -> Option<String> {
    let normalized = value
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|character| if character == '_' { ' ' } else { character })
        .collect::<String>();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

pub(crate) fn normalized_artist_keys(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| normalize_overlap_artist(value))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn overlap_artists_intersect(left: &[String], right: &[String]) -> bool {
    let left = normalized_artist_keys(left)
        .into_iter()
        .collect::<BTreeSet<_>>();
    normalized_artist_keys(right)
        .into_iter()
        .any(|artist| left.contains(&artist))
}

pub(crate) fn verified_overlap_pages(bundle: &ArtifactBundle) -> Option<Vec<&PageArtifact>> {
    let pages = bundle
        .pages
        .iter()
        .filter(|page| !page.excluded)
        .collect::<Vec<_>>();
    if pages.is_empty()
        || pages.len() != bundle.artifact.expected_page_count as usize
        || pages.iter().any(|page| {
            page.state != PageArtifactState::Present
                || page.byte_length.is_none()
                || page.sha256.is_none()
                || page.storage_format.is_none()
                || page.source_revision.as_deref().is_none_or(str::is_empty)
                || page.verified_at.as_deref().is_none_or(str::is_empty)
        })
    {
        return None;
    }
    Some(pages)
}

pub(crate) fn overlap_artifact_fingerprint(
    bundle: &ArtifactBundle,
    profile_version: u32,
) -> Option<String> {
    let mut pages = verified_overlap_pages(bundle)?;
    pages.sort_by_key(|page| page.page_id.source_page_number);
    let mut digest = Sha256::new();
    digest.update(b"atsumi-download-overlap\0");
    digest.update(profile_version.to_le_bytes());
    digest.update(DOWNLOAD_OVERLAP_POLICY_VERSION.to_le_bytes());
    for page in pages {
        digest.update(page.page_id.source_page_number.get().to_le_bytes());
        digest.update([0]);
        digest.update(page.source_revision.as_deref()?.as_bytes());
        digest.update([0]);
        digest.update(page.sha256.as_ref()?.as_str().as_bytes());
        digest.update([0xff]);
    }
    Some(format!("{:x}", digest.finalize()))
}

pub(crate) fn overlap_gallery_ref(bundle: &ArtifactBundle) -> DownloadOverlapGalleryRef {
    DownloadOverlapGalleryRef {
        entry_id: bundle.artifact.entry_id.to_string(),
        gallery_id: bundle.gallery.id,
        title: bundle.gallery.metadata.title.clone(),
        artists: bundle.gallery.metadata.artists.clone(),
        page_count: bundle.artifact.expected_page_count,
    }
}

pub(crate) fn analyze_download_overlap_pair(
    review_id: &str,
    incoming: &HashedArtifact,
    existing: &HashedArtifact,
    existing_fingerprint: String,
    profile: &HashProfile,
) -> Option<DownloadOverlapCandidate> {
    let record = analyze_artifact_pair(review_id, incoming, existing, profile, None)?;
    blocking_candidate_from_record(review_id, incoming, existing, existing_fingerprint, record)
}

fn blocking_candidate_from_record(
    review_id: &str,
    incoming: &HashedArtifact,
    existing: &HashedArtifact,
    existing_fingerprint: String,
    record: DuplicateCandidateRecord,
) -> Option<DownloadOverlapCandidate> {
    let incoming_is_parent = record.candidate.parent.entry_id == incoming.gallery.entry_id;
    let (incoming_coverage, existing_coverage) = if incoming_is_parent {
        (
            record.candidate.parent_coverage,
            record.candidate.candidate_coverage,
        )
    } else {
        (
            record.candidate.candidate_coverage,
            record.candidate.parent_coverage,
        )
    };
    let mut page_pairs = record
        .page_pairs
        .into_iter()
        .map(|pair| DownloadOverlapPagePair {
            incoming_source_page: if incoming_is_parent {
                pair.parent_source_page
            } else {
                pair.candidate_source_page
            },
            existing_source_page: if incoming_is_parent {
                pair.candidate_source_page
            } else {
                pair.parent_source_page
            },
            exact_sha256: pair.exact_sha256,
            d_hash_distance: pair.d_hash_distance,
            p_hash_distance: pair.p_hash_distance,
            detail_hash_distance: pair.detail_hash_distance,
            edge_similarity: pair.edge_similarity,
            visual_similarity: pair.visual_similarity,
            low_information: pair.low_information,
        })
        .collect::<Vec<_>>();
    page_pairs.sort_by_key(|pair| (pair.incoming_source_page, pair.existing_source_page));
    let matched_pages = u32::try_from(page_pairs.len()).ok()?;
    let exact_pages =
        u32::try_from(page_pairs.iter().filter(|pair| pair.exact_sha256).count()).ok()?;
    let visual_pages = matched_pages.saturating_sub(exact_pages);
    let non_low_information = page_pairs
        .iter()
        .filter(|pair| !pair.low_information)
        .count();
    let incoming_pages = u32::try_from(incoming.pages.len()).ok()?;
    let existing_pages = u32::try_from(existing.pages.len()).ok()?;
    let smaller_pages = incoming_pages.min(existing_pages);
    let tiny_exact =
        smaller_pages <= 3 && matched_pages == smaller_pages && exact_pages == matched_pages;
    let near_equivalent = record.candidate.relation == DuplicateRelation::Exact
        || (incoming_coverage >= 0.95
            && existing_coverage >= 0.95
            && (smaller_pages > 3 || tiny_exact));

    let relation = if near_equivalent {
        DownloadOverlapRelation::NearEquivalent
    } else if record.candidate.relation == DuplicateRelation::Contains
        && existing_coverage >= 0.90
        && incoming_pages > existing_pages
    {
        DownloadOverlapRelation::IncomingContainsExisting
    } else if record.candidate.relation == DuplicateRelation::Contains
        && incoming_coverage >= 0.90
        && existing_pages > incoming_pages
    {
        DownloadOverlapRelation::ExistingContainsIncoming
    } else if record.candidate.relation == DuplicateRelation::TranslationVisual {
        DownloadOverlapRelation::TranslationEdition
    } else {
        DownloadOverlapRelation::PartialOverlap
    };
    let aligned_run = longest_aligned_run(&page_pairs);

    let blocks = match relation {
        DownloadOverlapRelation::NearEquivalent => {
            if smaller_pages <= 3 {
                tiny_exact
            } else {
                record.candidate.relation == DuplicateRelation::Exact
                    || (incoming_coverage >= 0.95 && existing_coverage >= 0.95)
            }
        }
        DownloadOverlapRelation::IncomingContainsExisting
        | DownloadOverlapRelation::ExistingContainsIncoming => {
            if smaller_pages < 4 {
                tiny_exact
            } else {
                matched_pages >= 4 && incoming_coverage.max(existing_coverage) >= 0.90
            }
        }
        DownloadOverlapRelation::TranslationEdition => {
            smaller_pages > 3
                && non_low_information * 2 > page_pairs.len()
                && ((matched_pages >= 6 && incoming_coverage.max(existing_coverage) >= 0.90)
                    || (smaller_pages >= 8
                        && matched_pages >= 8
                        && incoming_coverage >= 0.75
                        && existing_coverage >= 0.75
                        && aligned_run >= 8))
        }
        DownloadOverlapRelation::PartialOverlap => {
            smaller_pages > 3
                && non_low_information * 2 > page_pairs.len()
                && ((matched_pages >= 6 && incoming_coverage.max(existing_coverage) >= 0.80)
                    || (matched_pages >= 10
                        && incoming_coverage >= 0.60
                        && existing_coverage >= 0.60))
        }
    };
    if !blocks {
        return None;
    }

    Some(DownloadOverlapCandidate {
        candidate_id: overlap_candidate_id(review_id, &existing.gallery.entry_id),
        existing: DownloadOverlapGalleryRef {
            entry_id: existing.gallery.entry_id.clone(),
            gallery_id: existing.gallery.gallery_id,
            title: existing.gallery.title.clone(),
            artists: existing.gallery.artist.iter().cloned().collect(),
            page_count: existing_pages,
        },
        existing_fingerprint,
        relation,
        confidence: record.candidate.confidence,
        matched_pages,
        exact_pages,
        visual_pages,
        existing_coverage,
        incoming_coverage,
        existing_unique_pages: existing_pages.saturating_sub(matched_pages),
        incoming_unique_pages: incoming_pages.saturating_sub(matched_pages),
        longest_aligned_run: aligned_run,
        rank: 0,
        decision: None,
        page_pairs,
    })
}

fn overlap_candidate_id(review_id: &str, existing_entry_id: &str) -> String {
    let digest = Sha256::digest(format!("{review_id}\0{existing_entry_id}").as_bytes());
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("overlap-candidate-{suffix}")
}

fn longest_aligned_run(pairs: &[DownloadOverlapPagePair]) -> u32 {
    let mut longest = 0_u32;
    let mut current = 0_u32;
    let mut previous = None;
    for pair in pairs {
        let coordinate = (pair.incoming_source_page, pair.existing_source_page);
        current = if previous.is_some_and(|previous: (u32, u32)| {
            coordinate.0 == previous.0.saturating_add(1)
                && coordinate.1 == previous.1.saturating_add(1)
        }) {
            current.saturating_add(1)
        } else {
            1
        };
        longest = longest.max(current);
        previous = Some(coordinate);
    }
    longest
}

pub(crate) fn hashed_artifact(
    gallery: crate::domain::DuplicateGalleryRef,
    mut pages: Vec<DuplicatePageHash>,
) -> HashedArtifact {
    pages.sort_by_key(|page| page.source_page_number);
    HashedArtifact { gallery, pages }
}

#[cfg(test)]
mod tests {
    use crate::domain::{ArtifactSha256, DuplicateGalleryRef, GalleryId, SourcePageNumber};

    use super::*;

    fn gallery(id: i64, pages: u32, artist: &str) -> DuplicateGalleryRef {
        DuplicateGalleryRef {
            gallery_id: GalleryId::new(id).unwrap(),
            entry_id: format!("entry-{id}"),
            title: format!("Gallery {id}"),
            artist: Some(artist.into()),
            group: None,
            page_count: pages,
        }
    }

    fn page(id: i64, source_page: u32, scene: u64, exact_salt: u64) -> DuplicatePageHash {
        DuplicatePageHash {
            entry_id: format!("entry-{id}"),
            gallery_id: GalleryId::new(id).unwrap(),
            source_page_number: SourcePageNumber::new(source_page).unwrap(),
            profile_version: 1,
            artifact_sha256: ArtifactSha256::new(format!("{:064x}", scene + exact_salt)).unwrap(),
            coarse_d_hash: scene,
            detail_d_hash_hex: format!("{:0256x}", scene),
            p_hash: scene,
            mean_luma: 127.0,
            std_dev: 40.0,
            non_uniform_ratio: 0.7,
            edge_density: 0.2,
            width: 100,
            height: 140,
            low_information: false,
        }
    }

    fn artifact(id: i64, count: u32, salt: u64) -> HashedArtifact {
        hashed_artifact(
            gallery(id, count, "Some_Artist"),
            (1..=count)
                .map(|source_page| page(id, source_page, u64::from(source_page), salt))
                .collect(),
        )
    }

    #[test]
    fn artist_normalization_is_nfkc_case_and_whitespace_exact() {
        assert_eq!(
            normalize_overlap_artist("  Ａrtist__Name  ").as_deref(),
            Some("artist name")
        );
        assert!(overlap_artists_intersect(
            &["Artist_Name".into(), "Second".into()],
            &["artist name".into()]
        ));
        assert!(!overlap_artists_intersect(
            &["artist-name".into()],
            &["artist name".into()]
        ));
        assert!(!overlap_artists_intersect(&[], &["artist".into()]));
    }

    #[test]
    fn exact_and_contains_relations_are_incoming_directional() {
        let profile = HashProfile::current();
        let existing = artifact(10, 20, 0);
        let incoming = artifact(20, 25, 0);
        let candidate =
            analyze_download_overlap_pair("review", &incoming, &existing, "a".repeat(64), &profile)
                .unwrap();
        assert_eq!(
            candidate.relation,
            DownloadOverlapRelation::IncomingContainsExisting
        );
        assert_eq!(candidate.matched_pages, 20);
        assert_eq!(candidate.existing_coverage, 1.0);
        assert_eq!(candidate.incoming_coverage, 0.8);

        let smaller_incoming = artifact(5, 15, 0);
        let reverse = analyze_download_overlap_pair(
            "reverse",
            &smaller_incoming,
            &existing,
            "9".repeat(64),
            &profile,
        )
        .unwrap();
        assert_eq!(
            reverse.relation,
            DownloadOverlapRelation::ExistingContainsIncoming
        );
        assert_eq!(reverse.matched_pages, 15);
        assert_eq!(reverse.existing_coverage, 0.75);
        assert_eq!(reverse.incoming_coverage, 1.0);

        let same = artifact(30, 20, 0);
        let exact =
            analyze_download_overlap_pair("same", &same, &existing, "b".repeat(64), &profile)
                .unwrap();
        assert_eq!(exact.relation, DownloadOverlapRelation::NearEquivalent);
    }

    #[test]
    fn recompressed_full_sequence_is_near_equivalent_without_exact_sha() {
        let profile = HashProfile::current();
        let existing = artifact(10, 8, 0);
        let incoming = artifact(20, 8, 10_000);
        let candidate =
            analyze_download_overlap_pair("review", &incoming, &existing, "c".repeat(64), &profile)
                .unwrap();
        assert_eq!(candidate.relation, DownloadOverlapRelation::NearEquivalent);
        assert_eq!(candidate.exact_pages, 0);
        assert_eq!(candidate.visual_pages, 8);
    }

    #[test]
    fn guarded_typesetting_sequence_reaches_download_overlap_review() {
        let profile = HashProfile::current();
        let existing = artifact(10, 25, 0);
        let mut incoming = artifact(20, 25, 10_000);
        for page in incoming.pages.iter_mut().take(22).skip(11) {
            page.edge_density = 0.29;
            page.width = 120;
            page.height = 168;
        }
        for page in incoming.pages.iter_mut().skip(22) {
            page.low_information = true;
            page.std_dev = 0.0;
            page.non_uniform_ratio = 0.0;
            page.edge_density = 0.0;
        }

        let candidate = analyze_download_overlap_pair(
            "typesetting-review",
            &incoming,
            &existing,
            "7".repeat(64),
            &profile,
        )
        .expect("a long guarded typesetting sequence should pause for review");
        assert_eq!(
            candidate.relation,
            DownloadOverlapRelation::TranslationEdition
        );
        assert_eq!(candidate.matched_pages, 22);
        assert_eq!(candidate.exact_pages, 0);
        assert_eq!(candidate.visual_pages, 22);
        assert_eq!(candidate.incoming_coverage, 0.88);
        assert_eq!(candidate.existing_coverage, 0.88);
        assert_eq!(candidate.longest_aligned_run, 22);
    }

    #[test]
    fn one_or_two_shared_pages_do_not_block() {
        let profile = HashProfile::current();
        let existing = artifact(10, 12, 0);
        let mut incoming = artifact(20, 12, 100_000);
        for (index, page) in incoming.pages.iter_mut().enumerate() {
            page.coarse_d_hash = u64::MAX.saturating_sub(index as u64 * 97);
            page.p_hash = u64::MAX.saturating_sub(index as u64 * 193);
            page.detail_d_hash_hex = "ff".repeat(128);
        }
        incoming.pages[0] = existing.pages[0].clone();
        incoming.pages[0].entry_id = "entry-20".into();
        incoming.pages[0].gallery_id = GalleryId::new(20).unwrap();
        incoming.pages[1] = existing.pages[1].clone();
        incoming.pages[1].entry_id = "entry-20".into();
        incoming.pages[1].gallery_id = GalleryId::new(20).unwrap();
        assert!(analyze_download_overlap_pair(
            "review",
            &incoming,
            &existing,
            "d".repeat(64),
            &profile,
        )
        .is_none());
    }

    #[test]
    fn tiny_visual_only_artifacts_do_not_block_but_tiny_exact_artifacts_do() {
        let profile = HashProfile::current();
        let existing = artifact(10, 3, 0);
        let visual_only = artifact(20, 3, 10_000);
        assert!(analyze_download_overlap_pair(
            "tiny-visual",
            &visual_only,
            &existing,
            "1".repeat(64),
            &profile,
        )
        .is_none());

        let exact = artifact(30, 3, 0);
        let exact_candidate = analyze_download_overlap_pair(
            "tiny-exact",
            &exact,
            &existing,
            "2".repeat(64),
            &profile,
        )
        .unwrap();
        assert_eq!(
            exact_candidate.relation,
            DownloadOverlapRelation::NearEquivalent
        );
        assert_eq!(exact_candidate.exact_pages, 3);
    }

    #[test]
    fn low_information_shared_pages_alone_do_not_block() {
        let profile = HashProfile::current();
        let existing = artifact(10, 12, 0);
        let mut incoming = artifact(20, 12, 100_000);
        for index in 0..6 {
            incoming.pages[index] = existing.pages[index].clone();
            incoming.pages[index].entry_id = "entry-20".into();
            incoming.pages[index].gallery_id = GalleryId::new(20).unwrap();
            incoming.pages[index].low_information = true;
            incoming.pages[index].std_dev = 0.0;
            incoming.pages[index].edge_density = 0.0;
            incoming.pages[index].non_uniform_ratio = 0.0;
        }
        for (index, page) in incoming.pages.iter_mut().enumerate().skip(6) {
            page.coarse_d_hash = u64::MAX.saturating_sub(index as u64 * 97);
            page.p_hash = u64::MAX.saturating_sub(index as u64 * 193);
            page.detail_d_hash_hex = "ff".repeat(128);
        }
        assert!(analyze_download_overlap_pair(
            "low-information",
            &incoming,
            &existing,
            "3".repeat(64),
            &profile,
        )
        .is_none());
    }

    #[test]
    fn strong_partial_blocks_but_weak_partial_passes() {
        let profile = HashProfile::current();
        let existing = artifact(10, 10, 0);
        let mut incoming = artifact(20, 10, 100_000);
        for index in 0..8 {
            incoming.pages[index] = existing.pages[index].clone();
            incoming.pages[index].entry_id = "entry-20".into();
            incoming.pages[index].gallery_id = GalleryId::new(20).unwrap();
        }
        assert!(analyze_download_overlap_pair(
            "strong",
            &incoming,
            &existing,
            "e".repeat(64),
            &profile,
        )
        .is_some());

        for (index, page) in incoming.pages.iter_mut().enumerate().skip(6) {
            page.artifact_sha256 =
                ArtifactSha256::new(format!("{:064x}", 900_000 + index)).unwrap();
            page.coarse_d_hash = u64::MAX.saturating_sub(index as u64 * 97);
            page.p_hash = u64::MAX.saturating_sub(index as u64 * 193);
            page.detail_d_hash_hex = "ff".repeat(128);
        }
        assert!(analyze_download_overlap_pair(
            "weak",
            &incoming,
            &existing,
            "f".repeat(64),
            &profile,
        )
        .is_none());
    }
}
