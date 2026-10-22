//! Complete-catalog grouping; opaque cursors bind an immutable metadata revision.
use super::{broadcast, LibraryItem, MediaJob};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{cmp::Reverse, collections::BTreeMap};

const CURSOR_LIMIT: usize = 512;
const CURSOR_ERROR: &str = "목록 위치를 확인해 주세요.";

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LibraryEntry {
    Video {
        key: String,
        item: LibraryItem,
    },
    RecordingGroup {
        key: String,
        group_revision: String,
        representative: LibraryItem,
        file_count: usize,
        total_bytes: u64,
        latest_completed_at: u64,
    },
}
impl LibraryEntry {
    pub(super) fn item(&self) -> &LibraryItem {
        match self {
            Self::Video { item, .. } => item,
            Self::RecordingGroup { representative, .. } => representative,
        }
    }
    pub(super) fn item_mut(&mut self) -> &mut LibraryItem {
        match self {
            Self::Video { item, .. } => item,
            Self::RecordingGroup { representative, .. } => representative,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GroupsPage {
    Ready {
        catalog_revision: String,
        entries: Vec<LibraryEntry>,
        cursor: Option<String>,
    },
    Changed {
        catalog_revision: String,
    },
}
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MembersPage {
    Ready {
        group: LibraryEntry,
        items: Vec<LibraryItem>,
        cursor: Option<String>,
    },
    Changed {
        group_revision: String,
    },
    Missing,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Boundary {
    v: u8,
    p: String,
    r: String,
    t: u64,
    k: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    g: Option<String>,
}
fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn key_valid(key: &str, group_only: bool) -> bool {
    key.strip_prefix("g1_")
        .or_else(|| (!group_only).then(|| key.strip_prefix("v1_")).flatten())
        .is_some_and(digest_valid)
}
fn provider_valid(provider: &str) -> Result<(), String> {
    if matches!(provider, "youtube" | "soop") {
        Ok(())
    } else {
        Err("서비스를 확인해 주세요.".into())
    }
}
impl Boundary {
    fn decode(cursor: &str, provider: &str, group: Option<&str>) -> Result<Self, String> {
        if cursor.is_empty() || cursor.len() > CURSOR_LIMIT {
            return Err(CURSOR_ERROR.into());
        }
        let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| CURSOR_ERROR)?;
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| CURSOR_ERROR)?;
        let key_ok = if group.is_some() {
            uuid::Uuid::parse_str(&value.k).is_ok_and(|id| id.to_string() == value.k)
        } else {
            key_valid(&value.k, false)
        };
        if value.v != 1
            || value.p != provider
            || !digest_valid(&value.r)
            || value.g.as_deref() != group
            || !key_ok
        {
            return Err(CURSOR_ERROR.into());
        }
        Ok(value)
    }
    fn encode(
        provider: &str,
        revision: &str,
        time: u64,
        key: &str,
        group: Option<&str>,
    ) -> Result<String, String> {
        let boundary = Self {
            v: 1,
            p: provider.into(),
            r: revision.into(),
            t: time,
            k: key.into(),
            g: group.map(String::from),
        };
        let encoded =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&boundary).map_err(|_| CURSOR_ERROR)?);
        if encoded.len() > CURSOR_LIMIT {
            return Err(CURSOR_ERROR.into());
        }
        Ok(encoded)
    }
}
fn text(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value.as_bytes());
}
fn number(hash: &mut Sha256, value: u64) {
    hash.update(value.to_le_bytes());
}
fn identity(job: &MediaJob, item: &LibraryItem) -> String {
    let mut hash = Sha256::new();
    text(&mut hash, "atsumi-library-group-v1");
    text(&mut hash, &item.reference.provider);
    let verified = job.broadcast_id.as_deref().filter(|id| {
        item.reference == job.reference
            && item.broadcast_id.as_deref() == Some(*id)
            && broadcast::valid(&item.reference, id)
    });
    if let Some(id) = verified {
        text(&mut hash, "broadcast");
        text(&mut hash, id);
        if item.reference.provider == "soop" {
            text(&mut hash, &item.reference.external_id.to_ascii_lowercase());
        }
    } else {
        text(&mut hash, "individual");
        text(&mut hash, &item.library_id);
    }
    let prefix = if item.reference.kind == "live" {
        "g1"
    } else {
        "v1"
    };
    format!("{prefix}_{:x}", hash.finalize())
}
fn hash_item(hash: &mut Sha256, job: &MediaJob, item: &LibraryItem) {
    // Sidecars are deliberately not revision inputs: each requested page refreshes
    // its bounded statuses independently, after releasing the metadata lock.
    for value in [
        &item.library_id,
        &item.reference.provider,
        &item.reference.kind,
        &item.reference.external_id,
        &item.title,
        &item.channel_name,
        &item.sha256,
    ] {
        text(hash, value);
    }
    text(hash, item.broadcast_id.as_deref().unwrap_or(""));
    for value in [
        job.revision,
        item.revision,
        item.bytes,
        item.duration.to_bits(),
        item.completed_at,
        item.modified_millis,
    ] {
        number(hash, value);
    }
}
struct Aggregate<'a> {
    key: String,
    revision: String,
    members: Vec<(&'a MediaJob, &'a LibraryItem)>,
    representative: &'a LibraryItem,
    total_bytes: u64,
}
impl Aggregate<'_> {
    fn entry(&self) -> LibraryEntry {
        if self.representative.reference.kind == "live" {
            LibraryEntry::RecordingGroup {
                key: self.key.clone(),
                group_revision: self.revision.clone(),
                representative: self.representative.clone(),
                file_count: self.members.len(),
                total_bytes: self.total_bytes,
                latest_completed_at: self.representative.completed_at,
            }
        } else {
            LibraryEntry::Video {
                key: self.key.clone(),
                item: self.representative.clone(),
            }
        }
    }
}
struct Catalog<'a> {
    groups: BTreeMap<String, Aggregate<'a>>,
    revision: String,
}
fn catalog<'a>(
    jobs: impl Iterator<Item = &'a MediaJob>,
    provider: &str,
) -> Result<Catalog<'a>, String> {
    let mut groups: BTreeMap<String, Aggregate<'a>> = BTreeMap::new();
    for job in jobs.filter(|job| job.state == "completed" && job.reference.provider == provider) {
        let Some(item) = job
            .library
            .as_ref()
            .filter(|item| item.library_id == job.job_id && item.reference == job.reference)
        else {
            continue;
        };
        let key = identity(job, item);
        let group = groups.entry(key.clone()).or_insert_with(|| Aggregate {
            key,
            revision: String::new(),
            members: Vec::new(),
            representative: item,
            total_bytes: 0,
        });
        group.total_bytes = group
            .total_bytes
            .checked_add(item.bytes)
            .ok_or("파일 크기의 합계를 확인하지 못했습니다.")?;
        group.members.push((job, item));
        if (Reverse(item.completed_at), &item.library_id)
            < (
                Reverse(group.representative.completed_at),
                &group.representative.library_id,
            )
        {
            group.representative = item;
        }
    }
    let mut catalog_hash = Sha256::new();
    text(&mut catalog_hash, "atsumi-catalog-v1");
    text(&mut catalog_hash, provider);
    for group in groups.values_mut() {
        group
            .members
            .sort_unstable_by(|a, b| a.1.library_id.cmp(&b.1.library_id));
        let mut hash = Sha256::new();
        text(&mut hash, &group.key);
        for (job, item) in &group.members {
            hash_item(&mut hash, job, item);
        }
        group.revision = format!("{:x}", hash.finalize());
        text(&mut catalog_hash, &group.key);
        text(&mut catalog_hash, &group.revision);
    }
    Ok(Catalog {
        groups,
        revision: format!("{:x}", catalog_hash.finalize()),
    })
}

