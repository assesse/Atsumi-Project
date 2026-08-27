use std::io::Cursor;

use image::{imageops::FilterType, GenericImageView, ImageReader, Limits};

use crate::domain::{
    ArtifactBundle, DuplicateCandidate, DuplicateCandidateRecord, DuplicateEvidence,
    DuplicateEvidenceKind, DuplicateGalleryRef, DuplicatePageHash, DuplicatePagePair,
    DuplicateRelation, ExternalRelationEvidence, HashProfile, PageArtifact, SourcePageNumber,
};

use super::RepositoryError;

const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_DECODE_ALLOC: u64 = 256 * 1024 * 1024;
const DETAIL_HASH_BITS: u32 = 1_024;

#[derive(Debug, Clone)]
pub(crate) struct HashedArtifact {
    pub gallery: DuplicateGalleryRef,
    pub pages: Vec<DuplicatePageHash>,
}

pub(crate) fn verified_scan_pages(bundle: &ArtifactBundle) -> Option<Vec<&PageArtifact>> {
    if bundle.artifact.state != crate::domain::DownloadArtifactState::Complete
        || bundle.artifact.manifest_schema_version.is_none()
        || bundle.artifact.completed_at.is_none()
        || bundle.artifact.hash_profile_version == 0
    {
        return None;
    }
    let pages = bundle
        .pages
        .iter()
        .filter(|page| !page.excluded)
        .collect::<Vec<_>>();
    if pages.is_empty()
        || pages.iter().any(|page| {
            page.state != crate::domain::PageArtifactState::Present
                || page.byte_length.is_none()
                || page.sha256.is_none()
                || page.storage_format.is_none()
                || page.source_revision.is_none()
                || page.verified_at.is_none()
        })
    {
        return None;
    }
    Some(pages)
}

pub(crate) fn gallery_ref(bundle: &ArtifactBundle, page_count: u32) -> DuplicateGalleryRef {
    DuplicateGalleryRef {
        gallery_id: bundle.gallery.id,
        entry_id: bundle.artifact.entry_id.to_string(),
        title: bundle.gallery.metadata.title.clone(),
        artist: bundle.gallery.metadata.primary_artist.clone(),
        group: bundle.gallery.metadata.primary_group.clone(),
        page_count,
    }
}

pub(crate) fn compute_page_hash(
    entry_id: &str,
    gallery_id: crate::domain::GalleryId,
    source_page_number: SourcePageNumber,
    artifact_sha256: crate::domain::ArtifactSha256,
    bytes: &[u8],
    profile: &HashProfile,
) -> Result<DuplicatePageHash, RepositoryError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| RepositoryError::Other("managed page image format is unsupported".into()))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| {
        RepositoryError::Other("managed page image could not be decoded safely".into())
    })?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(RepositoryError::Other(
            "managed page image has invalid dimensions".into(),
        ));
    }
    let gray = image.to_luma8();
    let stats_image = image::imageops::resize(&gray, 64, 64, FilterType::Triangle);
    let stats = image_stats(stats_image.as_raw());
    let coarse = image::imageops::resize(&gray, 9, 8, FilterType::Triangle);
    let coarse_d_hash = adjacent_hash_64(&coarse);
    let detail = image::imageops::resize(&gray, 33, 32, FilterType::Triangle);
    let detail_d_hash_hex = detail_hash_hex(&detail);
    let p_hash = perceptual_hash(&gray);
    let low_information = stats.std_dev < profile.low_information_std_dev_threshold
        || stats.non_uniform_ratio < 0.06
        || stats.edge_density < 0.008;

    Ok(DuplicatePageHash {
        entry_id: entry_id.to_owned(),
        gallery_id,
        source_page_number,
        profile_version: profile.profile_version,
        artifact_sha256,
        coarse_d_hash,
        detail_d_hash_hex,
        p_hash,
        mean_luma: stats.mean,
        std_dev: stats.std_dev,
        non_uniform_ratio: stats.non_uniform_ratio,
        edge_density: stats.edge_density,
        width,
        height,
        low_information,
    })
}

