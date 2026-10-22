//! Bounded read models. Cursors name an ordering boundary, never a mutable offset.
use super::{LibraryItem, LibraryPage, MediaJob};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::{cmp::Reverse, collections::BTreeMap};

pub(super) const ACTIVE_JOB_LIMIT: usize = 128;
const JOB_SUMMARY_LIMIT: usize = 256;
const CURSOR_LIMIT: usize = 256;
const CURSOR_ERROR: &str = "목록 위치를 확인해 주세요.";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Boundary {
    version: u8,
    provider: String,
    completed_at: u64,
    library_id: String,
}
impl Boundary {
    fn decode(cursor: &str, provider: &str) -> Result<Self, String> {
        if cursor.is_empty() || cursor.len() > CURSOR_LIMIT {
            return Err(CURSOR_ERROR.into());
        }
        let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| CURSOR_ERROR)?;
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| CURSOR_ERROR)?;
        if value.version != 1
            || value.provider != provider
            || uuid::Uuid::parse_str(&value.library_id)
                .map_or(true, |id| id.to_string() != value.library_id)
        {
            return Err(CURSOR_ERROR.into());
        }
        Ok(value)
    }
    fn encode(item: &LibraryItem, provider: &str) -> Result<String, String> {
        let value = Self {
            version: 1,
            provider: provider.into(),
            completed_at: item.completed_at,
            library_id: item.library_id.clone(),
        };
        Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).map_err(|_| CURSOR_ERROR)?))
    }
}

pub(super) fn library<'a>(
    jobs: impl Iterator<Item = &'a MediaJob>,
    provider: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<LibraryPage, String> {
    if !matches!(provider, "youtube" | "soop") {
        return Err("서비스를 확인해 주세요.".into());
    }
    let boundary = cursor
        .map(|value| Boundary::decode(value, provider))
        .transpose()?;
    let limit = limit.clamp(1, 100);
    let mut selected = BTreeMap::new();
    for item in jobs
        .filter(|job| job.reference.provider == provider && job.state == "completed")
        .filter_map(|job| job.library.as_ref())
    {
        let key = (Reverse(item.completed_at), item.library_id.as_str());
        if boundary
            .as_ref()
            .is_some_and(|b| key <= (Reverse(b.completed_at), b.library_id.as_str()))
        {
            continue;
        }
        selected.insert(key, item);
        if selected.len() > limit + 1 {
            selected.pop_last();
        }
    }
    let has_more = selected.len() > limit;
    if has_more {
        selected.pop_last();
    }
    let next = if has_more {
        selected
            .last_key_value()
            .map(|(_, item)| Boundary::encode(item, provider))
            .transpose()?
    } else {
        None
    };
    Ok(LibraryPage {
        items: selected.into_values().cloned().collect(),
        cursor: next,
    })
}

