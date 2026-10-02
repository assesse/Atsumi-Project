use super::*;
use crate::domain::{DownloadOverlapCandidate, DownloadOverlapGalleryRef, SourcePageNumber};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageSelection {
    pub existing: Vec<u32>,
    pub incoming: Vec<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ComposedPage {
    pub donor: bool,
    pub page: u32,
}

pub(super) fn plan(
    candidate: &DownloadOverlapCandidate,
    request: &DownloadOverlapMergeRequest,
    source: &ArtifactBundle,
    target: &ArtifactBundle,
) -> Result<Vec<ComposedPage>, ApplicationError> {
    let Some(selected) = &request.selected_pages else {
        return Ok(vec![]);
    };
    let (donor, keep) = match request.source_side {
        DownloadOverlapMergeSide::Existing => (&selected.existing, &selected.incoming),
        DownloadOverlapMergeSide::Incoming => (&selected.incoming, &selected.existing),
    };
    if request.source_pages.len() != donor.len()
        || request.source_pages.iter().collect::<BTreeSet<_>>()
            != donor.iter().collect::<BTreeSet<_>>()
    {
        return Err(invalid("The donor selection changed"));
    }
    if donor.is_empty() && keep.is_empty() || donor.len() + keep.len() > 4000 {
        return Err(invalid("Select between 1 and 4000 pages before merging"));
    }
    for (pages, count) in [(donor, source.pages.len()), (keep, target.pages.len())] {
        if pages.iter().any(|p| *p == 0 || *p as usize > count)
            || pages.iter().collect::<BTreeSet<_>>().len() != pages.len()
        {
            return Err(invalid("Invalid or repeated selection page"));
        }
    }
    let mut pairs = candidate
        .page_pairs
        .iter()
        .map(|p| match request.source_side {
            DownloadOverlapMergeSide::Existing => (p.existing_source_page, p.incoming_source_page),
            DownloadOverlapMergeSide::Incoming => (p.incoming_source_page, p.existing_source_page),
        })
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    let (expected_source_unique, expected_target_unique) = match request.source_side {
        DownloadOverlapMergeSide::Existing => (
            candidate.existing_unique_pages,
            candidate.incoming_unique_pages,
        ),
        DownloadOverlapMergeSide::Incoming => (
            candidate.incoming_unique_pages,
            candidate.existing_unique_pages,
        ),
    };
    if candidate.matched_pages as usize != pairs.len()
        || source.pages.len().checked_sub(pairs.len()) != Some(expected_source_unique as usize)
        || target.pages.len().checked_sub(pairs.len()) != Some(expected_target_unique as usize)
    {
        return Err(invalid(
            "Stored correspondence counts changed; compare again",
        ));
    }
    if pairs.is_empty()
        || pairs.iter().any(|(s, t)| {
            *s == 0
                || *s as usize > source.pages.len()
                || *t == 0
                || *t as usize > target.pages.len()
        })
        || pairs
            .windows(2)
            .any(|p| p[0].0 >= p[1].0 || p[0].1 >= p[1].1)
        || pairs
            .iter()
            .any(|(s, t)| donor.contains(s) && keep.contains(t))
    {
        return Err(invalid(
            "Select only one side of each verified, ordered page pair",
        ));
    }
    // Retain the destination's unmatched run, then append selected donor pages
    // in the same gap before the next verified anchor. Neither run is reordered.
    let mut output = vec![];
    let (mut s, mut t) = (1, 1);
    for (sp, tp) in pairs {
        while t < tp {
            output.push(ComposedPage {
                donor: false,
                page: t,
            });
            t += 1;
        }
        while s < sp {
            if donor.contains(&s) {
                output.push(ComposedPage {
                    donor: true,
                    page: s,
                });
            }
            s += 1;
        }
        output.push(if donor.contains(&s) {
            ComposedPage {
                donor: true,
                page: s,
            }
        } else {
            ComposedPage {
                donor: false,
                page: t,
            }
        });
        s += 1;
        t += 1;
    }
    while t <= target.artifact.expected_page_count {
        output.push(ComposedPage {
            donor: false,
            page: t,
        });
        t += 1;
    }
    while s <= source.artifact.expected_page_count {
        if donor.contains(&s) {
            output.push(ComposedPage {
                donor: true,
                page: s,
            });
        }
        s += 1;
    }
    Ok(output)
}

pub(super) fn stage(
    journal: &Journal,
    source_root: &Path,
    source: &ArtifactBundle,
    target: &mut ArtifactBundle,
    staging: &Path,
) -> Result<(), ApplicationError> {
    let original = target.clone();
    let mut pages = Vec::with_capacity(journal.composition.len());
    for (index, choice) in journal.composition.iter().enumerate() {
        let number = (index + 1) as u32;
        let (bundle, root) = if choice.donor {
            (source, source_root)
        } else {
            (&original, Path::new(&journal.target_root))
        };
        let from = page(bundle, choice.page)?;
        let mut next = from.clone();
        next.entry_id = target.artifact.entry_id.clone();
        next.page_id.gallery_id = target.gallery.id;
        next.page_id.source_page_number = SourcePageNumber::new(number)?;
        next.relative_path = ArtifactRelativePath::new(format!(
            "{}/{number:04}.webp",
            target.artifact.relative_directory
        ))?;
        next.source_revision = Some(format!("local-composition:{}:{number}", journal.merge_id));
        copy_verified_replace(
            &checked_path(root, from.relative_path.as_str())?,
            &staging.join(format!("{number:04}.webp")),
            from.sha256.as_ref().unwrap().as_str(),
            from.byte_length.unwrap(),
        )?;
        pages.push(next);
    }
    target.pages = pages;
    target.artifact.expected_page_count = journal.composition.len() as u32;
    target.gallery.metadata.source_page_count = journal.composition.len() as u32;
    target.validate()?;
    Ok(())
}

pub(super) fn commit(c: &Connection, j: &Journal) -> Result<(), ApplicationError> {
    // Stage both identities before deleting target rows (hashes have FK cascade).
    c.execute_batch("CREATE TEMP TABLE merge_page_cache AS SELECT * FROM download_pages WHERE 0; CREATE TEMP TABLE merge_hash_cache AS SELECT * FROM duplicate_page_hashes WHERE 0;").map_err(sql)?;
    c.execute(
        "INSERT INTO merge_page_cache SELECT * FROM download_pages WHERE entry_id IN(?1,?2)",
        params![j.target_entry_id, j.source_entry_id],
    )
    .map_err(sql)?;
    c.execute(
        "INSERT INTO merge_hash_cache SELECT * FROM duplicate_page_hashes WHERE entry_id IN(?1,?2)",
        params![j.target_entry_id, j.source_entry_id],
    )
    .map_err(sql)?;
    c.execute(
        "DELETE FROM download_pages WHERE entry_id=?1",
        [&j.target_entry_id],
    )
    .map_err(sql)?;
    for (index, choice) in j.composition.iter().enumerate() {
        let number = (index + 1) as u32;
        let old_entry = if choice.donor {
            &j.source_entry_id
        } else {
            &j.target_entry_id
        };
        let changed=c.execute("INSERT INTO download_pages(entry_id,gallery_id,source_page_number,relative_path,state,byte_length,sha256,storage_format,source_revision,verified_at,excluded) SELECT ?1,?2,?3,?4,state,byte_length,sha256,storage_format,?5,strftime('%Y-%m-%dT%H:%M:%fZ','now'),excluded FROM merge_page_cache WHERE entry_id=?6 AND source_page_number=?7",params![j.target_entry_id,j.target_gallery_id,number,format!("{}/{number:04}.webp",j.target_directory),format!("local-composition:{}:{number}",j.merge_id),old_entry,choice.page]).map_err(sql)?;
        if changed != 1 {
            return Err(invalid("A composed page disappeared before commit"));
        }
        c.execute("INSERT INTO duplicate_page_hashes(entry_id,gallery_id,source_page_number,profile_version,artifact_sha256,coarse_d_hash_hex,detail_d_hash_hex,p_hash_hex,mean_luma,std_dev,non_uniform_ratio,edge_density,width,height,low_information,computed_at) SELECT ?1,?2,?3,h.profile_version,h.artifact_sha256,h.coarse_d_hash_hex,h.detail_d_hash_hex,h.p_hash_hex,h.mean_luma,h.std_dev,h.non_uniform_ratio,h.edge_density,h.width,h.height,h.low_information,h.computed_at FROM merge_hash_cache h JOIN merge_page_cache p ON p.entry_id=h.entry_id AND p.source_page_number=h.source_page_number AND p.sha256=h.artifact_sha256 WHERE h.entry_id=?4 AND h.source_page_number=?5",params![j.target_entry_id,j.target_gallery_id,number,old_entry,choice.page]).map_err(sql)?;
    }
    c.execute(
        "UPDATE download_artifacts SET expected_page_count=?2 WHERE entry_id=?1",
        params![j.target_entry_id, j.composition.len()],
    )
    .map_err(sql)?;
    c.execute("UPDATE galleries SET source_page_count=?2,source_revision=?3,revision=revision+1 WHERE gallery_id=?1",params![j.target_gallery_id,j.composition.len(),format!("local-composition:{}",j.merge_id)]).map_err(sql)?;
    let old_cover: Option<u32> = c
        .query_row(
            "SELECT manual_source_page FROM gallery_previews WHERE gallery_id=?1",
            [j.target_gallery_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .flatten();
    if let Some(old) = old_cover {
        let new_page = j
            .composition
            .iter()
            .position(|p| !p.donor && p.page == old)
            .map(|index| (index + 1) as u32);
        c.execute(
            "UPDATE gallery_previews SET mode=?2,manual_source_page=?3 WHERE gallery_id=?1",
            params![
                j.target_gallery_id,
                if new_page.is_some() {
                    "manual"
                } else {
                    "automatic"
                },
                new_page
            ],
        )
        .map_err(sql)?;
    }
    c.execute_batch("DROP TABLE merge_page_cache; DROP TABLE merge_hash_cache;")
        .map_err(sql)?;
    Ok(())
}

fn uncensored(title: &str) -> bool {
    let title = title
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "uncensored",
        "decensored",
        "uncen",
        "無修正",
        "无修正",
        "無碼",
        "无码",
    ]
    .iter()
    .any(|m| title.contains(m))
}
pub(crate) fn automatic_source(
    incoming: &DownloadOverlapGalleryRef,
    c: &DownloadOverlapCandidate,
) -> Option<DownloadOverlapMergeSide> {
    let (side, small, big, unique, coverage) = if incoming.page_count < c.existing.page_count {
        (
            DownloadOverlapMergeSide::Incoming,
            incoming,
            &c.existing,
            c.incoming_unique_pages,
            c.incoming_coverage,
        )
    } else {
        (
            DownloadOverlapMergeSide::Existing,
            &c.existing,
            incoming,
            c.existing_unique_pages,
            c.existing_coverage,
        )
    };
    if small.page_count >= big.page_count
        || !uncensored(&small.title)
        || uncensored(&big.title)
        || small.page_count < 1
        || unique != 0
        || coverage < 1.0
        || c.matched_pages != small.page_count
        || c.page_pairs.len() != small.page_count as usize
        || c.confidence < 0.85
        || c.longest_aligned_run < 2.min(small.page_count)
        || c.page_pairs.iter().filter(|p| !p.low_information).count() as f64
            / (small.page_count as f64)
            < 0.75
        || (small.page_count <= 3 && c.exact_pages != small.page_count)
    {
        return None;
    }
    let mut pairs = c
        .page_pairs
        .iter()
        .map(|p| (p.existing_source_page, p.incoming_source_page))
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    if pairs
        .windows(2)
        .any(|p| p[0].0 >= p[1].0 || p[0].1 >= p[1].1)
    {
        return None;
    }
    Some(side)
}
pub(super) fn validate_automatic(
    review: &DownloadOverlapReview,
    c: &DownloadOverlapCandidate,
    r: &DownloadOverlapMergeRequest,
    incoming: &ArtifactBundle,
    existing: &ArtifactBundle,
) -> Result<(), ApplicationError> {
    if automatic_source(&review.incoming, c) != Some(r.source_side)
        || !r.exclude_source
        || r.selected_pages.is_some()
        || incoming.gallery.metadata.language.is_none()
        || incoming.gallery.metadata.language != existing.gallery.metadata.language
    {
        return Err(invalid(
            "Automatic merging requires complete same-language containment and an uncensored donor",
        ));
    }
    let count = if r.source_side == DownloadOverlapMergeSide::Incoming {
        review.incoming.page_count
    } else {
        c.existing.page_count
    };
    if r.source_pages.iter().copied().collect::<BTreeSet<_>>() != (1..=count).collect() {
        return Err(invalid("Automatic merge must preserve every donor page"));
    }
    Ok(())
}
