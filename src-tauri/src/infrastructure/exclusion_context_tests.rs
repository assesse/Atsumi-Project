use super::*;

fn fixture() -> SqliteRepository {
    let repository = SqliteRepository::open_in_memory().unwrap();
    {
        let db = repository.connection().unwrap();
        for id in 1..=4 {
            db.execute("INSERT INTO galleries(gallery_id,revision,title,source_page_count) VALUES(?1,0,?2,1)", params![id, format!("Album {id}")]).unwrap();
            db.execute("INSERT INTO download_entries(entry_id,gallery_id,revision,state,progress,created_at,updated_at) VALUES(?1,?2,0,'completed',100,'now','now')", params![format!("entry-{id}"),id]).unwrap();
        }
    }
    repository
}

fn review(db: &Connection, id: &str, incoming: i64, candidates: &[i64]) {
    db.execute("INSERT INTO download_overlap_reviews(review_id,entry_id,incoming_gallery_id,revision,state,profile_version,policy_version,incoming_fingerprint,created_at,updated_at) VALUES(?1,?2,?3,1,'resolved',1,1,?4,'now','now')",
        params![id,format!("entry-{incoming}"),incoming,"a".repeat(64)]).unwrap();
    for (index, candidate) in candidates.iter().enumerate() {
        db.execute("INSERT INTO download_overlap_candidates(candidate_id,review_id,existing_entry_id,existing_gallery_id,existing_fingerprint,relation,confidence,matched_pages,exact_pages,visual_pages,existing_coverage,incoming_coverage,existing_unique_pages,incoming_unique_pages,longest_aligned_run,rank) VALUES(?1,?2,?3,?4,?5,'near_equivalent',1,1,1,0,1,1,0,0,1,?6)",
            params![format!("{id}-{candidate}"),id,format!("entry-{candidate}"),candidate,"b".repeat(64),index+1]).unwrap();
    }
}

fn decision(
    db: &Connection,
    id: &str,
    review: &str,
    candidate: Option<&str>,
    action: &str,
    excluded: i64,
) {
    db.execute("INSERT INTO download_overlap_decisions(decision_id,review_id,review_revision,candidate_id,action,created_at) VALUES(?1,?2,1,?3,?4,'now')", params![id,review,candidate,action]).unwrap();
    db.execute("INSERT INTO duplicate_hidden_galleries(gallery_id,decision_id,created_at) VALUES(?1,?2,'now')",params![excluded,id]).unwrap();
}

fn context(repository: &SqliteRepository, id: i64) -> crate::domain::ExplorationExclusionContext {
    repository
        .exploration_exclusion_context(GalleryId::new(id).unwrap())
        .unwrap()
}

#[test]
fn selected_existing_only_links_to_its_actual_keeper_and_restoration_removes_the_link() {
    let repository = fixture();
    {
        let db = repository.connection().unwrap();
        review(&db, "review", 1, &[2, 3]);
        decision(
            &db,
            "decision",
            "review",
            Some("review-2"),
            "remove_existing_continue",
            2,
        );
    }
    let selected = context(&repository, 2);
    assert_eq!(selected.retained_gallery.unwrap().gallery_id.get(), 1);
    assert_eq!(selected.review_id.as_deref(), Some("review"));
    assert_eq!(selected.review_gallery_id.unwrap().get(), 1);
    assert_eq!(selected.reasons.len(), 1);
    let other = context(&repository, 3);
    assert!(
        other.retained_gallery.is_none() && other.review_id.is_none() && other.reasons.is_empty()
    );
    repository
        .exploration_exclusions_restore(&[GalleryId::new(2).unwrap()])
        .unwrap();
    assert!(context(&repository, 2).retained_gallery.is_none());
    assert!(context(&repository, 2).reasons.is_empty());
}

#[test]
fn incoming_removal_never_selects_an_unrelated_candidate_and_follows_later_replacements() {
    let repository = fixture();
    {
        let db = repository.connection().unwrap();
        review(&db, "first", 1, &[2, 3]);
        decision(
            &db,
            "first-decision",
            "first",
            Some("first-3"),
            "remove_incoming",
            1,
        );
    }
    assert_eq!(
        context(&repository, 1)
            .retained_gallery
            .unwrap()
            .gallery_id
            .get(),
        3
    );
    {
        let db = repository.connection().unwrap();
        review(&db, "second", 4, &[3]);
        decision(
            &db,
            "second-decision",
            "second",
            Some("second-3"),
            "remove_existing_continue",
            3,
        );
    }
    let result = context(&repository, 1);
    assert_eq!(result.retained_gallery.unwrap().gallery_id.get(), 4);
    assert_eq!(result.review_id.as_deref(), Some("first"));
    {
        let db = repository.connection().unwrap();
        review(&db, "cycle", 1, &[4]);
        decision(
            &db,
            "cycle-decision",
            "cycle",
            Some("cycle-4"),
            "remove_existing_continue",
            4,
        );
    }
    assert!(context(&repository, 1).retained_gallery.is_none());
}

#[test]
fn ambiguous_legacy_decision_has_evidence_but_does_not_guess_a_keeper() {
    let repository = fixture();
    {
        let db = repository.connection().unwrap();
        review(&db, "review", 1, &[2, 3]);
        decision(&db, "decision", "review", None, "cancel_incoming", 1);
    }
    let result = context(&repository, 1);
    assert_eq!(result.review_id.as_deref(), Some("review"));
    assert!(result.retained_gallery.is_none());
}

#[test]
fn applied_merge_links_donor_to_target_and_manual_quarantine_has_its_own_reason() {
    let repository = fixture();
    {
        let db = repository.connection().unwrap();
        review(&db, "review", 1, &[2, 3]);
        db.execute_batch(r#"
            INSERT INTO overlap_page_merges VALUES('merge','review','entry-1','entry-2','applied','{"exclude_source":true}','now','now');
            INSERT INTO duplicate_hidden_galleries VALUES(2,'merge','now');
            UPDATE download_entries SET state='quarantined' WHERE gallery_id=4;
            INSERT INTO quarantine_records(record_id,entry_id,original_relative_path,quarantine_relative_path,reason,state,created_at)
            VALUES('quarantine','entry-4','gallery-4','quarantine/gallery-4','사용자 격리','quarantined','now');
        "#).unwrap();
    }
    assert_eq!(
        context(&repository, 2)
            .retained_gallery
            .unwrap()
            .gallery_id
            .get(),
        1
    );
    let quarantined = context(&repository, 4);
    assert!(quarantined.quarantined);
    assert_eq!(quarantined.quarantine_entry_id.as_deref(), Some("entry-4"));
    assert_eq!(quarantined.reasons[0].detail, "사용자 격리");
    assert!(quarantined.retained_gallery.is_none());
    {
        let db = repository.connection().unwrap();
        db.execute(
            "UPDATE download_entries SET state='quarantined' WHERE gallery_id=1",
            [],
        )
        .unwrap();
    }
    assert!(context(&repository, 2).retained_gallery.is_none());
}