pub(super) fn jobs<'a>(
    jobs: impl Iterator<Item = &'a MediaJob>,
    provider: Option<&str>,
) -> Vec<MediaJob> {
    let mut active = BTreeMap::new();
    let mut recent = BTreeMap::new();
    for job in jobs.filter(|job| provider.is_none_or(|p| p == job.reference.provider)) {
        let key = (Reverse(job.created_at), job.job_id.as_str());
        if job.active() {
            // All registration paths enforce ACTIVE_JOB_LIMIT; never hide ongoing work.
            active.insert(key, job);
        } else {
            recent.insert(key, job);
            if recent.len() > JOB_SUMMARY_LIMIT {
                recent.pop_last();
            }
        }
    }
    let remaining = JOB_SUMMARY_LIMIT.saturating_sub(active.len());
    active
        .into_values()
        .chain(recent.into_values().take(remaining))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_media::MediaRef;
    use std::path::PathBuf;

    fn job(id: u128, time: u64, state: &str, provider: &str) -> MediaJob {
        let reference = MediaRef {
            provider: provider.into(),
            kind: "video".into(),
            external_id: "jNQXAC9IVRw".into(),
        };
        let id = uuid::Uuid::from_u128(id).to_string();
        MediaJob {
            broadcast_id: None,
            job_id: id.clone(),
            request_id: id.clone(),
            reference: reference.clone(),
            revision: 1,
            state: state.into(),
            title: id.clone(),
            channel_name: "fixture".into(),
            created_at: time,
            received_bytes: 1,
            total_bytes: Some(1),
            error: None,
            download_root: PathBuf::new(),
            library: Some(LibraryItem {
                broadcast_id: None,
                chat_archive: None,
                capture_outcome: None,
                library_id: id,
                revision: 1,
                reference,
                title: "fixture".into(),
                channel_name: "fixture".into(),
                bytes: 1,
                duration: 1.0,
                sha256: "fixture".into(),
                completed_at: time,
                modified_millis: 0,
            }),
        }
    }
    fn ids(page: &LibraryPage) -> Vec<String> {
        page.items
            .iter()
            .map(|item| item.library_id.clone())
            .collect()
    }

    #[test]
    fn keyset_survives_new_completion_and_deleted_boundary() {
        let mut records: Vec<_> = (1..=6)
            .map(|id| job(id, 100 - id as u64, "completed", "youtube"))
            .collect();
        let first = library(records.iter(), "youtube", None, 2).unwrap();
        records.retain(|row| {
            row.job_id != first.items[0].library_id && row.job_id != first.items[1].library_id
        });
        records.push(job(9, 200, "completed", "youtube"));
        let second = library(records.iter(), "youtube", first.cursor.as_deref(), 2).unwrap();
        let third = library(records.iter(), "youtube", second.cursor.as_deref(), 2).unwrap();
        assert_eq!(
            ids(&second),
            vec![records[0].job_id.clone(), records[1].job_id.clone()]
        );
        assert_eq!(
            ids(&third),
            vec![records[2].job_id.clone(), records[3].job_id.clone()]
        );
        assert!(third.cursor.is_none());
        assert_eq!(
            library(records.iter(), "youtube", None, 1).unwrap().items[0].library_id,
            records[4].job_id
        );
    }

    #[test]
    fn equal_timestamps_and_provider_scope_are_stable() {
        let mut records: Vec<_> = (1..=8)
            .rev()
            .map(|id| job(id, 100, "completed", "youtube"))
            .collect();
        records.push(job(20, 999, "completed", "soop"));
        records.push(job(21, 999, "failed", "youtube"));
        let mut cursor = None;
        let mut found = Vec::new();
        loop {
            let page = library(records.iter(), "youtube", cursor.as_deref(), 3).unwrap();
            found.extend(ids(&page));
            cursor = page.cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            found,
            (1..=8)
                .map(|id| uuid::Uuid::from_u128(id).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn cursor_shape_and_page_size_are_bounded() {
        let records: Vec<_> = (1..=150)
            .map(|id| job(id, 100, "completed", "youtube"))
            .collect();
        let page = library(records.iter(), "youtube", None, usize::MAX).unwrap();
        assert_eq!(page.items.len(), 100);
        assert!(page.cursor.as_ref().unwrap().len() <= CURSOR_LIMIT);
        assert_eq!(
            library(records.iter(), "youtube", None, 0)
                .unwrap()
                .items
                .len(),
            1
        );
        for cursor in ["".to_string(), "0".into(), "!".into(), "a".repeat(257), URL_SAFE_NO_PAD.encode(b"[]"), URL_SAFE_NO_PAD.encode(br#"{"version":2,"provider":"youtube","completed_at":1,"library_id":"00000000-0000-0000-0000-000000000001"}"#), URL_SAFE_NO_PAD.encode(br#"{"version":1,"provider":"youtube","completed_at":1,"library_id":"../../secret"}"#)] {
            assert!(library(records.iter(), "youtube", Some(&cursor), 10).is_err());
        }
        assert!(library(records.iter(), "soop", page.cursor.as_deref(), 10).is_err());
        assert!(library(records.iter(), "invalid", None, 10).is_err());
    }

    #[test]
    fn all_active_jobs_survive_a_large_settled_history() {
        let mut records: Vec<_> = (1..=1000)
            .map(|id| job(id, id as u64 + 100, "completed", "youtube"))
            .collect();
        let states = [
            "queued",
            "resolving",
            "downloading",
            "recording",
            "finalizing",
            "verifying",
        ];
        records.extend((0..ACTIVE_JOB_LIMIT).map(|index| {
            job(
                2000 + index as u128,
                1,
                states[index % states.len()],
                "youtube",
            )
        }));
        let output = jobs(records.iter(), Some("youtube"));
        assert_eq!(output.len(), JOB_SUMMARY_LIMIT);
        assert_eq!(
            output.iter().filter(|job| job.active()).count(),
            ACTIVE_JOB_LIMIT
        );
        assert_eq!(output[ACTIVE_JOB_LIMIT].created_at, 1100);
        assert_eq!(output.last().unwrap().created_at, 973);
        assert!(output[..ACTIVE_JOB_LIMIT]
            .windows(2)
            .all(|pair| pair[0].job_id < pair[1].job_id));
        assert!(jobs(records.iter(), Some("soop")).is_empty());
    }
}