pub(crate) fn analyze_artifact_pair(
    run_id: &str,
    parent: &HashedArtifact,
    candidate: &HashedArtifact,
    profile: &HashProfile,
    external_relation: Option<ExternalRelationEvidence>,
) -> Option<DuplicateCandidateRecord> {
    if parent.gallery.gallery_id > candidate.gallery.gallery_id {
        return analyze_artifact_pair(run_id, candidate, parent, profile, external_relation);
    }
    let alignment = align_pages(&parent.pages, &candidate.pages, profile);
    if alignment.is_empty() {
        return None;
    }
    let exact_pages = alignment.iter().filter(|pair| pair.exact_sha256).count();
    let matched_pages = alignment.len();
    let parent_coverage = matched_pages as f64 / parent.pages.len() as f64;
    let candidate_coverage = matched_pages as f64 / candidate.pages.len() as f64;
    let average_visual = alignment
        .iter()
        .map(|pair| pair.visual_similarity)
        .sum::<f64>()
        / matched_pages as f64;
    // One side being fully covered means that gallery is contained in the
    // other.  The minimum is deliberately retained as overlap coverage for
    // confidence/partial classification, not for containment.
    let contained_coverage = parent_coverage.max(candidate_coverage);
    let overlap_coverage = parent_coverage.min(candidate_coverage);
    let relation = if exact_pages == matched_pages
        && matched_pages == parent.pages.len()
        && matched_pages == candidate.pages.len()
    {
        DuplicateRelation::Exact
    } else if matched_pages >= 2
        && overlap_coverage >= 0.65
        && exact_pages * 2 < matched_pages
        && average_visual >= profile.visual_match_threshold
    {
        DuplicateRelation::TranslationVisual
    } else if matched_pages >= 2 && contained_coverage >= 0.999 {
        DuplicateRelation::Contains
    } else if matched_pages >= 2
        && overlap_coverage >= 0.45
        && (average_visual >= profile.visual_match_threshold || exact_pages * 2 >= matched_pages)
    {
        DuplicateRelation::Partial
    } else {
        return None;
    };

    let coverage_confidence = overlap_coverage;
    let sequence_confidence = (matched_pages as f64
        / parent.pages.len().max(candidate.pages.len()) as f64)
        .clamp(0.0, 1.0);
    let mut confidence = match relation {
        DuplicateRelation::Exact => 1.0,
        DuplicateRelation::Contains => {
            (0.62 + average_visual * 0.25 + sequence_confidence * 0.13).min(0.99)
        }
        DuplicateRelation::TranslationVisual => {
            (average_visual * 0.55 + coverage_confidence * 0.35 + 0.08).min(0.96)
        }
        DuplicateRelation::Partial => {
            (average_visual * 0.45 + coverage_confidence * 0.40 + 0.05).min(0.90)
        }
    };
    if let Some(external) = &external_relation {
        confidence = (confidence * 0.9 + external.confidence.clamp(0.0, 1.0) * 0.1).min(1.0);
    }

    let candidate_id = format!(
        "duplicate-p{}-{}-{}",
        profile.profile_version,
        parent.gallery.gallery_id.get(),
        candidate.gallery.gallery_id.get()
    );
    let mut evidence = Vec::new();
    if exact_pages > 0 {
        evidence.push(DuplicateEvidence {
            evidence_id: format!("{candidate_id}-exact"),
            kind: DuplicateEvidenceKind::ExactSha256,
            confidence: 1.0,
            matched_pages: exact_pages as u32,
            description: "Verified artifact SHA-256 digests match for these source pages".into(),
        });
    }
    let visual_pages = matched_pages - exact_pages;
    if visual_pages > 0 {
        evidence.push(DuplicateEvidence {
            evidence_id: format!("{candidate_id}-visual"),
            kind: DuplicateEvidenceKind::VisualHash,
            confidence: average_visual,
            matched_pages: visual_pages as u32,
            description:
                "Non-blank pages match through perceptual, 1024-bit detail, and edge gates".into(),
        });
    }
    evidence.push(DuplicateEvidence {
        evidence_id: format!("{candidate_id}-sequence"),
        kind: DuplicateEvidenceKind::SequenceAlignment,
        confidence: sequence_confidence,
        matched_pages: matched_pages as u32,
        description: "Source pages form a monotonic one-to-one gap-tolerant alignment".into(),
    });
    if let Some(external) = external_relation {
        evidence.push(DuplicateEvidence {
            evidence_id: format!("{candidate_id}-external"),
            kind: DuplicateEvidenceKind::EHentaiRelation,
            confidence: external.confidence.clamp(0.0, 1.0),
            matched_pages: 0,
            description: external.description,
        });
    }

    Some(DuplicateCandidateRecord {
        run_id: run_id.to_owned(),
        candidate: DuplicateCandidate {
            candidate_id,
            revision: 0,
            parent: parent.gallery.clone(),
            candidate: candidate.gallery.clone(),
            relation,
            confidence,
            matched_pages: matched_pages as u32,
            parent_coverage,
            candidate_coverage,
            created_at: String::new(),
            updated_at: String::new(),
        },
        evidence,
        page_pairs: alignment,
    })
}