pub(super) fn page<'a>(
    jobs: impl Iterator<Item = &'a MediaJob>,
    provider: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<GroupsPage, String> {
    provider_valid(provider)?;
    let boundary = cursor
        .map(|cursor| Boundary::decode(cursor, provider, None))
        .transpose()?;
    let catalog = catalog(jobs, provider)?;
    if boundary
        .as_ref()
        .is_some_and(|value| value.r != catalog.revision)
    {
        return Ok(GroupsPage::Changed {
            catalog_revision: catalog.revision,
        });
    }
    let limit = limit.clamp(1, 100);
    let mut selected = BTreeMap::new();
    for group in catalog.groups.values() {
        let key = (
            Reverse(group.representative.completed_at),
            group.key.as_str(),
        );
        if boundary
            .as_ref()
            .is_some_and(|b| key <= (Reverse(b.t), b.k.as_str()))
        {
            continue;
        }
        selected.insert(key, group);
        if selected.len() > limit + 1 {
            selected.pop_last();
        }
    }
    let more = selected.len() > limit;
    if more {
        selected.pop_last();
    }
    let cursor = if more {
        selected
            .last_key_value()
            .map(|(_, group)| {
                Boundary::encode(
                    provider,
                    &catalog.revision,
                    group.representative.completed_at,
                    &group.key,
                    None,
                )
            })
            .transpose()?
    } else {
        None
    };
    Ok(GroupsPage::Ready {
        catalog_revision: catalog.revision,
        entries: selected.into_values().map(Aggregate::entry).collect(),
        cursor,
    })
}

