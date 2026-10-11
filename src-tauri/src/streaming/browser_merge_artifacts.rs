//! Track native merge attempts before writing their outputs. Successful final
//! verification can then retire their exact files without scanning directories
//! or guessing ownership from a filename. Raw capture partials are never targets.
use super::*;

const MANIFEST: &str = "merge-attempts.json";
const LIMIT: u64 = 4 * 1024 * 1024;
const MAX_ATTEMPTS: usize = 8192;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Attempts {
    version: u32,
    recording_id: String,
    attempts: Vec<Attempt>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Attempt {
    token: String,
    first: u64,
    end: u64,
}

fn load(job: &BrowserMergeJob) -> Result<Attempts, StreamError> {
    let root = validate_merge_generation(job)?;
    let path = root.join(MANIFEST);
    if !path.try_exists().map_err(|_| storage())? {
        return Ok(Attempts {
            version: 1,
            recording_id: job.recording.id.clone(),
            attempts: vec![],
        });
    }
    let value: Attempts = serde_json::from_reader(BufReader::new(pinned_file(&path, LIMIT)?))
        .map_err(|_| invalid())?;
    let mut tokens = HashSet::new();
    if value.version != 1
        || value.recording_id != job.recording.id
        || value.attempts.len() > MAX_ATTEMPTS
        || value.attempts.iter().any(|a| {
            !valid_id(&a.token)
                || a.first >= a.end
                || a.end > job.recording.segment_count
                || !tokens.insert(&a.token)
        })
    {
        return Err(invalid());
    }
    Ok(value)
}

pub(crate) fn register(job: &BrowserMergeJob) -> Result<(), StreamError> {
    validate_id(&job.token)?;
    let root = validate_merge_generation(job)?;
    let mut value = load(job)?;
    let (first, end) = job
        .part
        .as_ref()
        .map_or((0, job.recording.segment_count), |p| (p.first, p.end));
    if first >= end || end > job.recording.segment_count {
        return Err(invalid());
    }
    if let Some(old) = value.attempts.iter().find(|a| a.token == job.token) {
        return if old.first == first && old.end == end {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    if value.attempts.len() >= MAX_ATTEMPTS {
        return Err(storage());
    }
    // A token collision must not claim a pre-existing file for later cleanup.
    for name in attempt_names(&job.token, extension(&job.recording.mime_type)) {
        match fs::symlink_metadata(root.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(invalid()),
        }
    }
    value.attempts.push(Attempt {
        token: job.token.clone(),
        first,
        end,
    });
    let bytes = serde_json::to_vec(&value).map_err(|_| storage())?;
    if bytes.len() as u64 > LIMIT {
        return Err(storage());
    }
    validate_merge_generation(job)?;
    atomic_write(&root, MANIFEST, &bytes)
}

fn attempt_names(token: &str, ext: &str) -> Vec<String> {
    let mut names = vec![format!("merge-{token}.ffconcat")];
    for suffix in [
        ext.to_owned(),
        format!("{ext}.partial"),
        "timeline.jsonl".into(),
        "timeline.jsonl.partial".into(),
        "cleanup.json".into(),
        "cleanup.json.partial".into(),
    ] {
        names.push(format!("merged-{token}.{suffix}"));
    }
    names
}

fn candidates(
    job: &BrowserMergeJob,
    derivatives: &[DerivativeProof],
) -> Result<Vec<String>, StreamError> {
    if job.part.is_some() {
        return Err(invalid());
    }
    validate_id(&job.token)?;
    let mut tokens: HashSet<String> = load(job)?.attempts.into_iter().map(|a| a.token).collect();
    // Older recordings still have durable part ownership, even without the new
    // attempt manifest. Unknown failed attempts remain untouched on upgrade.
    for part in parts::load(&job.recording)? {
        tokens.insert(part.file[7..39].to_owned());
    }
    tokens.insert(job.token.clone());
    let mut names = Vec::new();
    let ext = extension(&job.recording.mime_type);
    for token in tokens {
        if token == job.token {
            names.push(format!("merge-{token}.ffconcat"));
        } else {
            names.extend(attempt_names(&token, ext));
        }
    }
    let proven_ranges: HashSet<_> = derivatives.iter().map(|d| d.file.as_str()).collect();
    names.retain(|name| !proven_ranges.contains(name.as_str()));
    names.sort();
    Ok(names)
}

pub(super) fn snapshot(
    job: &BrowserMergeJob,
    derivatives: &[DerivativeProof],
    cancel: &AtomicBool,
) -> Result<(Vec<DerivativeProof>, Option<String>), StreamError> {
    if job.part.is_some() {
        return Ok((vec![], None));
    }
    let root = validate_merge_generation(job)?;
    let names = candidates(job, derivatives)?;
    let manifest = root.join(MANIFEST);
    let manifest_hash = if manifest.try_exists().map_err(|_| storage())? {
        Some(hash_file(&mut pinned_file(&manifest, LIMIT)?, cancel)?)
    } else {
        None
    };
    let mut artifacts = Vec::new();
    for name in names {
        if cancel.load(Ordering::Acquire) {
            return Err(inactive());
        }
        let path = root.join(&name);
        if !path.try_exists().map_err(|_| storage())? {
            continue;
        }
        let mut file = pinned_file(&path, u64::MAX)?;
        artifacts.push(DerivativeProof {
            file: name,
            bytes: file.metadata().map_err(|_| storage())?.len(),
            sha256: hash_file(&mut file, cancel)?,
        });
    }
    Ok((artifacts, manifest_hash))
}

/// The final proof authenticates sizes/content, and the attempt/part manifests
/// restrict names independently. Even a malformed proof cannot delete the final
/// file, chat, profiles, source partials, another recording or an arbitrary file.
pub(super) fn verify(
    job: &BrowserMergeJob,
    artifacts: &[DerivativeProof],
    manifest_hash: Option<&str>,
    derivatives: &[DerivativeProof],
    cancel: &AtomicBool,
) -> Result<Option<File>, StreamError> {
    if artifacts.is_empty() && manifest_hash.is_none() {
        return Ok(None);
    } // Legacy proof.
    let root = validate_merge_generation(job)?;
    let guard = if let Some(expected) = manifest_hash {
        if !valid_hash(expected) {
            return Err(invalid());
        }
        let mut file = pinned_file(&root.join(MANIFEST), LIMIT)?;
        if hash_file(&mut file, cancel)? != expected {
            return Err(invalid());
        }
        Some(file)
    } else {
        None
    };
    let allowed: HashSet<_> = candidates(job, derivatives)?.into_iter().collect();
    let mut seen = HashSet::new();
    if artifacts
        .iter()
        .any(|a| !allowed.contains(&a.file) || !valid_hash(&a.sha256) || !seen.insert(&a.file))
    {
        return Err(invalid());
    }
    Ok(guard)
}