#[derive(Debug, Clone, Copy)]
struct ImageStats {
    mean: f64,
    std_dev: f64,
    non_uniform_ratio: f64,
    edge_density: f64,
}

fn image_stats(pixels: &[u8]) -> ImageStats {
    let mean = pixels.iter().map(|value| f64::from(*value)).sum::<f64>() / pixels.len() as f64;
    let variance = pixels
        .iter()
        .map(|value| (f64::from(*value) - mean).powi(2))
        .sum::<f64>()
        / pixels.len() as f64;
    let std_dev = variance.sqrt();
    let non_uniform_ratio = pixels
        .iter()
        .filter(|value| (f64::from(**value) - mean).abs() >= 12.0)
        .count() as f64
        / pixels.len() as f64;
    let mut edges = 0_u32;
    let mut comparisons = 0_u32;
    for y in 0..64_usize {
        for x in 0..64_usize {
            let current = pixels[y * 64 + x];
            if x + 1 < 64 {
                comparisons += 1;
                edges += u32::from(current.abs_diff(pixels[y * 64 + x + 1]) >= 24);
            }
            if y + 1 < 64 {
                comparisons += 1;
                edges += u32::from(current.abs_diff(pixels[(y + 1) * 64 + x]) >= 24);
            }
        }
    }
    ImageStats {
        mean,
        std_dev,
        non_uniform_ratio,
        edge_density: f64::from(edges) / f64::from(comparisons.max(1)),
    }
}

fn adjacent_hash_64(image: &image::GrayImage) -> u64 {
    let mut hash = 0_u64;
    for y in 0..8 {
        for x in 0..8 {
            let bit = y * 8 + x;
            if image.get_pixel(x, y)[0] > image.get_pixel(x + 1, y)[0] {
                hash |= 1_u64 << bit;
            }
        }
    }
    hash
}

fn detail_hash_hex(image: &image::GrayImage) -> String {
    let mut bytes = [0_u8; 128];
    for y in 0..32 {
        for x in 0..32 {
            let bit = (y * 32 + x) as usize;
            if image.get_pixel(x, y)[0] > image.get_pixel(x + 1, y)[0] {
                bytes[bit / 8] |= 1 << (bit % 8);
            }
        }
    }
    hex_bytes(&bytes)
}

fn perceptual_hash(image: &image::GrayImage) -> u64 {
    let resized = image::imageops::resize(image, 32, 32, FilterType::Triangle);
    let mut coefficients = [0_f64; 64];
    for v in 0..8 {
        for u in 0..8 {
            let mut sum = 0_f64;
            for y in 0..32 {
                for x in 0..32 {
                    let pixel = f64::from(resized.get_pixel(x, y)[0]) - 127.5;
                    sum += pixel
                        * ((std::f64::consts::PI * f64::from((2 * x + 1) * u)) / 64.0).cos()
                        * ((std::f64::consts::PI * f64::from((2 * y + 1) * v)) / 64.0).cos();
                }
            }
            coefficients[(v * 8 + u) as usize] = sum;
        }
    }
    let mut median_values = coefficients[1..].to_vec();
    median_values.sort_by(f64::total_cmp);
    let median = median_values[median_values.len() / 2];
    coefficients
        .iter()
        .enumerate()
        .fold(0_u64, |hash, (index, coefficient)| {
            hash | (u64::from(*coefficient >= median) << index)
        })
}

#[derive(Debug, Clone)]
struct PairMetric {
    score: f64,
    pair: DuplicatePagePair,
}