pub(super) fn members<'a>(
    jobs: impl Iterator<Item = &'a MediaJob>,
    provider: &str,
    group_key: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<MembersPage, String> {
    provider_valid(provider)?;
    if !key_valid(group_key, true) {
        return Err(CURSOR_ERROR.into());
    }
    let boundary = cursor
        .map(|cursor| Boundary::decode(cursor, provider, Some(group_key)))
        .transpose()?;
    let catalog = catalog(jobs, provider)?;
    let Some(group) = catalog.groups.get(group_key) else {
        return Ok(MembersPage::Missing);
    };
    if boundary.as_ref().is_some_and(|b| b.r != group.revision) {
        return Ok(MembersPage::Changed {
            group_revision: group.revision.clone(),
        });
    }
    let limit = limit.clamp(1, 100);
    let mut selected = BTreeMap::new();
    for (_, item) in &group.members {
        let key = (Reverse(item.completed_at), item.library_id.as_str());
        if boundary
            .as_ref()
            .is_some_and(|b| key <= (Reverse(b.t), b.k.as_str()))
        {
            continue;
        }
        selected.insert(key, *item);
        if selected.len() > limit + 1 {
            selected.pop_last();
        }
    }
    let more = selected.len() > limit;
    if more {
        selected.pop_last();
    }
    let cursor = if more {
        selected
            .last_key_value()
            .map(|(_, item)| {
                Boundary::encode(
                    provider,
                    &group.revision,
                    item.completed_at,
                    &item.library_id,
                    Some(group_key),
                )
            })
            .transpose()?
    } else {
        None
    };
    Ok(MembersPage::Ready {
        group: group.entry(),
        items: selected.into_values().cloned().collect(),
        cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_media::{MediaRef, PlatformMediaService, PlatformMediaTools};
    use std::{collections::HashSet, path::PathBuf};

    fn job(id: u128, broadcast_id: Option<&str>, time: u64) -> MediaJob {
        let id = uuid::Uuid::from_u128(id).to_string();
        let reference = MediaRef {
            provider: "soop".into(),
            kind: "live".into(),
            external_id: "Fixture_1".into(),
        };
        MediaJob {
            job_id: id.clone(),
            request_id: id.clone(),
            reference: reference.clone(),
            broadcast_id: broadcast_id.map(String::from),
            revision: 3,
            state: "completed".into(),
            title: "same title".into(),
            channel_name: "fixture".into(),
            created_at: time,
            received_bytes: 10,
            total_bytes: Some(10),
            error: None,
            download_root: PathBuf::new(),
            library: Some(LibraryItem {
                library_id: id,
                revision: 3,
                reference,
                broadcast_id: broadcast_id.map(String::from),
                chat_archive: None,
                capture_outcome: None,
                title: "same title".into(),
                channel_name: "fixture".into(),
                bytes: 10,
                duration: 1.25,
                sha256: "0".repeat(64),
                completed_at: time,
                modified_millis: 0,
            }),
        }
    }
    fn reference(job: &mut MediaJob, provider: &str, kind: &str, id: &str) {
        job.reference = MediaRef {
            provider: provider.into(),
            kind: kind.into(),
            external_id: id.into(),
        };
        job.library.as_mut().unwrap().reference = job.reference.clone();
    }
    fn ready(page: GroupsPage) -> (String, Vec<LibraryEntry>, Option<String>) {
        match page {
            GroupsPage::Ready {
                catalog_revision,
                entries,
                cursor,
            } => (catalog_revision, entries, cursor),
            _ => panic!("expected ready"),
        }
    }
    fn key(entry: &LibraryEntry) -> &str {
        match entry {
            LibraryEntry::Video { key, .. } | LibraryEntry::RecordingGroup { key, .. } => key,
        }
    }
    fn revision(entry: &LibraryEntry) -> &str {
        match entry {
            LibraryEntry::RecordingGroup { group_revision, .. } => group_revision,
            _ => panic!("expected recording"),
        }
    }
    fn count(entry: &LibraryEntry) -> usize {
        match entry {
            LibraryEntry::RecordingGroup { file_count, .. } => *file_count,
            _ => 1,
        }
    }
    fn member_ready(page: MembersPage) -> (LibraryEntry, Vec<LibraryItem>, Option<String>) {
        match page {
            MembersPage::Ready {
                group,
                items,
                cursor,
            } => (group, items, cursor),
            _ => panic!("expected ready"),
        }
    }

    #[test]
    fn entire_135_file_broadcast_has_exact_summary_and_all_members() {
        let records: Vec<_> = (1..=135)
            .map(|id| job(id, Some("123"), id as u64))
            .collect();
        let (_, entries, cursor) = ready(page(records.iter(), "soop", None, 1).unwrap());
        assert_eq!(entries.len(), 1);
        assert!(cursor.is_none());
        match &entries[0] {
            LibraryEntry::RecordingGroup {
                file_count,
                total_bytes,
                latest_completed_at,
                representative,
                ..
            } => {
                assert_eq!(
                    (*file_count, *total_bytes, *latest_completed_at),
                    (135, 1350, 135)
                );
                assert_eq!(representative.library_id, records[134].job_id);
            }
            _ => panic!("not grouped"),
        }
        let group_key = key(&entries[0]);
        let mut cursor = None;
        let mut ids = HashSet::new();
        let mut sizes = Vec::new();
        loop {
            let (group, items, next) = member_ready(
                members(records.iter(), "soop", group_key, cursor.as_deref(), 100).unwrap(),
            );
            assert_eq!(count(&group), 135);
            assert_eq!(revision(&group), revision(&entries[0]));
            sizes.push(items.len());
            for item in items {
                assert!(ids.insert(item.library_id));
            }
            cursor = next;
            if cursor.is_none() {
                break;
            }
            assert!(cursor.as_ref().unwrap().len() <= CURSOR_LIMIT);
        }
        assert_eq!(sizes, [100, 35]);
        assert_eq!(ids.len(), 135);
    }

    #[test]
    fn identities_are_provider_broadcast_channel_and_file_scoped() {
        let mut records = vec![
            job(1, Some("123"), 1),
            job(2, Some("123"), 2),
            job(3, Some("124"), 3),
            job(4, None, 4),
            job(5, None, 5),
        ];
        reference(&mut records[1], "soop", "live", "fixture_1");
        let mut other_channel = job(6, Some("123"), 6);
        reference(&mut other_channel, "soop", "live", "different");
        records.push(other_channel);
        for id in 7..=8 {
            let mut vod = job(id, Some("123"), id as u64);
            reference(&mut vod, "soop", "video", "123");
            records.push(vod);
        }
        let mut invalid = job(9, Some("0123"), 9);
        invalid.library.as_mut().unwrap().broadcast_id = Some("123".into());
        records.push(invalid);
        let (_, entries, _) = ready(page(records.iter(), "soop", None, 100).unwrap());
        assert_eq!(entries.len(), 8);
        assert_eq!(entries.iter().filter(|entry| count(entry) == 2).count(), 1);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, LibraryEntry::Video { .. }))
                .count(),
            2
        );
        let stable_key = identity(&records[0], records[0].library.as_ref().unwrap());
        records[0].library.as_mut().unwrap().title = "renamed".into();
        records[0].library.as_mut().unwrap().completed_at = 900;
        assert_eq!(
            stable_key,
            identity(&records[0], records[0].library.as_ref().unwrap())
        );
        let mut yt1 = job(10, Some("jNQXAC9IVRw"), 1);
        reference(&mut yt1, "youtube", "live", "jNQXAC9IVRw");
        let mut yt2 = job(11, Some("jNQXAC9IVRw"), 2);
        reference(&mut yt2, "youtube", "live", "jNQXAC9IVRw");
        records.extend([yt1, yt2]);
        let (_, youtube, _) = ready(page(records.iter(), "youtube", None, 100).unwrap());
        assert_eq!(youtube.len(), 1);
        assert_eq!(count(&youtube[0]), 2);
        assert_ne!(key(&youtube[0]), stable_key);
    }

    #[test]
    fn group_keyset_is_deterministic_and_invalidates_insert_delete_or_metadata_changes() {
        let mut records: Vec<_> = (1..=135)
            .map(|id| job(id, Some(&id.to_string()), 100))
            .collect();
        let (revision, first, cursor) = ready(page(records.iter(), "soop", None, 100).unwrap());
        let (reversed_revision, reversed, _) =
            ready(page(records.iter().rev(), "soop", None, 100).unwrap());
        assert_eq!(revision, reversed_revision);
        assert_eq!(
            first.iter().map(key).collect::<Vec<_>>(),
            reversed.iter().map(key).collect::<Vec<_>>()
        );
        let (_, tail, end) = ready(page(records.iter(), "soop", cursor.as_deref(), 100).unwrap());
        assert_eq!(tail.len(), 35);
        assert!(end.is_none());
        let unique: HashSet<_> = first.iter().chain(tail.iter()).map(key).collect();
        assert_eq!(unique.len(), 135);
        assert!(page(records.iter(), "youtube", cursor.as_deref(), 1).is_err());
        records.push(job(500, Some("500"), 200));
        assert!(matches!(
            page(records.iter(), "soop", cursor.as_deref(), 1).unwrap(),
            GroupsPage::Changed { .. }
        ));
        records.pop();
        records.retain(|job| job.job_id != first.last().unwrap().item().library_id);
        assert!(matches!(
            page(records.iter(), "soop", cursor.as_deref(), 1).unwrap(),
            GroupsPage::Changed { .. }
        ));
        let (_, _, fresh_cursor) = ready(page(records.iter(), "soop", None, 1).unwrap());
        records[0].library.as_mut().unwrap().title = "new title".into();
        assert!(matches!(
            page(records.iter(), "soop", fresh_cursor.as_deref(), 1).unwrap(),
            GroupsPage::Changed { .. }
        ));
    }

    #[test]
    fn member_revision_is_local_and_first_page_is_coherent_after_catalog_change() {
        let mut records: Vec<_> = (1..=135).map(|id| job(id, Some("123"), 10)).collect();
        let (_, summary, _) = ready(page(records.iter(), "soop", None, 100).unwrap());
        let group_key = key(&summary[0]).to_owned();
        let (_, _, old_cursor) =
            member_ready(members(records.iter(), "soop", &group_key, None, 100).unwrap());
        records.push(job(1000, Some("999"), 500));
        assert!(matches!(
            members(
                records.iter(),
                "soop",
                &group_key,
                old_cursor.as_deref(),
                100
            )
            .unwrap(),
            MembersPage::Ready { .. }
        ));
        records.remove(0);
        assert!(matches!(
            members(
                records.iter(),
                "soop",
                &group_key,
                old_cursor.as_deref(),
                100
            )
            .unwrap(),
            MembersPage::Changed { .. }
        ));
        let (fresh, items, _) =
            member_ready(members(records.iter(), "soop", &group_key, None, 100).unwrap());
        assert_eq!(count(&fresh), 134);
        assert_eq!(items.len(), 100);
        assert_ne!(revision(&fresh), revision(&summary[0]));
        assert_eq!(
            fresh.item().library_id,
            records[0].job_id,
            "equal timestamp representative uses UUID"
        );
        let other_key = identity(
            records.last().unwrap(),
            records.last().unwrap().library.as_ref().unwrap(),
        );
        assert!(members(
            records.iter(),
            "soop",
            &other_key,
            old_cursor.as_deref(),
            100
        )
        .is_err());
        records.retain(|job| job.broadcast_id.as_deref() != Some("123"));
        assert!(matches!(
            members(
                records.iter(),
                "soop",
                &group_key,
                old_cursor.as_deref(),
                100
            )
            .unwrap(),
            MembersPage::Missing
        ));
    }

    #[test]
    fn cursors_limits_and_wire_tags_are_strict() {
        let records = [
            job(1, Some("123"), 1),
            job(2, Some("123"), 2),
            job(3, None, 3),
        ];
        let (catalog_revision, entries, cursor) =
            ready(page(records.iter(), "soop", None, 1).unwrap());
        assert!(cursor.as_ref().unwrap().len() <= 512);
        for malformed in [
            "".into(),
            "0".into(),
            "a".repeat(513),
            URL_SAFE_NO_PAD.encode(b"[]"),
            URL_SAFE_NO_PAD
                .encode(br#"{"v":1,"p":"soop","r":"abc","t":0,"k":"../../secret","extra":1}"#),
        ] {
            assert!(page(records.iter(), "soop", Some(&malformed), 1).is_err());
        }
        assert!(members(records.iter(), "soop", "../../secret", None, 100).is_err());
        assert!(page(records.iter(), "invalid", None, 1).is_err());
        let wire = serde_json::to_value(GroupsPage::Ready {
            catalog_revision: catalog_revision.clone(),
            entries: entries.clone(),
            cursor,
        })
        .unwrap();
        assert_eq!(wire["status"], "ready");
        assert!(wire["catalogRevision"].is_string());
        assert_eq!(wire["entries"][0]["type"], "recordingGroup");
        for field in [
            "groupRevision",
            "fileCount",
            "totalBytes",
            "latestCompletedAt",
        ] {
            assert!(!wire["entries"][0][field].is_null());
        }
        assert_eq!(
            serde_json::to_value(GroupsPage::Changed { catalog_revision }).unwrap()["status"],
            "changed"
        );
        let missing = serde_json::to_value(MembersPage::Missing).unwrap();
        assert_eq!(missing, serde_json::json!({"status":"missing"}));
        let changed = serde_json::to_value(MembersPage::Changed {
            group_revision: "f".repeat(64),
        })
        .unwrap();
        assert_eq!(changed["status"], "changed");
        assert!(changed["groupRevision"].is_string());
        let large: Vec<_> = (1..=135).map(|id| job(id, Some("123"), 1)).collect();
        let (_, single, _) = ready(page(large.iter(), "soop", None, usize::MAX).unwrap());
        assert_eq!(
            member_ready(members(large.iter(), "soop", key(&single[0]), None, usize::MAX).unwrap())
                .1
                .len(),
            100
        );
        assert_eq!(
            member_ready(members(large.iter(), "soop", key(&single[0]), None, 0).unwrap())
                .1
                .len(),
            1
        );
    }

    #[test]
    fn large_catalog_uses_complete_metadata_with_bounded_dto_output() {
        let records: Vec<_> = (1..=10_000)
            .map(|id| job(id, Some(&(1 + (id - 1) / 100).to_string()), id as u64))
            .collect();
        let started = std::time::Instant::now();
        let (revision, entries, _) = ready(page(records.iter(), "soop", None, 10).unwrap());
        assert_eq!(entries.len(), 10);
        for entry in &entries {
            assert_eq!(count(entry), 100);
        }
        let (reverse, _, _) = ready(page(records.iter().rev(), "soop", None, 10).unwrap());
        assert_eq!(revision, reverse);
        let json = serde_json::to_vec(&entries).unwrap();
        assert!(json.len() < 32 * 1024);
        eprintln!(
            "group catalog fixture: metadata=10000 groups=100 returned_dtos=10 elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }

    #[test]
    fn sidecar_warning_refresh_is_independent_of_metadata_revision_and_never_mutates_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        let mut record = job(1, Some("123"), 1);
        record.download_root = root.clone();
        let directory = super::super::storage::job_dir(&root, "soop", &record.job_id).unwrap();
        std::fs::create_dir_all(&directory).unwrap();
        let missing_tool = temp.path().join("must-not-run.exe");
        let service = PlatformMediaService::new(
            temp.path().join("data"),
            PlatformMediaTools {
                yt_dlp: missing_tool.clone(),
                deno: missing_tool.clone(),
                ffmpeg: missing_tool.clone(),
                ffprobe: missing_tool,
            },
            None,
        )
        .unwrap();
        service
            .inner
            .state
            .lock()
            .unwrap()
            .jobs
            .insert(record.job_id.clone(), record);
        let (catalog_revision, summary, _) =
            ready(service.library_groups("soop", None, 10).unwrap());
        let group_key = key(&summary[0]).to_owned();
        assert!(summary[0].item().chat_archive.is_none());
        let (before, items, _) = member_ready(
            service
                .library_group_members("soop", &group_key, None, 10)
                .unwrap(),
        );
        assert!(items[0].chat_archive.is_none());
        std::fs::write(
            directory.join("chat-status.json"),
            b"malformed synthetic status",
        )
        .unwrap();
        let (after, items, _) = member_ready(
            service
                .library_group_members("soop", &group_key, None, 10)
                .unwrap(),
        );
        assert_eq!(revision(&before), revision(&after));
        assert!(items[0].chat_archive.is_some());
        assert_eq!(
            ready(service.library_groups("soop", None, 10).unwrap()).0,
            catalog_revision
        );
        assert!(service
            .inner
            .state
            .lock()
            .unwrap()
            .jobs
            .values()
            .next()
            .unwrap()
            .library
            .as_ref()
            .unwrap()
            .chat_archive
            .is_none());
        assert!(service.active_ids().is_empty());
        service.shutdown();
    }
}