fn align_pages(
    parent: &[DuplicatePageHash],
    candidate: &[DuplicatePageHash],
    profile: &HashProfile,
) -> Vec<DuplicatePagePair> {
    let mut metrics = vec![vec![None; candidate.len()]; parent.len()];
    for (parent_index, parent_page) in parent.iter().enumerate() {
        for (candidate_index, candidate_page) in candidate.iter().enumerate() {
            metrics[parent_index][candidate_index] =
                page_metric(parent_page, candidate_page, profile);
        }
    }
    let mut scores = vec![vec![0_f64; candidate.len() + 1]; parent.len() + 1];
    let mut directions = vec![vec![0_u8; candidate.len() + 1]; parent.len() + 1];
    for i in 1..=parent.len() {
        for j in 1..=candidate.len() {
            let up = scores[i - 1][j];
            let left = scores[i][j - 1];
            let diagonal = metrics[i - 1][j - 1]
                .as_ref()
                .map_or(f64::NEG_INFINITY, |metric| {
                    scores[i - 1][j - 1] + metric.score
                });
            if diagonal >= up && diagonal >= left {
                scores[i][j] = diagonal;
                directions[i][j] = 3;
            } else if up >= left {
                scores[i][j] = up;
                directions[i][j] = 1;
            } else {
                scores[i][j] = left;
                directions[i][j] = 2;
            }
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (parent.len(), candidate.len());
    while i > 0 && j > 0 {
        match directions[i][j] {
            3 => {
                if let Some(metric) = &metrics[i - 1][j - 1] {
                    pairs.push(metric.pair.clone());
                }
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            2 => j -= 1,
            _ => break,
        }
    }
    pairs.reverse();
    pairs
}

fn page_metric(
    parent: &DuplicatePageHash,
    candidate: &DuplicatePageHash,
    profile: &HashProfile,
) -> Option<PairMetric> {
    let exact = parent.artifact_sha256 == candidate.artifact_sha256;
    let coarse_distance = (parent.coarse_d_hash ^ candidate.coarse_d_hash).count_ones();
    let p_distance = (parent.p_hash ^ candidate.p_hash).count_ones();
    let detail_distance =
        hex_hamming_distance(&parent.detail_d_hash_hex, &candidate.detail_d_hash_hex)?;
    let central_detail_distance =
        central_detail_distance(&parent.detail_d_hash_hex, &candidate.detail_d_hash_hex)?;
    let edge_similarity = ratio_similarity(parent.edge_density, candidate.edge_density, 0.20);
    let std_similarity = ratio_similarity(parent.std_dev, candidate.std_dev, 96.0);
    let content_similarity =
        ratio_similarity(parent.non_uniform_ratio, candidate.non_uniform_ratio, 0.75);
    let coarse_similarity = 1.0 - f64::from(coarse_distance) / 64.0;
    let p_similarity = 1.0 - f64::from(p_distance) / 64.0;
    let detail_similarity = 1.0 - f64::from(detail_distance) / f64::from(DETAIL_HASH_BITS);
    let visual_similarity = (coarse_similarity * 0.15
        + p_similarity * 0.25
        + detail_similarity * 0.35
        + edge_similarity * 0.15
        + std_similarity * 0.05
        + content_similarity * 0.05)
        .clamp(0.0, 1.0);
    let low_information = parent.low_information || candidate.low_information;
    let visual_match = !low_information
        && detail_distance <= 260
        && central_detail_distance <= 48
        && p_distance <= 16
        && coarse_distance <= 20
        && edge_similarity >= 0.62
        && content_similarity >= 0.60
        && visual_similarity >= profile.visual_match_threshold;
    if !exact && !visual_match {
        return None;
    }
    Some(PairMetric {
        score: if exact { 1.0 } else { visual_similarity },
        pair: DuplicatePagePair {
            parent_source_page: parent.source_page_number.get(),
            candidate_source_page: candidate.source_page_number.get(),
            exact_sha256: exact,
            d_hash_distance: coarse_distance,
            p_hash_distance: p_distance,
            detail_hash_distance: detail_distance,
            edge_similarity,
            visual_similarity: if exact { 1.0 } else { visual_similarity },
            low_information,
        },
    })
}

#[allow(dead_code)]
pub(crate) fn compare_page_evidence(
    left: &DuplicatePageHash,
    right: &DuplicatePageHash,
    profile: &HashProfile,
) -> Option<DuplicatePagePair> {
    page_metric(left, right, profile).map(|metric| metric.pair)
}

fn ratio_similarity(left: f64, right: f64, scale: f64) -> f64 {
    (1.0 - (left - right).abs() / scale).clamp(0.0, 1.0)
}

fn hex_hamming_distance(left: &str, right: &str) -> Option<u32> {
    if left.len() != right.len() || !left.len().is_multiple_of(2) {
        return None;
    }
    let mut distance = 0_u32;
    for index in (0..left.len()).step_by(2) {
        let left = u8::from_str_radix(&left[index..index + 2], 16).ok()?;
        let right = u8::from_str_radix(&right[index..index + 2], 16).ok()?;
        distance += (left ^ right).count_ones();
    }
    Some(distance)
}

fn central_detail_distance(left: &str, right: &str) -> Option<u32> {
    if left.len() != 256 || right.len() != 256 {
        return None;
    }
    let mut distance = 0_u32;
    for y in 7..25_usize {
        for x in 7..25_usize {
            let bit = y * 32 + x;
            let byte_index = bit / 8;
            let mask = 1_u8 << (bit % 8);
            let left = u8::from_str_radix(&left[byte_index * 2..byte_index * 2 + 2], 16).ok()?;
            let right = u8::from_str_radix(&right[byte_index * 2..byte_index * 2 + 2], 16).ok()?;
            distance += u32::from((left & mask) != (right & mask));
        }
    }
    Some(distance)
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::{fs, hint::black_box, time::Instant};

    use image::{
        codecs::jpeg::JpegEncoder, DynamicImage, ExtendedColorType, GrayImage, ImageFormat, Luma,
    };
    use sha2::{Digest, Sha256};

    use crate::domain::{ArtifactSha256, GalleryId, SourcePageNumber};

    use super::*;

    fn gallery(gallery_id: i64, page_count: u32) -> DuplicateGalleryRef {
        DuplicateGalleryRef {
            gallery_id: GalleryId::new(gallery_id).unwrap(),
            entry_id: format!("entry-{gallery_id}"),
            title: format!("Gallery {gallery_id}"),
            artist: None,
            group: None,
            page_count,
        }
    }

    fn exact_hash(gallery_id: i64, source_page: u32, digest_seed: u64) -> DuplicatePageHash {
        DuplicatePageHash {
            entry_id: format!("entry-{gallery_id}"),
            gallery_id: GalleryId::new(gallery_id).unwrap(),
            source_page_number: SourcePageNumber::new(source_page).unwrap(),
            profile_version: 1,
            artifact_sha256: ArtifactSha256::new(format!("{digest_seed:064x}")).unwrap(),
            coarse_d_hash: digest_seed,
            detail_d_hash_hex: "00".repeat(128),
            p_hash: digest_seed,
            mean_luma: 127.0,
            std_dev: 45.0,
            non_uniform_ratio: 0.7,
            edge_density: 0.2,
            width: 100,
            height: 100,
            low_information: false,
        }
    }

    fn contained_artifacts() -> (HashedArtifact, HashedArtifact) {
        let parent_pages = (1..=20)
            .map(|page| exact_hash(1, page, u64::from(page)))
            .collect::<Vec<_>>();
        let candidate_pages = (1..=10)
            .map(|page| exact_hash(2, page, u64::from(page + 5)))
            .collect::<Vec<_>>();
        (
            HashedArtifact {
                gallery: gallery(1, 20),
                pages: parent_pages,
            },
            HashedArtifact {
                gallery: gallery(2, 10),
                pages: candidate_pages,
            },
        )
    }

    #[test]
    fn containment_uses_full_shorter_side_in_both_input_orders() {
        let (long, short) = contained_artifacts();
        for (left, right) in [(&long, &short), (&short, &long)] {
            let record = analyze_artifact_pair("run", left, right, &HashProfile::current(), None)
                .expect("ten-page gallery is contained in twenty-page gallery");
            assert_eq!(record.candidate.relation, DuplicateRelation::Contains);
            assert_eq!(record.candidate.parent.gallery_id.get(), 1);
            assert_eq!(record.candidate.candidate.gallery_id.get(), 2);
            assert_eq!(record.candidate.matched_pages, 10);
            assert_eq!(record.candidate.parent_coverage, 0.5);
            assert_eq!(record.candidate.candidate_coverage, 1.0);
        }
    }

    #[test]
    fn alignment_is_monotonic_and_never_reuses_a_page() {
        let parent = HashedArtifact {
            gallery: gallery(1, 2),
            pages: vec![exact_hash(1, 1, 1), exact_hash(1, 2, 2)],
        };
        let candidate = HashedArtifact {
            gallery: gallery(2, 3),
            pages: vec![
                exact_hash(2, 1, 1),
                exact_hash(2, 2, 1),
                exact_hash(2, 3, 2),
            ],
        };
        let record =
            analyze_artifact_pair("run", &parent, &candidate, &HashProfile::current(), None)
                .expect("gap-tolerant alignment");
        assert_eq!(record.page_pairs.len(), 2);
        assert!(record
            .page_pairs
            .windows(2)
            .all(
                |pair| pair[0].parent_source_page < pair[1].parent_source_page
                    && pair[0].candidate_source_page < pair[1].candidate_source_page
            ));
    }

    #[test]
    fn visually_similar_blank_pages_are_rejected_without_exact_sha() {
        let first = GrayImage::from_pixel(128, 128, Luma([248]));
        let second = GrayImage::from_pixel(256, 192, Luma([245]));
        let first = png_bytes(first);
        let second = png_bytes(second);
        let first = computed_hash(1, &first);
        let second = computed_hash(2, &second);
        assert!(first.low_information && second.low_information);
        assert!(page_metric(&first, &second, &HashProfile::current()).is_none());
    }

    #[test]
    fn unrelated_high_contrast_black_and_white_pages_are_not_visual_matches() {
        let vertical = GrayImage::from_fn(256, 256, |x, _| Luma([if x < 128 { 8 } else { 248 }]));
        let horizontal = GrayImage::from_fn(256, 256, |_, y| Luma([if y < 128 { 8 } else { 248 }]));
        let vertical = computed_hash(1, &png_bytes(vertical));
        let horizontal = computed_hash(2, &png_bytes(horizontal));
        assert!(!vertical.low_information && !horizontal.low_information);
        assert!(page_metric(&vertical, &horizontal, &HashProfile::current()).is_none());
    }

    #[test]
    fn recompressed_resized_artwork_is_translation_visual_evidence() {
        let source = scene_image(320, 480, false);
        let translated = scene_image(640, 960, true);
        let source_hash = computed_hash(1, &png_bytes(source));
        let translated_hash = computed_hash(2, &jpeg_bytes(&translated, 67));
        let metric = page_metric(&source_hash, &translated_hash, &HashProfile::current())
            .expect("resolution, recompression, and a small text-like overlay remain similar");
        assert!(!metric.pair.exact_sha256);
        assert!(!metric.pair.low_information);

        let mut source_second = source_hash.clone();
        source_second.source_page_number = SourcePageNumber::new(2).unwrap();
        let mut translated_second = translated_hash.clone();
        translated_second.source_page_number = SourcePageNumber::new(2).unwrap();
        let first = HashedArtifact {
            gallery: gallery(1, 2),
            pages: vec![source_hash, source_second],
        };
        let second = HashedArtifact {
            gallery: gallery(2, 2),
            pages: vec![translated_hash, translated_second],
        };
        let record = analyze_artifact_pair("run", &first, &second, &HashProfile::current(), None)
            .expect("translation-like gallery relation");
        assert_eq!(
            record.candidate.relation,
            DuplicateRelation::TranslationVisual
        );
    }

    #[test]
    fn a_salient_scene_change_is_not_a_strong_visual_page_match() {
        let original = scene_image(320, 480, false);
        let mut changed = original.clone();
        for y in 190..290 {
            for x in 115..205 {
                let pixel = changed.get_pixel_mut(x, y);
                pixel.0[0] = 255 - pixel.0[0];
            }
        }
        let original = computed_hash(1, &png_bytes(original));
        let changed = computed_hash(2, &png_bytes(changed));
        assert_ne!(original.artifact_sha256, changed.artifact_sha256);
        assert!(page_metric(&original, &changed, &HashProfile::current()).is_none());
    }

    #[test]
    fn two_shared_pages_or_watermarks_do_not_make_whole_galleries_duplicates() {
        let parent = HashedArtifact {
            gallery: gallery(1, 10),
            pages: (1..=10)
                .map(|page| exact_hash(1, page, u64::from(page)))
                .collect(),
        };
        let mut candidate_pages = (1..=10)
            .map(|page| {
                let mut hash = exact_hash(2, page, u64::from(page + 100));
                hash.low_information = true;
                hash
            })
            .collect::<Vec<_>>();
        candidate_pages[0] = exact_hash(2, 1, 1);
        candidate_pages[1] = exact_hash(2, 2, 2);
        let candidate = HashedArtifact {
            gallery: gallery(2, 10),
            pages: candidate_pages,
        };
        assert!(
            analyze_artifact_pair("run", &parent, &candidate, &HashProfile::current(), None,)
                .is_none()
        );
    }

    fn png_bytes(image: GrayImage) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image)
            .write_to(&mut cursor, ImageFormat::Png)
            .unwrap();
        cursor.into_inner()
    }

    fn computed_hash(gallery_id: i64, bytes: &[u8]) -> DuplicatePageHash {
        let digest = format!("{:x}", Sha256::digest(bytes));
        compute_page_hash(
            &format!("entry-{gallery_id}"),
            GalleryId::new(gallery_id).unwrap(),
            SourcePageNumber::new(1).unwrap(),
            ArtifactSha256::new(digest).unwrap(),
            bytes,
            &HashProfile::current(),
        )
        .unwrap()
    }

    #[test]
    #[ignore = "manual local CPU and storage-cache profile; production runs emit full DB timing"]
    fn profile_duplicate_hash_and_compare_stages() {
        const PAGES_PER_ARTIFACT: u32 = 48;
        let bytes = png_bytes(scene_image(640, 960, false));
        let temporary = tempfile::tempdir().unwrap();
        let image_path = temporary.path().join("profile-page.png");
        fs::write(&image_path, &bytes).unwrap();

        let read_started = Instant::now();
        for _ in 0..PAGES_PER_ARTIFACT * 2 {
            let loaded = fs::read(&image_path).unwrap();
            black_box(loaded.len());
        }
        let image_read = read_started.elapsed();

        let artifact_sha = ArtifactSha256::new(format!("{:x}", Sha256::digest(&bytes))).unwrap();
        let hash_started = Instant::now();
        let mut artifact_pages = [Vec::new(), Vec::new()];
        for gallery_id in 1..=2_i64 {
            for source_page in 1..=PAGES_PER_ARTIFACT {
                artifact_pages[(gallery_id - 1) as usize].push(
                    compute_page_hash(
                        &format!("profile-entry-{gallery_id}"),
                        GalleryId::new(gallery_id).unwrap(),
                        SourcePageNumber::new(source_page).unwrap(),
                        artifact_sha.clone(),
                        &bytes,
                        &HashProfile::current(),
                    )
                    .unwrap(),
                );
            }
        }
        let hash_compute = hash_started.elapsed();

        let left = HashedArtifact {
            gallery: gallery(1, PAGES_PER_ARTIFACT),
            pages: artifact_pages[0].clone(),
        };
        let right = HashedArtifact {
            gallery: gallery(2, PAGES_PER_ARTIFACT),
            pages: artifact_pages[1].clone(),
        };
        let compare_started = Instant::now();
        let record =
            analyze_artifact_pair("profile-run", &left, &right, &HashProfile::current(), None)
                .expect("identical synthetic artifacts should produce a comparison record");
        let hash_compare = compare_started.elapsed();
        assert_eq!(record.candidate.matched_pages, PAGES_PER_ARTIFACT);

        eprintln!(
            "duplicate_stage_profile pages_per_artifact={} encoded_bytes={} image_read_us={} hash_compute_us={} hash_compare_us={}",
            PAGES_PER_ARTIFACT,
            bytes.len(),
            image_read.as_micros(),
            hash_compute.as_micros(),
            hash_compare.as_micros(),
        );
    }

    fn scene_image(width: u32, height: u32, overlay: bool) -> GrayImage {
        GrayImage::from_fn(width, height, |x, y| {
            let nx = x as f64 / width as f64;
            let ny = y as f64 / height as f64;
            let mut value = 35.0 + nx * 90.0 + ny * 55.0;
            let circle = (nx - 0.35).powi(2) + (ny - 0.30).powi(2) < 0.12_f64.powi(2);
            if circle {
                value = 224.0;
            }
            if ((nx - ny * 0.58) * 18.0).abs() < 0.7 {
                value = 18.0;
            }
            if (nx * 8.0).fract() < 0.08 || (ny * 11.0).fract() < 0.055 {
                value = (value + 58.0).min(250.0);
            }
            if overlay && ny > 0.84 {
                value = if (ny * 90.0).fract() < 0.28 {
                    42.0
                } else {
                    232.0
                };
            }
            Luma([value.round() as u8])
        })
    }

    fn jpeg_bytes(image: &GrayImage, quality: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, quality)
            .encode(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::L8,
            )
            .unwrap();
        bytes
    }
}
