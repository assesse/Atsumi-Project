//! Test-only gold-corpus replay and evaluation.
//!
//! Cache-only replay deliberately bypasses `SqliteRepository::open`: a live user database is opened
//! with SQLite read-only/query-only flags, and missing HashProfile rows remain explicitly unscored.
//! An opt-in preparation test first online-backs up that database into task-owned `.runtime`, then
//! uses production verification/hash routines to fill only the shadow cache. The evaluator closes
//! that writer and reopens either input read-only; the live database is never opened for writing.

#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use rusqlite::{backup::Backup, params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{
    ArtifactSha256, DuplicateGalleryRef, DuplicatePageHash, GalleryId, HashProfile,
    InternalDuplicateGroup, InternalMatchKind, InternalPageEvidence, SourcePageNumber,
    INTERNAL_DUPLICATE_ALGORITHM_VERSION,
};
use crate::infrastructure::{FilesystemArtifactStore, SqliteRepository};

use super::{
    duplicate_analyzer::{compute_page_hash, verified_scan_pages, HashedArtifact},
    internal_duplicate_analyzer::detect_internal_groups,
    ArtifactStore, DownloadPipelineRepository, DuplicateRepository,
};

const SHORT_TRACK_DIAGNOSTIC_MAX_PAGES: usize = 3;

type DiagnosticResult<T> = Result<T, String>;
type PagePair = (u32, u32);
type PagePairSet = BTreeSet<PagePair>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    corpus_id: String,
    usage: String,
    visibility: String,
    scoring_policy: serde_json::Value,
    albums: Vec<GoldAlbum>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GoldAlbum {
    gallery_id: i64,
    family_id: String,
    role: String,
    page_count: u32,
    expected_edition_structure: String,
    evaluation_scopes: Vec<String>,
    #[serde(default)]
    album_tracks: Vec<GoldTrack>,
    #[serde(default)]
    scene_blocks: Vec<GoldBlock>,
    #[serde(default)]
    global_track_continuity_across_blocks: bool,
    preserve_page_ranges: Vec<String>,
    diagnostic_only_page_ranges: Vec<String>,
    hard_negative_pairs: Vec<HardNegativePair>,
    near_duplicate_distinct_scene_ranges: Vec<NearDuplicateRange>,
    non_bridge_page_ranges: Vec<String>,
    notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GoldTrack {
    id: String,
    human_label: String,
    page_ranges: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GoldBlock {
    id: String,
    track_slices: BTreeMap<String, Vec<String>>,
    alignment: GoldAlignment,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GoldAlignment {
    mode: String,
    #[serde(default)]
    rows: Vec<GoldRow>,
    #[serde(default)]
    known_same_scene_groups: Vec<KnownSceneGroup>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GoldRow {
    id: String,
    members: BTreeMap<String, u32>,
    #[serde(default)]
    missing_track_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KnownSceneGroup {
    id: String,
    pages: Vec<u32>,
    #[serde(default)]
    missing_track_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HardNegativePair {
    pages: [u32; 2],
    relation: String,
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NearDuplicateRange {
    #[serde(default)]
    track_id: Option<String>,
    page_range: String,
    note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GalleryPrediction {
    gallery_id: i64,
    expected_pages: u32,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry_id: Option<String>,
    input_fingerprint: String,
    input_source: String,
    available_hashes: usize,
    cached_hashes: usize,
    prepared_hashes: usize,
    missing_hash_pages: Vec<u32>,
    compared_pairs: u64,
    replay_runtime_micros: u128,
    detection_runtime_micros: u128,
    groups: Vec<InternalDuplicateGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metric {
    true_positive: usize,
    false_positive: usize,
    false_negative: usize,
    precision: f64,
    recall: f64,
    f1: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MatchRecord {
    gold_id: String,
    prediction_id: String,
    overlap_pages: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumEvaluation {
    gallery_id: i64,
    family_id: String,
    role: String,
    scored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    unscored_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    track_partition: Option<Metric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    track_page_pairs: Option<Metric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_partition: Option<Metric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scene_pairs: Option<Metric>,
    track_matches: Vec<MatchRecord>,
    block_matches: Vec<MatchRecord>,
    track_fragmentation: usize,
    distinct_track_merges: usize,
    spurious_short_tracks: usize,
    block_splits: usize,
    block_merges: usize,
    preserve_page_removal: usize,
    hard_negative_scene_merges: usize,
    near_duplicate_distinct_scene_merges: usize,
    non_bridge_page_uses: usize,
    replay_runtime_micros: u128,
    detection_runtime_micros: u128,
    compared_pairs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FamilyScore {
    family_id: String,
    albums_total: usize,
    scored_albums: usize,
    track_f1: Option<f64>,
    block_f1: Option<f64>,
    scene_f1: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    albums_total: usize,
    albums_scored: usize,
    albums_missing_hashes: usize,
    all_albums_scored: bool,
    adoption_valid: bool,
    album_macro_track_f1: Option<f64>,
    family_macro_track_f1: Option<f64>,
    album_macro_block_f1: Option<f64>,
    family_macro_block_f1: Option<f64>,
    album_macro_scene_f1: Option<f64>,
    family_macro_scene_f1: Option<f64>,
    family_scores: Vec<FamilyScore>,
    page_pair_micro: Metric,
    track_fragmentation: usize,
    distinct_track_merges: usize,
    spurious_short_tracks: usize,
    block_splits: usize,
    block_merges: usize,
    preserve_page_removal: usize,
    hard_negative_scene_merges: usize,
    near_duplicate_distinct_scene_merges: usize,
    #[serde(default)]
    non_bridge_page_uses: usize,
    visible_regression_albums_total: usize,
    visible_regression_albums_scored: usize,
    visible_regression_track_f1: Option<f64>,
    visible_regression_safety_pass: bool,
    compared_pairs: u64,
    replay_runtime_micros: u128,
    detection_runtime_micros: u128,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BaselineReport {
    report_version: u32,
    corpus_schema_version: u32,
    corpus_id: String,
    corpus_usage: String,
    corpus_visibility: String,
    scoring_policy: serde_json::Value,
    sqlite_schema_version: i64,
    hash_profile_version: u32,
    hash_profile: HashProfileFingerprint,
    input_fingerprint: String,
    internal_algorithm_version: u32,
    database_label: String,
    validator_output: String,
    predictions: Vec<GalleryPrediction>,
    evaluations: Vec<AlbumEvaluation>,
    summary: Summary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HashProfileFingerprint {
    profile_version: u32,
    algorithm_version: u32,
    d_hash_bits: u32,
    p_hash_bits: u32,
    visual_match_threshold: f64,
    low_information_std_dev_threshold: f64,
}

#[derive(Debug)]
struct LoadedHashes {
    entry_id: Option<String>,
    hashes: Vec<DuplicatePageHash>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShadowGalleryPreparation {
    gallery_id: i64,
    entry_id: String,
    expected_pages: u32,
    cached_hashes: usize,
    prepared_hashes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShadowPreparationReport {
    report_version: u32,
    source_database: String,
    shadow_database: String,
    hash_profile_version: u32,
    galleries: Vec<ShadowGalleryPreparation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComparisonEnvelope {
    corpus_schema_version: u32,
    corpus_id: String,
    hash_profile: HashProfileFingerprint,
    input_fingerprint: String,
    evaluations: Vec<ComparisonAlbum>,
    summary: Summary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComparisonAlbum {
    gallery_id: i64,
    family_id: String,
    role: String,
    scored: bool,
    track_partition: Option<Metric>,
    block_partition: Option<Metric>,
    scene_pairs: Option<Metric>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdoptionComparison {
    adopt: bool,
    reasons: Vec<String>,
    family_macro_track_delta: Option<f64>,
    album_macro_track_delta: Option<f64>,
    family_macro_block_delta: Option<f64>,
    family_macro_scene_delta: Option<f64>,
    visible_regression_track_delta: Option<f64>,
    compared_pair_delta: i128,
}

#[test]
#[ignore = "requires ATSUMI_INTERNAL_CORPUS_* paths and an existing read-only user database"]
fn validate_internal_duplicate_corpus_with_provided_validator() {
    let corpus_path = required_path("ATSUMI_INTERNAL_CORPUS_PATH").unwrap();
    let validator_path = required_path("ATSUMI_INTERNAL_CORPUS_VALIDATOR").unwrap();
    let python_path = required_path("ATSUMI_INTERNAL_CORPUS_PYTHON").unwrap();
    let output = run_validator(&python_path, &validator_path, &corpus_path).unwrap();
    assert!(output.contains("CORPUS VALID"), "{output}");
}

#[test]
#[ignore = "creates a task-owned .runtime shadow database and hashes only its missing dev pages"]
fn prepare_internal_duplicate_corpus_shadow() {
    run_shadow_preparation_from_environment().unwrap();
}

#[test]
#[ignore = "requires ATSUMI_INTERNAL_CORPUS_* paths and an existing read-only user database"]
fn export_and_evaluate_internal_duplicate_corpus() {
    run_export_from_environment().unwrap();
}

#[test]
#[ignore = "compares complete baseline/candidate reports below task-owned .runtime"]
fn compare_internal_duplicate_corpus_reports() {
    let baseline_path =
        required_runtime_input_path("ATSUMI_INTERNAL_CORPUS_BASELINE_REPORT").unwrap();
    let candidate_path =
        required_runtime_input_path("ATSUMI_INTERNAL_CORPUS_CANDIDATE_REPORT").unwrap();
    let output_path =
        required_runtime_output_path("ATSUMI_INTERNAL_CORPUS_COMPARISON_REPORT").unwrap();
    let baseline: ComparisonEnvelope = read_json(&baseline_path).unwrap();
    let candidate: ComparisonEnvelope = read_json(&candidate_path).unwrap();
    write_json(&output_path, &compare_reports(&baseline, &candidate)).unwrap();
}

#[test]
#[ignore = "removes only the explicitly resolved task-owned shadow and its sidecars"]
fn cleanup_internal_duplicate_corpus_shadow() {
    let shadow = required_runtime_input_path("ATSUMI_INTERNAL_CORPUS_SHADOW_DB").unwrap();
    let manifest = required_runtime_input_path("ATSUMI_INTERNAL_CORPUS_SHADOW_MANIFEST").unwrap();
    for generated in [
        shadow.clone(),
        PathBuf::from(format!("{}-wal", shadow.display())),
        PathBuf::from(format!("{}-shm", shadow.display())),
        manifest,
    ] {
        let generated =
            resolve_runtime_descendant_output_path(&generated, "task-owned shadow cleanup target")
                .unwrap();
        if generated.is_file() {
            std::fs::remove_file(&generated).unwrap_or_else(|error| {
                panic!("remove task-owned {}: {error}", generated.display())
            });
        }
    }
}

fn run_export_from_environment() -> DiagnosticResult<()> {
    let corpus_path = required_path("ATSUMI_INTERNAL_CORPUS_PATH")?;
    let schema_path = required_path("ATSUMI_INTERNAL_CORPUS_SCHEMA")?;
    let validator_path = required_path("ATSUMI_INTERNAL_CORPUS_VALIDATOR")?;
    let python_path = required_path("ATSUMI_INTERNAL_CORPUS_PYTHON")?;
    let database_path = required_path("ATSUMI_INTERNAL_DUPLICATE_DB")?;
    let database_label = env::var("ATSUMI_INTERNAL_CORPUS_INPUT_SOURCE")
        .unwrap_or_else(|_| "live_cache_read_only".into());
    let preparation = env::var_os("ATSUMI_INTERNAL_CORPUS_SHADOW_MANIFEST")
        .map(PathBuf::from)
        .map(|path| read_json::<ShadowPreparationReport>(&path))
        .transpose()?;
    let preparation_by_gallery = preparation
        .as_ref()
        .map(|report| {
            report
                .galleries
                .iter()
                .map(|gallery| (gallery.gallery_id, gallery))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let report_path = required_runtime_output_path("ATSUMI_INTERNAL_CORPUS_REPORT")?;

    let validator_output = run_validator(&python_path, &validator_path, &corpus_path)?;
    let schema: serde_json::Value = read_json(&schema_path)?;
    if schema.pointer("/properties/schemaVersion/const") != Some(&serde_json::json!(1)) {
        return Err("corpus schema does not declare schemaVersion const 1".into());
    }
    let corpus: Corpus = read_json(&corpus_path)?;
    validate_dev_corpus_contract(&corpus)?;
    let profile = HashProfile::current();
    if profile.profile_version == 0 {
        return Err("current HashProfile version must be positive".into());
    }
    if let Some(preparation) = &preparation {
        if preparation.hash_profile_version != profile.profile_version
            || preparation.galleries.len() != corpus.albums.len()
        {
            return Err("shadow preparation manifest is incomplete or uses another profile".into());
        }
    }

    let connection = open_read_only(&database_path)?;
    let sqlite_schema_version = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("read SQLite schema version: {error}"))?;
    let mut predictions = Vec::with_capacity(corpus.albums.len());
    for album in &corpus.albums {
        predictions.push(replay_gallery(
            &connection,
            album,
            &profile,
            &database_label,
            preparation_by_gallery.get(&album.gallery_id).copied(),
        )?);
    }
    let evaluations = corpus
        .albums
        .iter()
        .zip(&predictions)
        .map(|(album, prediction)| evaluate_album(album, prediction))
        .collect::<DiagnosticResult<Vec<_>>>()?;
    let summary = summarize(&evaluations);
    let input_fingerprint = report_input_fingerprint(&predictions);
    let report = BaselineReport {
        report_version: 1,
        corpus_schema_version: corpus.schema_version,
        corpus_id: corpus.corpus_id,
        corpus_usage: corpus.usage,
        corpus_visibility: corpus.visibility,
        scoring_policy: corpus.scoring_policy,
        sqlite_schema_version,
        hash_profile_version: profile.profile_version,
        hash_profile: HashProfileFingerprint::from(&profile),
        input_fingerprint,
        internal_algorithm_version: INTERNAL_DUPLICATE_ALGORITHM_VERSION,
        database_label,
        validator_output,
        predictions: predictions.clone(),
        evaluations,
        summary,
    };
    write_json(&report_path, &report)?;
    let prediction_directory = report_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("internal-duplicate-predictions");
    let prediction_directory = resolve_runtime_child_directory(
        &prediction_directory,
        "internal duplicate prediction directory",
    )?;
    for prediction in &predictions {
        write_json(
            &prediction_directory
                .join(format!("gallery-{}-prediction.json", prediction.gallery_id)),
            prediction,
        )?;
    }
    eprintln!(
        "internal duplicate corpus report: {}",
        report_path.display()
    );
    Ok(())
}

impl From<&HashProfile> for HashProfileFingerprint {
    fn from(profile: &HashProfile) -> Self {
        Self {
            profile_version: profile.profile_version,
            algorithm_version: profile.algorithm_version,
            d_hash_bits: profile.d_hash_bits,
            p_hash_bits: profile.p_hash_bits,
            visual_match_threshold: profile.visual_match_threshold,
            low_information_std_dev_threshold: profile.low_information_std_dev_threshold,
        }
    }
}

fn run_shadow_preparation_from_environment() -> DiagnosticResult<()> {
    let corpus_path = required_path("ATSUMI_INTERNAL_CORPUS_PATH")?;
    let validator_path = required_path("ATSUMI_INTERNAL_CORPUS_VALIDATOR")?;
    let python_path = required_path("ATSUMI_INTERNAL_CORPUS_PYTHON")?;
    let source_database = required_path("ATSUMI_INTERNAL_DUPLICATE_DB")?;
    let shadow_database = required_runtime_output_path("ATSUMI_INTERNAL_CORPUS_SHADOW_DB")?;
    let manifest_path = required_runtime_output_path("ATSUMI_INTERNAL_CORPUS_SHADOW_MANIFEST")?;
    let resume_shadow = env::var("ATSUMI_INTERNAL_CORPUS_SHADOW_RESUME")
        .ok()
        .as_deref()
        == Some("1");
    let validator_output = run_validator(&python_path, &validator_path, &corpus_path)?;
    if !validator_output.contains("CORPUS VALID") {
        return Err(format!("unexpected validator output: {validator_output}"));
    }
    let corpus: Corpus = read_json(&corpus_path)?;
    validate_dev_corpus_contract(&corpus)?;
    if resume_shadow {
        if !shadow_database.is_file() {
            return Err("shadow resume requested but the task-owned shadow does not exist".into());
        }
    } else {
        create_shadow_database(&source_database, &shadow_database)?;
    }
    let source_connection = open_read_only(&source_database)?;
    let repository = SqliteRepository::open(&shadow_database)
        .map_err(|error| format!("open task-owned shadow repository: {error}"))?;
    let store = FilesystemArtifactStore::new();
    let profile = HashProfile::current();
    let wanted = corpus
        .albums
        .iter()
        .map(|album| (album.gallery_id, album.page_count))
        .collect::<BTreeMap<_, _>>();
    let mut bundles = DuplicateRepository::duplicate_artifact_bundles(&repository)
        .map_err(|error| format!("read verified shadow artifacts: {error}"))?
        .into_iter()
        .filter(|bundle| wanted.contains_key(&bundle.gallery.id.get()))
        .collect::<Vec<_>>();
    bundles.sort_by(|left, right| {
        left.gallery.id.cmp(&right.gallery.id).then_with(|| {
            right
                .artifact
                .completed_at
                .cmp(&left.artifact.completed_at)
                .then_with(|| right.artifact.revision.cmp(&left.artifact.revision))
                .then_with(|| left.artifact.entry_id.cmp(&right.artifact.entry_id))
        })
    });
    bundles.dedup_by(|left, right| left.gallery.id == right.gallery.id);
    let mut preparations = Vec::with_capacity(corpus.albums.len());
    for album in &corpus.albums {
        let bundle = bundles
            .iter()
            .find(|bundle| bundle.gallery.id.get() == album.gallery_id)
            .ok_or_else(|| format!("#{} has no verified artifact", album.gallery_id))?;
        let pages = verified_scan_pages(bundle)
            .ok_or_else(|| format!("#{} artifact is not scan eligible", album.gallery_id))?;
        if pages.len() != album.page_count as usize {
            return Err(format!(
                "#{} verified page count differs from corpus: {}/{}",
                album.gallery_id,
                pages.len(),
                album.page_count
            ));
        }
        let root_snapshot = DownloadPipelineRepository::pipeline_artifact_root(
            &repository,
            &bundle.artifact.entry_id,
        )
        .map_err(|error| format!("read immutable artifact root snapshot: {error}"))?;
        if !root_snapshot.is_absolute() {
            return Err(format!(
                "#{} artifact root snapshot is not absolute",
                album.gallery_id
            ));
        }
        let cached_hashes = count_valid_hashes_for_entry(
            &source_connection,
            album.gallery_id,
            bundle.artifact.entry_id.as_str(),
            profile.profile_version,
        )?;
        for page in pages {
            let sha = page
                .sha256
                .as_ref()
                .ok_or_else(|| format!("#{} verified page lost SHA-256", album.gallery_id))?;
            if DuplicateRepository::duplicate_page_hash_get(
                &repository,
                bundle.artifact.entry_id.as_str(),
                page.page_id.source_page_number,
                profile.profile_version,
                sha.as_str(),
            )
            .map_err(|error| format!("read shadow hash cache: {error}"))?
            .is_some()
            {
                continue;
            }
            let bytes = store
                .read_verified_page_bytes(&root_snapshot, page)
                .map_err(|error| format!("read verified source page: {error}"))?;
            let hash = compute_page_hash(
                bundle.artifact.entry_id.as_str(),
                bundle.gallery.id,
                page.page_id.source_page_number,
                sha.clone(),
                &bytes,
                &profile,
            )
            .map_err(|error| format!("compute HashProfile 1 in shadow: {error}"))?;
            DuplicateRepository::duplicate_page_hash_upsert(&repository, &hash)
                .map_err(|error| format!("cache HashProfile 1 in shadow: {error}"))?;
        }
        preparations.push(ShadowGalleryPreparation {
            gallery_id: album.gallery_id,
            entry_id: bundle.artifact.entry_id.to_string(),
            expected_pages: album.page_count,
            cached_hashes,
            prepared_hashes: album.page_count as usize - cached_hashes,
        });
        write_json(
            &manifest_path,
            &ShadowPreparationReport {
                report_version: 1,
                source_database: "live_read_only_online_backup".into(),
                shadow_database: "task_owned_runtime_shadow".into(),
                hash_profile_version: profile.profile_version,
                galleries: preparations.clone(),
            },
        )?;
    }
    {
        let connection = repository
            .connection()
            .map_err(|error| format!("lock shadow for WAL checkpoint: {error}"))?;
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|error| format!("checkpoint task-owned shadow WAL: {error}"))?;
    }
    drop(repository);
    let report = ShadowPreparationReport {
        report_version: 1,
        source_database: "live_read_only_online_backup".into(),
        shadow_database: "task_owned_runtime_shadow".into(),
        hash_profile_version: profile.profile_version,
        galleries: preparations,
    };
    write_json(&manifest_path, &report)?;
    Ok(())
}

fn required_runtime_output_path(name: &str) -> DiagnosticResult<PathBuf> {
    let requested = env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required"))?;
    resolve_runtime_output_path(&requested, name)
}

fn required_runtime_input_path(name: &str) -> DiagnosticResult<PathBuf> {
    let path = required_runtime_output_path(name)?;
    if !path.is_file() {
        return Err(format!("{name} does not name an existing task-owned file"));
    }
    Ok(path)
}

fn resolve_runtime_output_path(requested: &Path, label: &str) -> DiagnosticResult<PathBuf> {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "Cargo manifest directory has no repository parent".to_owned())?;
    let declared_directory = repository_root.join(".runtime").join("diagnostics");
    if !requested.is_absolute() || requested.parent() != Some(declared_directory.as_path()) {
        return Err(format!(
            "{label} must name a direct child of the repository .runtime/diagnostics directory"
        ));
    }
    let file_name = requested
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("{label} has no file name"))?;
    let resolved_directory = resolve_runtime_diagnostics_directory()?;
    resolve_plain_output_leaf(&resolved_directory, file_name, label)
}

fn resolve_runtime_diagnostics_directory() -> DiagnosticResult<PathBuf> {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "Cargo manifest directory has no repository parent".to_owned())?;
    let resolved_repository = std::fs::canonicalize(repository_root)
        .map_err(|error| format!("resolve repository root: {error}"))?;
    let runtime_directory = ensure_plain_child_directory(
        &resolved_repository,
        &repository_root.join(".runtime"),
        "repository .runtime directory",
    )?;
    ensure_plain_child_directory(
        &runtime_directory,
        &runtime_directory.join("diagnostics"),
        "task-owned diagnostics directory",
    )
}

fn resolve_runtime_child_directory(requested: &Path, label: &str) -> DiagnosticResult<PathBuf> {
    let resolved_diagnostics = resolve_runtime_diagnostics_directory()?;
    let parent = requested
        .parent()
        .ok_or_else(|| format!("{label} has no parent directory"))?;
    let resolved_parent = std::fs::canonicalize(parent)
        .map_err(|error| format!("resolve {label} parent: {error}"))?;
    if resolved_parent != resolved_diagnostics {
        return Err(format!(
            "{label} must be a direct child of the task-owned diagnostics directory"
        ));
    }
    ensure_plain_child_directory(&resolved_diagnostics, requested, label)
}

fn ensure_plain_child_directory(
    resolved_parent: &Path,
    child: &Path,
    label: &str,
) -> DiagnosticResult<PathBuf> {
    match std::fs::symlink_metadata(child) {
        Ok(metadata) => {
            reject_link_or_reparse(child, &metadata, label)?;
            if !metadata.is_dir() {
                return Err(format!("{label} is not a directory"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(child).map_err(|error| format!("create {label}: {error}"))?;
        }
        Err(error) => return Err(format!("inspect {label}: {error}")),
    }
    let resolved =
        std::fs::canonicalize(child).map_err(|error| format!("resolve {label}: {error}"))?;
    if resolved.parent() != Some(resolved_parent) {
        return Err(format!("resolved {label} escapes its task-owned parent"));
    }
    Ok(resolved)
}

fn resolve_plain_output_leaf(
    resolved_parent: &Path,
    file_name: &std::ffi::OsStr,
    label: &str,
) -> DiagnosticResult<PathBuf> {
    let candidate = resolved_parent.join(file_name);
    match std::fs::symlink_metadata(&candidate) {
        Ok(metadata) => {
            reject_link_or_reparse(&candidate, &metadata, label)?;
            if !metadata.is_file() {
                return Err(format!("{label} is not a regular file"));
            }
            let resolved = std::fs::canonicalize(&candidate)
                .map_err(|error| format!("resolve existing {label}: {error}"))?;
            if resolved.parent() != Some(resolved_parent) {
                return Err(format!("resolved {label} escapes its task-owned parent"));
            }
            Ok(resolved)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(candidate),
        Err(error) => Err(format!("inspect {label}: {error}")),
    }
}

fn resolve_runtime_descendant_output_path(
    requested: &Path,
    label: &str,
) -> DiagnosticResult<PathBuf> {
    if !requested.is_absolute() {
        return Err(format!("{label} must be absolute"));
    }
    let resolved_diagnostics = resolve_runtime_diagnostics_directory()?;
    let parent = requested
        .parent()
        .ok_or_else(|| format!("{label} has no parent directory"))?;
    let relative_parent = parent
        .strip_prefix(&resolved_diagnostics)
        .map_err(|_| format!("{label} must remain below the task-owned diagnostics directory"))?;
    let mut resolved_parent = resolved_diagnostics;
    for component in relative_parent.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(format!("{label} contains a non-normal path component"));
        };
        let child = resolved_parent.join(name);
        resolved_parent = ensure_plain_child_directory(&resolved_parent, &child, label)?;
    }
    let file_name = requested
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("{label} has no file name"))?;
    resolve_plain_output_leaf(&resolved_parent, file_name, label)
}

fn reject_link_or_reparse(
    path: &Path,
    metadata: &std::fs::Metadata,
    label: &str,
) -> DiagnosticResult<()> {
    if metadata.file_type().is_symlink() || metadata_is_windows_reparse_point(metadata) {
        return Err(format!(
            "{label} must not be a symbolic link, junction, or reparse point: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_windows_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    has_windows_reparse_attribute(metadata.file_attributes())
}

#[cfg(not(windows))]
fn metadata_is_windows_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn has_windows_reparse_attribute(file_attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn create_shadow_database(source_path: &Path, shadow_path: &Path) -> DiagnosticResult<()> {
    let source = std::fs::canonicalize(source_path)
        .map_err(|error| format!("resolve source database: {error}"))?;
    let parent = shadow_path
        .parent()
        .ok_or_else(|| "shadow database has no parent directory".to_owned())?;
    let resolved_parent =
        std::fs::canonicalize(parent).map_err(|error| format!("resolve shadow parent: {error}"))?;
    let resolved_shadow = resolved_parent.join(
        shadow_path
            .file_name()
            .ok_or_else(|| "shadow database has no file name".to_owned())?,
    );
    if resolved_shadow == source {
        return Err("shadow database must differ from the live database".into());
    }
    for generated in [
        resolved_shadow.clone(),
        PathBuf::from(format!("{}-wal", resolved_shadow.display())),
        PathBuf::from(format!("{}-shm", resolved_shadow.display())),
    ] {
        let generated = resolve_runtime_descendant_output_path(
            &generated,
            "task-owned shadow database output",
        )?;
        if generated.is_file() {
            std::fs::remove_file(&generated)
                .map_err(|error| format!("replace task-owned {}: {error}", generated.display()))?;
        }
    }
    let source_connection = open_read_only(&source)?;
    let mut shadow_connection = Connection::open(&resolved_shadow)
        .map_err(|error| format!("create task-owned shadow database: {error}"))?;
    let backup = Backup::new(&source_connection, &mut shadow_connection)
        .map_err(|error| format!("start SQLite online backup: {error}"))?;
    backup
        .run_to_completion(128, Duration::from_millis(5), None)
        .map_err(|error| format!("complete SQLite online backup: {error}"))?;
    Ok(())
}

fn required_path(name: &str) -> DiagnosticResult<PathBuf> {
    let value = env::var_os(name).ok_or_else(|| format!("{name} is required"))?;
    let path = PathBuf::from(value);
    if !path.is_file() {
        return Err(format!(
            "{name} does not name an existing file: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn validate_dev_corpus_contract(corpus: &Corpus) -> DiagnosticResult<()> {
    if corpus.schema_version != 1
        || corpus.corpus_id != "atsumi-internal-duplicate-dev-v1"
        || corpus.usage != "development"
        || corpus.visibility != "share_with_implementation_agent"
    {
        return Err("only the explicitly shared development corpus v1 is accepted".into());
    }
    Ok(())
}

fn count_valid_hashes_for_entry(
    connection: &Connection,
    gallery_id: i64,
    entry_id: &str,
    profile_version: u32,
) -> DiagnosticResult<usize> {
    let count = connection
        .query_row(
            r#"
                SELECT COUNT(*)
                FROM duplicate_page_hashes h
                JOIN download_pages p
                  ON p.entry_id = h.entry_id
                 AND p.gallery_id = h.gallery_id
                 AND p.source_page_number = h.source_page_number
                 AND p.sha256 = h.artifact_sha256
                WHERE h.gallery_id = ?1
                  AND h.entry_id = ?2
                  AND h.profile_version = ?3
                  AND p.state = 'present'
                  AND p.excluded = 0
            "#,
            params![gallery_id, entry_id, i64::from(profile_version)],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("count source hash cache: {error}"))?;
    usize::try_from(count).map_err(|_| format!("invalid source hash count: {count}"))
}

fn run_validator(python: &Path, validator: &Path, corpus: &Path) -> DiagnosticResult<String> {
    let output = Command::new(python)
        .arg(validator)
        .arg(corpus)
        .output()
        .map_err(|error| format!("run corpus validator: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !output.status.success() {
        return Err(format!(
            "corpus validator failed with {}: stdout={stdout:?}, stderr={stderr:?}",
            output.status
        ));
    }
    Ok(if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n{stderr}")
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> DiagnosticResult<T> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> DiagnosticResult<()> {
    let path = resolve_runtime_descendant_output_path(path, "diagnostic JSON output")?;
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    bytes.push(b'\n');
    std::fs::write(&path, bytes).map_err(|error| format!("write {}: {error}", path.display()))
}

fn open_read_only(path: &Path) -> DiagnosticResult<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(path, flags)
        .map_err(|error| format!("open SQLite read-only {}: {error}", path.display()))?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|error| format!("enable SQLite query_only: {error}"))?;
    Ok(connection)
}

fn replay_gallery(
    connection: &Connection,
    album: &GoldAlbum,
    profile: &HashProfile,
    input_source: &str,
    preparation: Option<&ShadowGalleryPreparation>,
) -> DiagnosticResult<GalleryPrediction> {
    let replay_started = Instant::now();
    let loaded = load_hashes(
        connection,
        album.gallery_id,
        profile.profile_version,
        preparation.map(|value| value.entry_id.as_str()),
    )?;
    let input_fingerprint =
        prediction_input_fingerprint(loaded.entry_id.as_deref(), &loaded.hashes);
    let present = loaded
        .hashes
        .iter()
        .map(|hash| hash.source_page_number.get())
        .collect::<BTreeSet<_>>();
    let missing_hash_pages = (1..=album.page_count)
        .filter(|page| !present.contains(page))
        .collect::<Vec<_>>();
    if let Some(preparation) = preparation {
        if loaded.entry_id.as_deref() != Some(preparation.entry_id.as_str()) {
            return Err(format!(
                "#{} shadow preparation entry differs from replay entry",
                album.gallery_id
            ));
        }
    }
    let (cached_hashes, prepared_hashes) = preparation
        .map(|value| (value.cached_hashes, value.prepared_hashes))
        .unwrap_or((loaded.hashes.len(), 0));
    if preparation.is_some() && cached_hashes + prepared_hashes != loaded.hashes.len() {
        return Err(format!(
            "#{} shadow preparation counts differ from replay input",
            album.gallery_id
        ));
    }
    if loaded.hashes.len() != album.page_count as usize || !missing_hash_pages.is_empty() {
        return Ok(GalleryPrediction {
            gallery_id: album.gallery_id,
            expected_pages: album.page_count,
            status: "missing_hashes",
            entry_id: loaded.entry_id,
            input_fingerprint,
            input_source: input_source.to_owned(),
            available_hashes: loaded.hashes.len(),
            cached_hashes,
            prepared_hashes,
            missing_hash_pages,
            compared_pairs: 0,
            replay_runtime_micros: replay_started.elapsed().as_micros(),
            detection_runtime_micros: 0,
            groups: Vec::new(),
        });
    }
    let entry_id = loaded
        .entry_id
        .clone()
        .ok_or_else(|| format!("#{} hashes have no entry ID", album.gallery_id))?;
    let title = connection
        .query_row(
            "SELECT title FROM galleries WHERE gallery_id = ?1",
            [album.gallery_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("read #{} title: {error}", album.gallery_id))?;
    let gallery_id = GalleryId::new(album.gallery_id).map_err(|error| error.to_string())?;
    let artifact = HashedArtifact {
        gallery: DuplicateGalleryRef {
            gallery_id,
            entry_id,
            title,
            artist: None,
            group: None,
            page_count: album.page_count,
        },
        pages: loaded.hashes,
    };
    let detection_started = Instant::now();
    let detection = detect_internal_groups("corpus-replay", &artifact, profile);
    let detection_runtime_micros = detection_started.elapsed().as_micros();
    Ok(GalleryPrediction {
        gallery_id: album.gallery_id,
        expected_pages: album.page_count,
        status: "predicted",
        entry_id: loaded.entry_id,
        input_fingerprint,
        input_source: input_source.to_owned(),
        available_hashes: album.page_count as usize,
        cached_hashes,
        prepared_hashes,
        missing_hash_pages,
        compared_pairs: detection.compared_pairs,
        replay_runtime_micros: replay_started.elapsed().as_micros(),
        detection_runtime_micros,
        groups: detection
            .groups
            .into_iter()
            .map(|record| record.group)
            .collect(),
    })
}

fn load_hashes(
    connection: &Connection,
    gallery_id: i64,
    profile_version: u32,
    preferred_entry_id: Option<&str>,
) -> DiagnosticResult<LoadedHashes> {
    let entry_id = if let Some(preferred) = preferred_entry_id {
        Some(preferred.to_owned())
    } else {
        canonical_entry_id(connection, gallery_id)?
    };
    let Some(entry_id) = entry_id else {
        return Ok(LoadedHashes {
            entry_id: None,
            hashes: Vec::new(),
        });
    };
    let mut statement = connection
        .prepare(
            r#"
                SELECT h.entry_id, h.gallery_id, h.source_page_number, h.profile_version,
                       h.artifact_sha256, h.coarse_d_hash_hex, h.detail_d_hash_hex,
                       h.p_hash_hex, h.mean_luma, h.std_dev, h.non_uniform_ratio,
                       h.edge_density, h.width, h.height, h.low_information
                FROM duplicate_page_hashes h
                JOIN download_pages p
                  ON p.entry_id = h.entry_id
                 AND p.gallery_id = h.gallery_id
                 AND p.source_page_number = h.source_page_number
                 AND p.sha256 = h.artifact_sha256
                WHERE h.entry_id = ?1
                  AND h.gallery_id = ?2
                  AND h.profile_version = ?3
                  AND p.state = 'present'
                  AND p.excluded = 0
                ORDER BY h.source_page_number ASC
            "#,
        )
        .map_err(|error| format!("prepare #{} hash replay: {error}", gallery_id))?;
    let rows = statement
        .query_map(
            params![&entry_id, gallery_id, i64::from(profile_version)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, f64>(8)?,
                    row.get::<_, f64>(9)?,
                    row.get::<_, f64>(10)?,
                    row.get::<_, f64>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, bool>(14)?,
                ))
            },
        )
        .map_err(|error| format!("query #{} hash replay: {error}", gallery_id))?;
    let mut hashes = Vec::new();
    for row in rows {
        let (
            stored_entry,
            stored_gallery,
            stored_page,
            stored_profile,
            sha,
            coarse,
            detail,
            phash,
            mean_luma,
            std_dev,
            non_uniform_ratio,
            edge_density,
            width,
            height,
            low_information,
        ) = row.map_err(|error| format!("read #{} hash replay row: {error}", gallery_id))?;
        hashes.push(DuplicatePageHash {
            entry_id: stored_entry,
            gallery_id: GalleryId::new(stored_gallery).map_err(|error| error.to_string())?,
            source_page_number: SourcePageNumber::new(stored_u32(stored_page, "source page")?)
                .map_err(|error| error.to_string())?,
            profile_version: stored_u32(stored_profile, "profile version")?,
            artifact_sha256: ArtifactSha256::new(sha).map_err(|error| error.to_string())?,
            coarse_d_hash: parse_hash_u64(&coarse, "coarse dHash")?,
            detail_d_hash_hex: detail,
            p_hash: parse_hash_u64(&phash, "pHash")?,
            mean_luma,
            std_dev,
            non_uniform_ratio,
            edge_density,
            width: stored_u32(width, "width")?,
            height: stored_u32(height, "height")?,
            low_information,
        });
    }
    Ok(LoadedHashes {
        entry_id: Some(entry_id),
        hashes,
    })
}

/// Fingerprints the exact cached input consumed by one gallery replay.  Every
/// stored profile field is included so two reports cannot be compared after a
/// cache row, artifact revision, or canonical entry silently changes.
fn prediction_input_fingerprint(
    selected_entry_id: Option<&str>,
    hashes: &[DuplicatePageHash],
) -> String {
    let mut digest = Sha256::new();
    fingerprint_field(&mut digest, b"atsumi-internal-corpus-prediction-input-v1");
    match selected_entry_id {
        Some(entry_id) => {
            digest.update([1]);
            fingerprint_field(&mut digest, entry_id.as_bytes());
        }
        None => digest.update([0]),
    }
    let mut ordered = hashes.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.source_page_number
            .cmp(&right.source_page_number)
            .then_with(|| left.artifact_sha256.cmp(&right.artifact_sha256))
            .then_with(|| left.profile_version.cmp(&right.profile_version))
    });
    digest.update((ordered.len() as u64).to_be_bytes());
    for hash in ordered {
        fingerprint_field(&mut digest, hash.entry_id.as_bytes());
        digest.update(hash.gallery_id.get().to_be_bytes());
        digest.update(hash.source_page_number.get().to_be_bytes());
        digest.update(hash.profile_version.to_be_bytes());
        fingerprint_field(&mut digest, hash.artifact_sha256.as_str().as_bytes());
        digest.update(hash.coarse_d_hash.to_be_bytes());
        fingerprint_field(&mut digest, hash.detail_d_hash_hex.as_bytes());
        digest.update(hash.p_hash.to_be_bytes());
        digest.update(hash.mean_luma.to_bits().to_be_bytes());
        digest.update(hash.std_dev.to_bits().to_be_bytes());
        digest.update(hash.non_uniform_ratio.to_bits().to_be_bytes());
        digest.update(hash.edge_density.to_bits().to_be_bytes());
        digest.update(hash.width.to_be_bytes());
        digest.update(hash.height.to_be_bytes());
        digest.update([u8::from(hash.low_information)]);
    }
    format!("{:x}", digest.finalize())
}

fn report_input_fingerprint(predictions: &[GalleryPrediction]) -> String {
    let mut digest = Sha256::new();
    fingerprint_field(&mut digest, b"atsumi-internal-corpus-report-input-v1");
    let mut ordered = predictions.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|prediction| prediction.gallery_id);
    digest.update((ordered.len() as u64).to_be_bytes());
    for prediction in ordered {
        digest.update(prediction.gallery_id.to_be_bytes());
        digest.update(prediction.expected_pages.to_be_bytes());
        fingerprint_field(&mut digest, prediction.input_fingerprint.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn fingerprint_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn canonical_entry_id(
    connection: &Connection,
    gallery_id: i64,
) -> DiagnosticResult<Option<String>> {
    connection
        .query_row(
            r#"
                SELECT a.entry_id
                FROM download_artifacts a
                JOIN download_entries e
                  ON e.entry_id = a.entry_id AND e.gallery_id = a.gallery_id
                WHERE a.gallery_id = ?1
                  AND e.state = 'completed'
                  AND a.state = 'complete'
                  AND a.manifest_relative_path IS NOT NULL
                  AND a.manifest_schema_version IS NOT NULL
                  AND a.writer_version IS NOT NULL
                  AND a.completed_at IS NOT NULL
                ORDER BY a.completed_at DESC, a.revision DESC, a.entry_id ASC
                LIMIT 1
            "#,
            [gallery_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("select #{} canonical artifact: {error}", gallery_id))
}

fn stored_u32(value: i64, label: &str) -> DiagnosticResult<u32> {
    u32::try_from(value).map_err(|_| format!("invalid {label}: {value}"))
}

fn parse_hash_u64(value: &str, label: &str) -> DiagnosticResult<u64> {
    if value.len() != 16 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid {label}: {value:?}"));
    }
    u64::from_str_radix(value, 16).map_err(|error| format!("decode {label}: {error}"))
}

fn evaluate_album(
    album: &GoldAlbum,
    prediction: &GalleryPrediction,
) -> DiagnosticResult<AlbumEvaluation> {
    if prediction.status != "predicted" {
        return Ok(AlbumEvaluation {
            gallery_id: album.gallery_id,
            family_id: album.family_id.clone(),
            role: album.role.clone(),
            scored: false,
            unscored_reason: Some(format!(
                "missing current HashProfile input: {}/{} pages available",
                prediction.available_hashes, album.page_count
            )),
            track_partition: None,
            track_page_pairs: None,
            block_partition: None,
            scene_pairs: None,
            track_matches: Vec::new(),
            block_matches: Vec::new(),
            track_fragmentation: 0,
            distinct_track_merges: 0,
            spurious_short_tracks: 0,
            block_splits: 0,
            block_merges: 0,
            preserve_page_removal: 0,
            hard_negative_scene_merges: 0,
            near_duplicate_distinct_scene_merges: 0,
            non_bridge_page_uses: 0,
            replay_runtime_micros: prediction.replay_runtime_micros,
            detection_runtime_micros: prediction.detection_runtime_micros,
            compared_pairs: prediction.compared_pairs,
        });
    }

    let diagnostic_pages = pages_from_ranges(&album.diagnostic_only_page_ranges)?;
    let gold_tracks = named_gold_tracks(album, &diagnostic_pages)?;
    let predicted_tracks = named_prediction_tracks(prediction, &diagnostic_pages);
    let (track_partition, track_matches) = matched_partition(&gold_tracks, &predicted_tracks);
    let gold_track_pairs = pairs_from_named_sets(&gold_tracks);
    let predicted_track_pairs = pairs_from_named_sets(&predicted_tracks);
    let track_page_pairs = compare_pair_sets(&gold_track_pairs, &predicted_track_pairs);

    let (block_partition, block_matches, block_splits, block_merges) =
        if album.evaluation_scopes.iter().any(|scope| scope == "block") {
            let gold_blocks = named_gold_blocks(album, &diagnostic_pages)?;
            let predicted_blocks = named_prediction_blocks(prediction, &diagnostic_pages);
            let (metric, matches) = matched_partition(&gold_blocks, &predicted_blocks);
            let splits = fragmentation_count(&gold_blocks, &predicted_blocks);
            let merges = fragmentation_count(&predicted_blocks, &gold_blocks);
            (Some(metric), matches, splits, merges)
        } else {
            (None, Vec::new(), 0, 0)
        };

    let (gold_scene_pairs, scene_pair_universe) = gold_scene_pairs(album, &diagnostic_pages)?;
    let prediction_scene_pairs = prediction_scene_pairs(prediction);
    let scoped_prediction_scene_pairs = prediction_scene_pairs
        .intersection(&scene_pair_universe)
        .copied()
        .collect::<BTreeSet<_>>();
    let scene_pairs = (!scene_pair_universe.is_empty())
        .then(|| compare_pair_sets(&gold_scene_pairs, &scoped_prediction_scene_pairs));

    let preserve_pages = pages_from_ranges(&album.preserve_page_ranges)?;
    let pages_in_rows = prediction
        .groups
        .iter()
        .filter(|group| group.pages.len() >= 2)
        .flat_map(|group| group.pages.iter().map(|page| page.source_page))
        .collect::<BTreeSet<_>>();
    let preserve_page_removal = pages_in_rows.intersection(&preserve_pages).count();
    let hard_negative_scene_merges = album
        .hard_negative_pairs
        .iter()
        .filter(|negative| {
            let pair = ordered_pair(negative.pages[0], negative.pages[1]);
            prediction_scene_pairs.contains(&pair)
        })
        .count();
    let mut near_duplicate_pairs = BTreeSet::new();
    for range in &album.near_duplicate_distinct_scene_ranges {
        let pages = pages_from_ranges(std::slice::from_ref(&range.page_range))?;
        add_pairs(&pages, &mut near_duplicate_pairs);
    }
    let near_duplicate_distinct_scene_merges = near_duplicate_pairs
        .intersection(&prediction_scene_pairs)
        .count();
    // PairRun causality is intentionally not reconstructed by the evaluator.
    // A declared non-bridge page appearing in a persisted multi-page scene row
    // is the conservative observable signal that it was accepted as structural
    // evidence. Unannotated pages and relationships remain unscored.
    let non_bridge_pages = pages_from_ranges(&album.non_bridge_page_ranges)?;
    let non_bridge_page_uses = prediction
        .groups
        .iter()
        .filter(|group| group.pages.len() >= 2)
        .flat_map(|group| group.pages.iter().map(|page| page.source_page))
        .filter(|page| non_bridge_pages.contains(page))
        .collect::<BTreeSet<_>>()
        .len();
    let spurious_short_tracks = spurious_short_track_count(&predicted_tracks, &track_matches);

    Ok(AlbumEvaluation {
        gallery_id: album.gallery_id,
        family_id: album.family_id.clone(),
        role: album.role.clone(),
        scored: true,
        unscored_reason: None,
        track_partition: Some(track_partition),
        track_page_pairs: Some(track_page_pairs),
        block_partition,
        scene_pairs,
        track_matches,
        block_matches,
        track_fragmentation: fragmentation_count(&gold_tracks, &predicted_tracks),
        distinct_track_merges: fragmentation_count(&predicted_tracks, &gold_tracks),
        spurious_short_tracks,
        block_splits,
        block_merges,
        preserve_page_removal,
        hard_negative_scene_merges,
        near_duplicate_distinct_scene_merges,
        non_bridge_page_uses,
        replay_runtime_micros: prediction.replay_runtime_micros,
        detection_runtime_micros: prediction.detection_runtime_micros,
        compared_pairs: prediction.compared_pairs,
    })
}

fn spurious_short_track_count(
    predicted_tracks: &BTreeMap<String, BTreeSet<u32>>,
    matches: &[MatchRecord],
) -> usize {
    let matched_prediction_tracks = matches
        .iter()
        .map(|matched| matched.prediction_id.as_str())
        .collect::<BTreeSet<_>>();
    predicted_tracks
        .iter()
        .filter(|(track_id, pages)| {
            pages.len() <= SHORT_TRACK_DIAGNOSTIC_MAX_PAGES
                && !matched_prediction_tracks.contains(track_id.as_str())
        })
        .count()
}

fn named_gold_tracks(
    album: &GoldAlbum,
    excluded: &BTreeSet<u32>,
) -> DiagnosticResult<BTreeMap<String, BTreeSet<u32>>> {
    album
        .album_tracks
        .iter()
        .map(|track| {
            Ok((
                track.id.clone(),
                pages_from_ranges(&track.page_ranges)?
                    .difference(excluded)
                    .copied()
                    .collect(),
            ))
        })
        .collect()
}

fn named_prediction_tracks(
    prediction: &GalleryPrediction,
    excluded: &BTreeSet<u32>,
) -> BTreeMap<String, BTreeSet<u32>> {
    let mut tracks = BTreeMap::<String, BTreeSet<u32>>::new();
    for page in prediction.groups.iter().flat_map(|group| &group.pages) {
        if excluded.contains(&page.source_page) {
            continue;
        }
        if let Some(track_id) = &page.edition_track_id {
            tracks
                .entry(track_id.clone())
                .or_default()
                .insert(page.source_page);
        }
    }
    tracks
}

fn named_gold_blocks(
    album: &GoldAlbum,
    excluded: &BTreeSet<u32>,
) -> DiagnosticResult<BTreeMap<String, BTreeSet<u32>>> {
    album
        .scene_blocks
        .iter()
        .map(|block| {
            let pages = block
                .track_slices
                .values()
                .map(|ranges| pages_from_ranges(ranges))
                .collect::<DiagnosticResult<Vec<_>>>()?
                .into_iter()
                .flatten()
                .filter(|page| !excluded.contains(page))
                .collect();
            Ok((block.id.clone(), pages))
        })
        .collect()
}

fn named_prediction_blocks(
    prediction: &GalleryPrediction,
    excluded: &BTreeSet<u32>,
) -> BTreeMap<String, BTreeSet<u32>> {
    let mut blocks = BTreeMap::<String, BTreeSet<u32>>::new();
    for group in &prediction.groups {
        let pages = blocks.entry(group.block_id.clone()).or_default();
        pages.extend(
            group
                .pages
                .iter()
                .map(|page| page.source_page)
                .filter(|page| !excluded.contains(page)),
        );
    }
    blocks
}

fn matched_partition(
    gold: &BTreeMap<String, BTreeSet<u32>>,
    predicted: &BTreeMap<String, BTreeSet<u32>>,
) -> (Metric, Vec<MatchRecord>) {
    let gold_values = gold.values().collect::<Vec<_>>();
    let prediction_values = predicted.values().collect::<Vec<_>>();
    let weights = gold_values
        .iter()
        .map(|gold_pages| {
            prediction_values
                .iter()
                .map(|prediction_pages| gold_pages.intersection(prediction_pages).count())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let assignment = maximum_weight_assignment(&weights);
    let gold_names = gold.keys().collect::<Vec<_>>();
    let prediction_names = predicted.keys().collect::<Vec<_>>();
    let matches = assignment
        .iter()
        .enumerate()
        .filter_map(|(gold_index, prediction_index)| {
            let prediction_index = (*prediction_index)?;
            let overlap = weights[gold_index][prediction_index];
            (overlap > 0).then(|| MatchRecord {
                gold_id: gold_names[gold_index].clone(),
                prediction_id: prediction_names[prediction_index].clone(),
                overlap_pages: overlap,
            })
        })
        .collect::<Vec<_>>();
    let true_positive = matches.iter().map(|value| value.overlap_pages).sum();
    let gold_total = gold.values().map(BTreeSet::len).sum::<usize>();
    let prediction_total = predicted.values().map(BTreeSet::len).sum::<usize>();
    (
        metric(
            true_positive,
            prediction_total.saturating_sub(true_positive),
            gold_total.saturating_sub(true_positive),
        ),
        matches,
    )
}

fn maximum_weight_assignment(weights: &[Vec<usize>]) -> Vec<Option<usize>> {
    let rows = weights.len();
    let columns = weights.first().map_or(0, Vec::len);
    let size = rows.max(columns);
    if size == 0 {
        return vec![None; rows];
    }
    let mut u = vec![0_i64; size + 1];
    let mut v = vec![0_i64; size + 1];
    let mut p = vec![0_usize; size + 1];
    let mut way = vec![0_usize; size + 1];
    for row in 1..=size {
        p[0] = row;
        let mut column0 = 0_usize;
        let mut min_value = vec![i64::MAX; size + 1];
        let mut used = vec![false; size + 1];
        loop {
            used[column0] = true;
            let current_row = p[column0];
            let mut delta = i64::MAX;
            let mut next_column = 0_usize;
            for column in 1..=size {
                if used[column] {
                    continue;
                }
                let weight = if current_row <= rows && column <= columns {
                    weights[current_row - 1][column - 1] as i64
                } else {
                    0
                };
                let current = -weight - u[current_row] - v[column];
                if current < min_value[column] {
                    min_value[column] = current;
                    way[column] = column0;
                }
                if min_value[column] < delta {
                    delta = min_value[column];
                    next_column = column;
                }
            }
            for column in 0..=size {
                if used[column] {
                    u[p[column]] += delta;
                    v[column] -= delta;
                } else {
                    min_value[column] -= delta;
                }
            }
            column0 = next_column;
            if p[column0] == 0 {
                break;
            }
        }
        loop {
            let previous = way[column0];
            p[column0] = p[previous];
            column0 = previous;
            if column0 == 0 {
                break;
            }
        }
    }
    let mut assignment = vec![None; rows];
    for column in 1..=size {
        if p[column] > 0 && p[column] <= rows && column <= columns {
            assignment[p[column] - 1] = Some(column - 1);
        }
    }
    assignment
}

fn fragmentation_count(
    sources: &BTreeMap<String, BTreeSet<u32>>,
    targets: &BTreeMap<String, BTreeSet<u32>>,
) -> usize {
    sources
        .values()
        .map(|source| {
            targets
                .values()
                .filter(|target| !source.is_disjoint(target))
                .count()
                .saturating_sub(1)
        })
        .sum()
}

fn gold_scene_pairs(
    album: &GoldAlbum,
    excluded: &BTreeSet<u32>,
) -> DiagnosticResult<(PagePairSet, PagePairSet)> {
    let mut positives = BTreeSet::new();
    let mut universe = BTreeSet::new();
    for block in &album.scene_blocks {
        let mut rows = Vec::<BTreeSet<u32>>::new();
        let mut definitive_pages = BTreeSet::new();
        let mut declared_negative_pairs = BTreeSet::new();
        let partial;
        match block.alignment.mode.as_str() {
            "by_position" => {
                partial = false;
                let slices = block
                    .track_slices
                    .values()
                    .map(|ranges| ordered_pages_from_ranges(ranges))
                    .collect::<DiagnosticResult<Vec<_>>>()?;
                let max_rows = slices.iter().map(Vec::len).max().unwrap_or(0);
                for index in 0..max_rows {
                    rows.push(
                        slices
                            .iter()
                            .filter_map(|pages| pages.get(index).copied())
                            .collect(),
                    );
                }
                definitive_pages.extend(slices.into_iter().flatten());
            }
            "explicit_rows" => {
                partial = false;
                rows.extend(
                    block
                        .alignment
                        .rows
                        .iter()
                        .map(|row| row.members.values().copied().collect()),
                );
                for ranges in block.track_slices.values() {
                    definitive_pages.extend(pages_from_ranges(ranges)?);
                }
            }
            "partial" => {
                partial = true;
                rows.extend(
                    block
                        .alignment
                        .known_same_scene_groups
                        .iter()
                        .map(|known| known.pages.iter().copied().collect()),
                );
                definitive_pages.extend(rows.iter().flatten().copied());
                for known in &block.alignment.known_same_scene_groups {
                    for missing_track_id in &known.missing_track_ids {
                        let ranges = block.track_slices.get(missing_track_id).ok_or_else(|| {
                            format!(
                                "{}/{} references missing track {missing_track_id}",
                                album.gallery_id, block.id
                            )
                        })?;
                        for missing_page in pages_from_ranges(ranges)? {
                            for known_page in &known.pages {
                                if missing_page != *known_page
                                    && !excluded.contains(&missing_page)
                                    && !excluded.contains(known_page)
                                {
                                    declared_negative_pairs
                                        .insert(ordered_pair(missing_page, *known_page));
                                }
                            }
                        }
                    }
                }
            }
            "not_scored" => continue,
            other => return Err(format!("unsupported scene alignment mode {other:?}")),
        }
        definitive_pages.retain(|page| !excluded.contains(page));
        let mut block_positives = BTreeSet::new();
        for mut row in rows {
            row.retain(|page| !excluded.contains(page));
            add_pairs(&row, &mut block_positives);
        }
        positives.extend(block_positives.iter().copied());
        if partial {
            // `partial` scores only known positives plus the explicit missing-track negatives.
            // Other alignment and cross-group pairs remain unknown and cannot reduce precision.
            universe.extend(block_positives);
            universe.extend(declared_negative_pairs);
        } else {
            add_pairs(&definitive_pages, &mut universe);
        }
    }
    Ok((positives, universe))
}

fn prediction_scene_pairs(prediction: &GalleryPrediction) -> PagePairSet {
    let mut pairs = BTreeSet::new();
    for group in &prediction.groups {
        add_pairs(
            &group
                .pages
                .iter()
                .map(|page| page.source_page)
                .collect::<BTreeSet<_>>(),
            &mut pairs,
        );
    }
    pairs
}

fn pairs_from_named_sets(values: &BTreeMap<String, BTreeSet<u32>>) -> PagePairSet {
    let mut pairs = BTreeSet::new();
    for pages in values.values() {
        add_pairs(pages, &mut pairs);
    }
    pairs
}

fn add_pairs(pages: &BTreeSet<u32>, output: &mut PagePairSet) {
    let values = pages.iter().copied().collect::<Vec<_>>();
    for left in 0..values.len() {
        for right in (left + 1)..values.len() {
            output.insert((values[left], values[right]));
        }
    }
}

fn ordered_pair(left: u32, right: u32) -> (u32, u32) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn compare_pair_sets(gold: &PagePairSet, predicted: &PagePairSet) -> Metric {
    let true_positive = gold.intersection(predicted).count();
    metric(
        true_positive,
        predicted.len().saturating_sub(true_positive),
        gold.len().saturating_sub(true_positive),
    )
}

fn metric(true_positive: usize, false_positive: usize, false_negative: usize) -> Metric {
    let precision = ratio_or_one(true_positive, true_positive + false_positive);
    let recall = ratio_or_one(true_positive, true_positive + false_negative);
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    Metric {
        true_positive,
        false_positive,
        false_negative,
        precision,
        recall,
        f1,
    }
}

fn ratio_or_one(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn pages_from_ranges(ranges: &[String]) -> DiagnosticResult<BTreeSet<u32>> {
    Ok(ordered_pages_from_ranges(ranges)?.into_iter().collect())
}

fn ordered_pages_from_ranges(ranges: &[String]) -> DiagnosticResult<Vec<u32>> {
    let mut pages = Vec::new();
    for value in ranges {
        let mut parts = value.splitn(2, '-');
        let start = parts
            .next()
            .ok_or_else(|| format!("invalid page range {value:?}"))?
            .parse::<u32>()
            .map_err(|error| format!("invalid page range {value:?}: {error}"))?;
        let end = parts
            .next()
            .map(str::parse::<u32>)
            .transpose()
            .map_err(|error| format!("invalid page range {value:?}: {error}"))?
            .unwrap_or(start);
        if start == 0 || end < start {
            return Err(format!("invalid page range {value:?}"));
        }
        pages.extend(start..=end);
    }
    Ok(pages)
}

fn summarize(evaluations: &[AlbumEvaluation]) -> Summary {
    let scored = evaluations
        .iter()
        .filter(|evaluation| evaluation.scored)
        .collect::<Vec<_>>();
    let all_albums_scored = scored.len() == evaluations.len();
    let album_macro_track_f1 = all_albums_scored
        .then(|| {
            optional_mean(metric_scores(&scored, |value| {
                value.track_partition.as_ref()
            }))
        })
        .flatten();
    let album_macro_block_f1 = all_albums_scored
        .then(|| {
            optional_mean(metric_scores(&scored, |value| {
                value.block_partition.as_ref()
            }))
        })
        .flatten();
    let album_macro_scene_f1 = all_albums_scored
        .then(|| optional_mean(metric_scores(&scored, |value| value.scene_pairs.as_ref())))
        .flatten();
    let mut family_members = BTreeMap::<String, Vec<&AlbumEvaluation>>::new();
    for evaluation in evaluations {
        family_members
            .entry(evaluation.family_id.clone())
            .or_default()
            .push(evaluation);
    }
    let family_scores = family_members
        .into_iter()
        .map(|(family_id, members)| {
            let scored_members = members
                .iter()
                .copied()
                .filter(|member| member.scored)
                .collect::<Vec<_>>();
            let complete = scored_members.len() == members.len();
            FamilyScore {
                family_id,
                albums_total: members.len(),
                scored_albums: scored_members.len(),
                track_f1: complete
                    .then(|| {
                        optional_mean(metric_scores(&scored_members, |value| {
                            value.track_partition.as_ref()
                        }))
                    })
                    .flatten(),
                block_f1: complete
                    .then(|| {
                        optional_mean(metric_scores(&scored_members, |value| {
                            value.block_partition.as_ref()
                        }))
                    })
                    .flatten(),
                scene_f1: complete
                    .then(|| {
                        optional_mean(metric_scores(&scored_members, |value| {
                            value.scene_pairs.as_ref()
                        }))
                    })
                    .flatten(),
            }
        })
        .collect::<Vec<_>>();
    let family_macro_track_f1 = all_albums_scored
        .then(|| {
            optional_mean(
                family_scores
                    .iter()
                    .filter_map(|family| family.track_f1)
                    .collect(),
            )
        })
        .flatten();
    let family_macro_block_f1 = all_albums_scored
        .then(|| {
            optional_mean(
                family_scores
                    .iter()
                    .filter_map(|family| family.block_f1)
                    .collect(),
            )
        })
        .flatten();
    let family_macro_scene_f1 = all_albums_scored
        .then(|| {
            optional_mean(
                family_scores
                    .iter()
                    .filter_map(|family| family.scene_f1)
                    .collect(),
            )
        })
        .flatten();
    let (pair_tp, pair_fp, pair_fn) = scored
        .iter()
        .filter_map(|evaluation| evaluation.track_page_pairs.as_ref())
        .fold((0, 0, 0), |(tp, fp, fn_), metric| {
            (
                tp + metric.true_positive,
                fp + metric.false_positive,
                fn_ + metric.false_negative,
            )
        });
    let preserve_page_removal = scored
        .iter()
        .map(|evaluation| evaluation.preserve_page_removal)
        .sum();
    let hard_negative_scene_merges = scored
        .iter()
        .map(|evaluation| evaluation.hard_negative_scene_merges)
        .sum();
    let near_duplicate_distinct_scene_merges = scored
        .iter()
        .map(|evaluation| evaluation.near_duplicate_distinct_scene_merges)
        .sum();
    let non_bridge_page_uses = scored
        .iter()
        .map(|evaluation| evaluation.non_bridge_page_uses)
        .sum();
    let visible = evaluations
        .iter()
        .filter(|evaluation| evaluation.role == "visible_regression")
        .collect::<Vec<_>>();
    let visible_scored = visible
        .iter()
        .copied()
        .filter(|evaluation| evaluation.scored)
        .collect::<Vec<_>>();
    let visible_regression_safety_pass = visible_scored.len() == visible.len()
        && visible_scored.iter().all(|evaluation| {
            evaluation.preserve_page_removal == 0
                && evaluation.hard_negative_scene_merges == 0
                && evaluation.near_duplicate_distinct_scene_merges == 0
                && evaluation.non_bridge_page_uses == 0
        });
    let adoption_valid = all_albums_scored
        && preserve_page_removal == 0
        && hard_negative_scene_merges == 0
        && near_duplicate_distinct_scene_merges == 0
        && non_bridge_page_uses == 0
        && visible_regression_safety_pass;
    Summary {
        albums_total: evaluations.len(),
        albums_scored: scored.len(),
        albums_missing_hashes: evaluations.len().saturating_sub(scored.len()),
        all_albums_scored,
        adoption_valid,
        album_macro_track_f1,
        family_macro_track_f1,
        album_macro_block_f1,
        family_macro_block_f1,
        album_macro_scene_f1,
        family_macro_scene_f1,
        family_scores,
        page_pair_micro: metric(pair_tp, pair_fp, pair_fn),
        track_fragmentation: scored
            .iter()
            .map(|evaluation| evaluation.track_fragmentation)
            .sum(),
        distinct_track_merges: scored
            .iter()
            .map(|evaluation| evaluation.distinct_track_merges)
            .sum(),
        spurious_short_tracks: scored
            .iter()
            .map(|evaluation| evaluation.spurious_short_tracks)
            .sum(),
        block_splits: scored
            .iter()
            .map(|evaluation| evaluation.block_splits)
            .sum(),
        block_merges: scored
            .iter()
            .map(|evaluation| evaluation.block_merges)
            .sum(),
        preserve_page_removal,
        hard_negative_scene_merges,
        near_duplicate_distinct_scene_merges,
        non_bridge_page_uses,
        visible_regression_albums_total: visible.len(),
        visible_regression_albums_scored: visible_scored.len(),
        visible_regression_track_f1: (visible_scored.len() == visible.len())
            .then(|| {
                optional_mean(metric_scores(&visible_scored, |value| {
                    value.track_partition.as_ref()
                }))
            })
            .flatten(),
        visible_regression_safety_pass,
        compared_pairs: evaluations
            .iter()
            .map(|evaluation| evaluation.compared_pairs)
            .sum(),
        replay_runtime_micros: evaluations
            .iter()
            .map(|evaluation| evaluation.replay_runtime_micros)
            .sum(),
        detection_runtime_micros: evaluations
            .iter()
            .map(|evaluation| evaluation.detection_runtime_micros)
            .sum(),
    }
}

fn metric_scores<'a>(
    evaluations: &[&'a AlbumEvaluation],
    select: impl Fn(&'a AlbumEvaluation) -> Option<&'a Metric>,
) -> Vec<f64> {
    evaluations
        .iter()
        .filter_map(|evaluation| select(evaluation).map(|metric| metric.f1))
        .collect()
}

fn optional_mean(values: Vec<f64>) -> Option<f64> {
    (!values.is_empty()).then(|| mean(&values))
}

fn compare_reports(
    baseline_report: &ComparisonEnvelope,
    candidate_report: &ComparisonEnvelope,
) -> AdoptionComparison {
    let baseline = &baseline_report.summary;
    let candidate = &candidate_report.summary;
    let mut reasons = Vec::new();
    if baseline_report.corpus_schema_version != candidate_report.corpus_schema_version
        || baseline_report.corpus_id != candidate_report.corpus_id
        || baseline_report.hash_profile != candidate_report.hash_profile
    {
        reasons.push("evaluation_identity_mismatch".into());
    }
    if baseline_report.input_fingerprint != candidate_report.input_fingerprint {
        reasons.push("evaluation_input_fingerprint_mismatch".into());
    }
    let baseline_albums = baseline_report
        .evaluations
        .iter()
        .map(|album| {
            (
                album.gallery_id,
                album.family_id.as_str(),
                album.role.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    let candidate_albums = candidate_report
        .evaluations
        .iter()
        .map(|album| {
            (
                album.gallery_id,
                album.family_id.as_str(),
                album.role.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    if baseline_albums != candidate_albums
        || baseline.albums_total != candidate.albums_total
        || baseline.family_scores.len() != candidate.family_scores.len()
    {
        reasons.push("evaluation_album_or_family_set_mismatch".into());
    }
    if !baseline.all_albums_scored {
        reasons.push("baseline_incomplete".into());
    }
    if !candidate.all_albums_scored {
        reasons.push("candidate_incomplete".into());
    }
    if !candidate.adoption_valid {
        reasons.push("candidate_safety_gate_failed".into());
    }
    if candidate.non_bridge_page_uses > 0 {
        reasons.push("non_bridge_page_use_detected".into());
    }
    match (
        baseline.family_macro_track_f1,
        candidate.family_macro_track_f1,
    ) {
        (Some(baseline_score), Some(candidate_score)) if candidate_score <= baseline_score => {
            reasons.push("family_macro_track_not_improved".into());
        }
        (Some(_), Some(_)) => {}
        _ => reasons.push("family_macro_track_unavailable".into()),
    }
    if option_regressed(
        baseline.family_macro_block_f1,
        candidate.family_macro_block_f1,
    ) {
        reasons.push("family_macro_block_regressed".into());
    }
    if option_regressed(
        baseline.family_macro_scene_f1,
        candidate.family_macro_scene_f1,
    ) {
        reasons.push("family_macro_scene_regressed".into());
    }
    let candidate_families = candidate
        .family_scores
        .iter()
        .map(|family| (family.family_id.as_str(), family))
        .collect::<BTreeMap<_, _>>();
    for baseline_family in &baseline.family_scores {
        let Some(baseline_score) = baseline_family.track_f1 else {
            continue;
        };
        match candidate_families
            .get(baseline_family.family_id.as_str())
            .and_then(|family| family.track_f1)
        {
            Some(candidate_score) if candidate_score + 1e-12 >= baseline_score => {}
            Some(_) => reasons.push(format!(
                "family_track_regression:{}",
                baseline_family.family_id
            )),
            None => reasons.push(format!(
                "family_track_missing:{}",
                baseline_family.family_id
            )),
        }
    }
    let candidate_albums_by_id = candidate_report
        .evaluations
        .iter()
        .map(|album| (album.gallery_id, album))
        .collect::<BTreeMap<_, _>>();
    for baseline_album in baseline_report
        .evaluations
        .iter()
        .filter(|album| album.role == "visible_regression")
    {
        let candidate_album = candidate_albums_by_id
            .get(&baseline_album.gallery_id)
            .copied()
            .filter(|album| album.scored);
        compare_visible_album_metric(
            baseline_album.gallery_id,
            "track",
            baseline_album.track_partition.as_ref(),
            candidate_album.and_then(|album| album.track_partition.as_ref()),
            &mut reasons,
        );
        compare_visible_album_metric(
            baseline_album.gallery_id,
            "block",
            baseline_album.block_partition.as_ref(),
            candidate_album.and_then(|album| album.block_partition.as_ref()),
            &mut reasons,
        );
        compare_visible_album_metric(
            baseline_album.gallery_id,
            "scene",
            baseline_album.scene_pairs.as_ref(),
            candidate_album.and_then(|album| album.scene_pairs.as_ref()),
            &mut reasons,
        );
    }
    if candidate.track_fragmentation > baseline.track_fragmentation {
        reasons.push("track_fragmentation_increased".into());
    }
    if candidate.distinct_track_merges > baseline.distinct_track_merges {
        reasons.push("distinct_track_merges_increased".into());
    }
    if candidate.spurious_short_tracks > baseline.spurious_short_tracks {
        reasons.push("spurious_short_tracks_increased".into());
    }
    if candidate.block_splits > baseline.block_splits {
        reasons.push("block_splits_increased".into());
    }
    if candidate.block_merges > baseline.block_merges {
        reasons.push("block_merges_increased".into());
    }
    if candidate.page_pair_micro.precision + 1e-12 < baseline.page_pair_micro.precision {
        reasons.push("page_pair_precision_regressed".into());
    }
    if candidate.page_pair_micro.f1 + 1e-12 < baseline.page_pair_micro.f1 {
        reasons.push("page_pair_f1_regressed".into());
    }
    if candidate.compared_pairs != baseline.compared_pairs {
        reasons.push("compared_pair_count_changed".into());
    }
    AdoptionComparison {
        adopt: reasons.is_empty(),
        reasons,
        family_macro_track_delta: option_delta(
            baseline.family_macro_track_f1,
            candidate.family_macro_track_f1,
        ),
        album_macro_track_delta: option_delta(
            baseline.album_macro_track_f1,
            candidate.album_macro_track_f1,
        ),
        family_macro_block_delta: option_delta(
            baseline.family_macro_block_f1,
            candidate.family_macro_block_f1,
        ),
        family_macro_scene_delta: option_delta(
            baseline.family_macro_scene_f1,
            candidate.family_macro_scene_f1,
        ),
        visible_regression_track_delta: option_delta(
            baseline.visible_regression_track_f1,
            candidate.visible_regression_track_f1,
        ),
        compared_pair_delta: i128::from(candidate.compared_pairs)
            - i128::from(baseline.compared_pairs),
    }
}

fn compare_visible_album_metric(
    gallery_id: i64,
    metric_name: &str,
    baseline: Option<&Metric>,
    candidate: Option<&Metric>,
    reasons: &mut Vec<String>,
) {
    match (baseline, candidate) {
        (Some(left), Some(right)) if right.f1 + 1e-12 < left.f1 => reasons.push(format!(
            "visible_regression_{metric_name}_decreased:{gallery_id}"
        )),
        (Some(_), None) => reasons.push(format!(
            "visible_regression_{metric_name}_missing:{gallery_id}"
        )),
        _ => {}
    }
}

fn option_regressed(baseline: Option<f64>, candidate: Option<f64>) -> bool {
    matches!((baseline, candidate), (Some(left), Some(right)) if right + 1e-12 < left)
        || matches!((baseline, candidate), (Some(_), None))
}

fn option_delta(baseline: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    baseline.zip(candidate).map(|(left, right)| right - left)
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

#[test]
fn matching_is_permutation_invariant_and_penalizes_fragments() {
    let gold = BTreeMap::from([
        ("t01".into(), BTreeSet::from([1, 2, 3, 4])),
        ("t02".into(), BTreeSet::from([5, 6, 7, 8])),
    ]);
    let prediction = BTreeMap::from([
        ("set-z".into(), BTreeSet::from([5, 6, 7, 8])),
        ("set-a".into(), BTreeSet::from([1, 2])),
        ("set-b".into(), BTreeSet::from([3, 4])),
    ]);
    let (score, matches) = matched_partition(&gold, &prediction);
    assert_eq!(matches.len(), 2);
    assert_eq!(score.true_positive, 6);
    assert_eq!(score.false_positive, 2);
    assert_eq!(score.false_negative, 2);
    assert_eq!(fragmentation_count(&gold, &prediction), 1);
}

#[test]
fn empty_gold_partition_only_passes_with_no_prediction() {
    let gold = BTreeMap::new();
    let empty_prediction = BTreeMap::new();
    assert_eq!(matched_partition(&gold, &empty_prediction).0.f1, 1.0);
    let prediction = BTreeMap::from([("short".into(), BTreeSet::from([1, 2]))]);
    assert_eq!(matched_partition(&gold, &prediction).0.f1, 0.0);
}

#[test]
fn prediction_label_renaming_does_not_change_partition_score() {
    let gold = BTreeMap::from([
        ("t01".into(), BTreeSet::from([1, 2, 3])),
        ("t02".into(), BTreeSet::from([4, 5, 6])),
    ]);
    let first = BTreeMap::from([
        ("set-a".into(), BTreeSet::from([4, 5, 6])),
        ("set-b".into(), BTreeSet::from([1, 2, 3])),
    ]);
    let renamed = BTreeMap::from([
        ("arbitrary-99".into(), BTreeSet::from([1, 2, 3])),
        ("arbitrary-01".into(), BTreeSet::from([4, 5, 6])),
    ]);
    let left = matched_partition(&gold, &first).0;
    let right = matched_partition(&gold, &renamed).0;
    assert_eq!(
        (left.true_positive, left.false_positive, left.false_negative),
        (
            right.true_positive,
            right.false_positive,
            right.false_negative
        )
    );
    assert_eq!(left.f1, right.f1);
}

#[test]
fn partial_alignment_scores_only_declared_positive_pairs() {
    let album = GoldAlbum {
        gallery_id: 1,
        family_id: "family".into(),
        role: "tuning".into(),
        page_count: 4,
        expected_edition_structure: "edition_tracks".into(),
        evaluation_scopes: vec!["scene".into()],
        album_tracks: Vec::new(),
        scene_blocks: vec![GoldBlock {
            id: "b01".into(),
            track_slices: BTreeMap::from([
                ("t01".into(), vec!["1".into()]),
                ("t02".into(), vec!["2".into(), "4".into()]),
                ("t03".into(), vec!["3".into()]),
            ]),
            alignment: GoldAlignment {
                mode: "partial".into(),
                rows: Vec::new(),
                known_same_scene_groups: vec![KnownSceneGroup {
                    id: "known-1".into(),
                    pages: vec![1, 3],
                    missing_track_ids: vec!["t02".into()],
                }],
            },
            notes: Vec::new(),
        }],
        global_track_continuity_across_blocks: false,
        preserve_page_ranges: Vec::new(),
        diagnostic_only_page_ranges: Vec::new(),
        hard_negative_pairs: Vec::new(),
        near_duplicate_distinct_scene_ranges: Vec::new(),
        non_bridge_page_ranges: Vec::new(),
        notes: Vec::new(),
    };
    let (positives, universe) = gold_scene_pairs(&album, &BTreeSet::new()).unwrap();
    assert_eq!(positives, BTreeSet::from([(1, 3)]));
    assert_eq!(
        universe,
        BTreeSet::from([(1, 2), (1, 3), (1, 4), (2, 3), (3, 4)])
    );
    assert!(!universe.contains(&(2, 4)));
}

#[test]
fn matched_short_track_is_not_spurious_but_unmatched_short_track_is() {
    let prediction = BTreeMap::from([
        ("valid-short".into(), BTreeSet::from([1, 2])),
        ("unmatched-short".into(), BTreeSet::from([3, 4])),
    ]);
    let matches = vec![MatchRecord {
        gold_id: "t01".into(),
        prediction_id: "valid-short".into(),
        overlap_pages: 2,
    }];
    assert_eq!(spurious_short_track_count(&prediction, &matches), 1);
}

#[test]
fn canonical_entry_uses_completed_at_then_revision_then_entry_id() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            r#"
                CREATE TABLE download_entries (
                    entry_id TEXT PRIMARY KEY, gallery_id INTEGER NOT NULL, state TEXT NOT NULL
                );
                CREATE TABLE download_artifacts (
                    entry_id TEXT PRIMARY KEY, gallery_id INTEGER NOT NULL, revision INTEGER NOT NULL,
                    state TEXT NOT NULL, manifest_relative_path TEXT,
                    manifest_schema_version INTEGER, writer_version TEXT, completed_at TEXT
                );
                INSERT INTO download_entries VALUES ('older-high-revision', 1, 'completed');
                INSERT INTO download_entries VALUES ('newer-low-revision', 1, 'completed');
                INSERT INTO download_artifacts VALUES (
                    'older-high-revision', 1, 99, 'complete', 'manifest.json', 1, 'test', '2026-01-01'
                );
                INSERT INTO download_artifacts VALUES (
                    'newer-low-revision', 1, 1, 'complete', 'manifest.json', 1, 'test', '2026-02-01'
                );
            "#,
        )
        .unwrap();
    assert_eq!(
        canonical_entry_id(&connection, 1).unwrap().as_deref(),
        Some("newer-low-revision")
    );
}

#[test]
fn read_only_connection_rejects_writes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("read-only.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE evidence (value INTEGER); INSERT INTO evidence VALUES (1);")
        .unwrap();
    drop(connection);
    let read_only = open_read_only(&path).unwrap();
    assert_eq!(
        read_only
            .query_row("SELECT value FROM evidence", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(read_only
        .execute("INSERT INTO evidence VALUES (2)", [])
        .is_err());
}

#[test]
fn runtime_output_path_rejects_parent_escape() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let escaped = repository
        .join(".runtime")
        .join("diagnostics")
        .join("..")
        .join("escape.sqlite3");
    assert!(resolve_runtime_output_path(&escaped, "fixture").is_err());
}

#[test]
fn runtime_write_helpers_reject_linked_leaf_and_prediction_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let diagnostics = temporary.path().join("diagnostics");
    let outside = temporary.path().join("outside");
    std::fs::create_dir(&diagnostics).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let resolved_diagnostics = std::fs::canonicalize(&diagnostics).unwrap();

    let outside_file = outside.join("report.json");
    std::fs::write(&outside_file, b"outside").unwrap();
    let linked_file = diagnostics.join("report.json");
    if create_test_file_symlink(&outside_file, &linked_file).is_ok() {
        assert!(resolve_plain_output_leaf(
            &resolved_diagnostics,
            linked_file.file_name().unwrap(),
            "fixture report"
        )
        .is_err());
        std::fs::remove_file(&linked_file).unwrap();
    }

    let linked_predictions = diagnostics.join("internal-duplicate-predictions");
    if create_test_directory_symlink(&outside, &linked_predictions).is_ok() {
        assert!(ensure_plain_child_directory(
            &resolved_diagnostics,
            &linked_predictions,
            "fixture prediction directory"
        )
        .is_err());
    }
    assert!(has_windows_reparse_attribute(0x400));
    assert!(!has_windows_reparse_attribute(0));
}

#[cfg(windows)]
fn create_test_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(windows))]
fn create_test_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_test_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(not(windows))]
fn create_test_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[test]
fn not_scored_alignment_produces_no_scene_labels() {
    let album = semantic_album(GoldAlignment {
        mode: "not_scored".into(),
        rows: Vec::new(),
        known_same_scene_groups: Vec::new(),
    });
    let (positives, universe) = gold_scene_pairs(&album, &BTreeSet::new()).unwrap();
    assert!(positives.is_empty());
    assert!(universe.is_empty());
}

#[test]
fn diagnostic_pages_are_excluded_from_track_accuracy() {
    let mut album = semantic_album(GoldAlignment {
        mode: "not_scored".into(),
        rows: Vec::new(),
        known_same_scene_groups: Vec::new(),
    });
    album.album_tracks = vec![GoldTrack {
        id: "t01".into(),
        human_label: "report-only".into(),
        page_ranges: vec!["1-3".into()],
    }];
    album.diagnostic_only_page_ranges = vec!["2".into()];
    let excluded = pages_from_ranges(&album.diagnostic_only_page_ranges).unwrap();
    assert_eq!(
        named_gold_tracks(&album, &excluded).unwrap()["t01"],
        BTreeSet::from([1, 3])
    );
}

#[test]
fn safety_labels_detect_preserve_hard_negative_and_near_distinct_merges() {
    let mut album = semantic_album(GoldAlignment {
        mode: "not_scored".into(),
        rows: Vec::new(),
        known_same_scene_groups: Vec::new(),
    });
    album.expected_edition_structure = "no_edition_tracks".into();
    album.album_tracks.clear();
    album.evaluation_scopes = vec!["album_track".into(), "safety".into()];
    album.preserve_page_ranges = vec!["1".into()];
    album.hard_negative_pairs = vec![HardNegativePair {
        pages: [2, 3],
        relation: "must_not_same_scene".into(),
        note: "fixture".into(),
    }];
    album.near_duplicate_distinct_scene_ranges = vec![NearDuplicateRange {
        track_id: None,
        page_range: "1-3".into(),
        note: "fixture".into(),
    }];
    let evidence = [1, 2, 3]
        .into_iter()
        .map(|source_page| InternalPageEvidence {
            source_page,
            exact_sha256: false,
            visual_similarity: 0.9,
            detail_hash_distance: 1,
            low_information: false,
            edition_track_id: None,
            edition_track_ordinal: None,
        })
        .collect();
    let prediction = GalleryPrediction {
        gallery_id: 1,
        expected_pages: 3,
        status: "predicted",
        entry_id: Some("entry".into()),
        input_fingerprint: "fixture-input".into(),
        input_source: "fixture".into(),
        available_hashes: 3,
        cached_hashes: 3,
        prepared_hashes: 0,
        missing_hash_pages: Vec::new(),
        compared_pairs: 3,
        replay_runtime_micros: 0,
        detection_runtime_micros: 0,
        groups: vec![InternalDuplicateGroup {
            group_id: "row".into(),
            block_id: "block".into(),
            sequence_index: 0,
            revision: 0,
            entry_id: "entry".into(),
            gallery_id: GalleryId::new(1).unwrap(),
            relation: InternalMatchKind::TranslationVisual,
            confidence: 0.9,
            recommended_keep_source_page: 1,
            pages: evidence,
            resolved: false,
            created_at: String::new(),
            updated_at: String::new(),
        }],
    };
    let unannotated_result = evaluate_album(&album, &prediction).unwrap();
    assert_eq!(unannotated_result.non_bridge_page_uses, 0);
    album.non_bridge_page_ranges = vec!["2".into()];
    let result = evaluate_album(&album, &prediction).unwrap();
    assert_eq!(result.preserve_page_removal, 1);
    assert_eq!(result.hard_negative_scene_merges, 1);
    assert_eq!(result.near_duplicate_distinct_scene_merges, 3);
    assert_eq!(result.non_bridge_page_uses, 1);
    let summary = summarize(&[result]);
    assert_eq!(summary.non_bridge_page_uses, 1);
    assert!(!summary.adoption_valid);
}

#[test]
fn completeness_gate_invalidates_partial_macro_and_comparator_is_deterministic() {
    let baseline = comparison_fixture(vec![evaluation_fixture(1, "family", true, 0.5)]);
    let candidate = comparison_fixture(vec![evaluation_fixture(1, "family", true, 0.6)]);
    let comparison = compare_reports(&baseline, &candidate);
    assert!(comparison.adopt, "{:?}", comparison.reasons);

    let incomplete = comparison_fixture(vec![
        evaluation_fixture(1, "family", true, 0.6),
        evaluation_fixture(2, "family", false, 0.0),
    ]);
    assert!(!incomplete.summary.all_albums_scored);
    assert!(!incomplete.summary.adoption_valid);
    assert!(incomplete.summary.album_macro_track_f1.is_none());
    assert!(!compare_reports(&baseline, &incomplete).adopt);
}

#[test]
fn comparator_rejects_identity_runtime_shape_and_per_album_regressions() {
    let mut baseline_evaluation = evaluation_fixture(1, "family", true, 0.5);
    baseline_evaluation.role = "visible_regression".into();
    baseline_evaluation.block_partition = Some(metric(8, 2, 2));
    baseline_evaluation.scene_pairs = Some(metric(8, 2, 2));
    let mut candidate_evaluation = evaluation_fixture(1, "family", true, 0.4);
    candidate_evaluation.role = "visible_regression".into();
    candidate_evaluation.block_partition = Some(metric(7, 3, 3));
    candidate_evaluation.scene_pairs = Some(metric(7, 3, 3));
    let baseline = comparison_fixture(vec![baseline_evaluation]);
    let mut candidate = comparison_fixture(vec![candidate_evaluation]);
    candidate.summary.family_macro_track_f1 = Some(0.6);
    candidate.summary.family_scores[0].track_f1 = Some(0.6);
    assert!(compare_reports(&baseline, &candidate)
        .reasons
        .iter()
        .any(|reason| reason.starts_with("visible_regression_track_decreased:")));
    let visible_reasons = compare_reports(&baseline, &candidate).reasons;
    assert!(visible_reasons
        .iter()
        .any(|reason| reason.starts_with("visible_regression_block_decreased:")));
    assert!(visible_reasons
        .iter()
        .any(|reason| reason.starts_with("visible_regression_scene_decreased:")));

    candidate.hash_profile.visual_match_threshold += 0.01;
    assert!(compare_reports(&baseline, &candidate)
        .reasons
        .contains(&"evaluation_identity_mismatch".to_owned()));
    candidate.hash_profile = baseline.hash_profile.clone();
    candidate.input_fingerprint = "different-input".into();
    assert!(compare_reports(&baseline, &candidate)
        .reasons
        .contains(&"evaluation_input_fingerprint_mismatch".to_owned()));
    candidate.input_fingerprint = baseline.input_fingerprint.clone();
    candidate.summary.compared_pairs += 1;
    assert!(compare_reports(&baseline, &candidate)
        .reasons
        .contains(&"compared_pair_count_changed".to_owned()));
    candidate.summary.compared_pairs = baseline.summary.compared_pairs;
    candidate.summary.family_macro_block_f1 = Some(0.4);
    let mut baseline_with_block = baseline;
    baseline_with_block.summary.family_macro_block_f1 = Some(0.5);
    assert!(compare_reports(&baseline_with_block, &candidate)
        .reasons
        .contains(&"family_macro_block_regressed".to_owned()));

    let baseline = comparison_fixture(vec![evaluation_fixture(1, "family", true, 0.5)]);
    let mut candidate = comparison_fixture(vec![evaluation_fixture(1, "family", true, 0.6)]);
    candidate.summary.track_fragmentation = baseline.summary.track_fragmentation + 1;
    candidate.summary.distinct_track_merges = baseline.summary.distinct_track_merges + 1;
    candidate.summary.spurious_short_tracks = baseline.summary.spurious_short_tracks + 1;
    candidate.summary.block_splits = baseline.summary.block_splits + 1;
    candidate.summary.block_merges = baseline.summary.block_merges + 1;
    candidate.summary.non_bridge_page_uses = 1;
    candidate.summary.page_pair_micro.precision = baseline.summary.page_pair_micro.precision - 0.1;
    candidate.summary.page_pair_micro.f1 = baseline.summary.page_pair_micro.f1 - 0.1;
    let reasons = compare_reports(&baseline, &candidate).reasons;
    for expected in [
        "track_fragmentation_increased",
        "distinct_track_merges_increased",
        "spurious_short_tracks_increased",
        "block_splits_increased",
        "block_merges_increased",
        "non_bridge_page_use_detected",
        "page_pair_precision_regressed",
        "page_pair_f1_regressed",
    ] {
        assert!(reasons.contains(&expected.to_owned()), "missing {expected}");
    }
}

#[test]
fn input_fingerprint_is_order_stable_and_covers_every_hash_field() {
    let first = duplicate_hash_fixture(1, 'a');
    let second = duplicate_hash_fixture(2, 'b');
    let ordered = vec![first.clone(), second.clone()];
    let reversed = vec![second, first.clone()];
    let fingerprint = prediction_input_fingerprint(Some("entry-a"), &ordered);
    assert_eq!(
        fingerprint,
        prediction_input_fingerprint(Some("entry-a"), &reversed)
    );
    assert_ne!(
        fingerprint,
        prediction_input_fingerprint(Some("entry-b"), &ordered)
    );

    let mut changed = ordered.clone();
    changed[0].edge_density = f64::from_bits(changed[0].edge_density.to_bits() + 1);
    assert_ne!(
        fingerprint,
        prediction_input_fingerprint(Some("entry-a"), &changed)
    );

    let report_left = vec![
        prediction_fingerprint_fixture(2, "two"),
        prediction_fingerprint_fixture(1, "one"),
    ];
    let report_right = vec![
        prediction_fingerprint_fixture(1, "one"),
        prediction_fingerprint_fixture(2, "two"),
    ];
    assert_eq!(
        report_input_fingerprint(&report_left),
        report_input_fingerprint(&report_right)
    );
    let changed_report = vec![
        prediction_fingerprint_fixture(1, "changed"),
        prediction_fingerprint_fixture(2, "two"),
    ];
    assert_ne!(
        report_input_fingerprint(&report_left),
        report_input_fingerprint(&changed_report)
    );
}

fn semantic_album(alignment: GoldAlignment) -> GoldAlbum {
    GoldAlbum {
        gallery_id: 1,
        family_id: "family".into(),
        role: "tuning".into(),
        page_count: 3,
        expected_edition_structure: "edition_tracks".into(),
        evaluation_scopes: vec!["album_track".into(), "scene".into()],
        album_tracks: vec![GoldTrack {
            id: "t01".into(),
            human_label: "report-only".into(),
            page_ranges: vec!["1-3".into()],
        }],
        scene_blocks: vec![GoldBlock {
            id: "b01".into(),
            track_slices: BTreeMap::from([("t01".into(), vec!["1-3".into()])]),
            alignment,
            notes: Vec::new(),
        }],
        global_track_continuity_across_blocks: false,
        preserve_page_ranges: Vec::new(),
        diagnostic_only_page_ranges: Vec::new(),
        hard_negative_pairs: Vec::new(),
        near_duplicate_distinct_scene_ranges: Vec::new(),
        non_bridge_page_ranges: Vec::new(),
        notes: Vec::new(),
    }
}

fn evaluation_fixture(
    gallery_id: i64,
    family_id: &str,
    scored: bool,
    track_f1: f64,
) -> AlbumEvaluation {
    AlbumEvaluation {
        gallery_id,
        family_id: family_id.into(),
        role: "tuning".into(),
        scored,
        unscored_reason: (!scored).then_some("fixture missing".into()),
        track_partition: scored.then_some(Metric {
            true_positive: 1,
            false_positive: 0,
            false_negative: 0,
            precision: track_f1,
            recall: track_f1,
            f1: track_f1,
        }),
        track_page_pairs: scored.then_some(metric(1, 0, 0)),
        block_partition: None,
        scene_pairs: None,
        track_matches: Vec::new(),
        block_matches: Vec::new(),
        track_fragmentation: 0,
        distinct_track_merges: 0,
        spurious_short_tracks: 0,
        block_splits: 0,
        block_merges: 0,
        preserve_page_removal: 0,
        hard_negative_scene_merges: 0,
        near_duplicate_distinct_scene_merges: 0,
        non_bridge_page_uses: 0,
        replay_runtime_micros: 0,
        detection_runtime_micros: 0,
        compared_pairs: 1,
    }
}

fn duplicate_hash_fixture(source_page: u32, sha_character: char) -> DuplicatePageHash {
    DuplicatePageHash {
        entry_id: "entry-a".into(),
        gallery_id: GalleryId::new(1).unwrap(),
        source_page_number: SourcePageNumber::new(source_page).unwrap(),
        profile_version: HashProfile::current().profile_version,
        artifact_sha256: ArtifactSha256::new(sha_character.to_string().repeat(64)).unwrap(),
        coarse_d_hash: u64::from(source_page),
        detail_d_hash_hex: format!("{source_page:0256x}"),
        p_hash: u64::from(source_page) << 1,
        mean_luma: 10.25 + f64::from(source_page),
        std_dev: 20.5 + f64::from(source_page),
        non_uniform_ratio: 0.75,
        edge_density: 0.25,
        width: 100,
        height: 200,
        low_information: false,
    }
}

fn prediction_fingerprint_fixture(gallery_id: i64, input_fingerprint: &str) -> GalleryPrediction {
    GalleryPrediction {
        gallery_id,
        expected_pages: 1,
        status: "predicted",
        entry_id: Some(format!("entry-{gallery_id}")),
        input_fingerprint: input_fingerprint.into(),
        input_source: "fixture".into(),
        available_hashes: 1,
        cached_hashes: 1,
        prepared_hashes: 0,
        missing_hash_pages: Vec::new(),
        compared_pairs: 0,
        replay_runtime_micros: 0,
        detection_runtime_micros: 0,
        groups: Vec::new(),
    }
}

fn comparison_fixture(evaluations: Vec<AlbumEvaluation>) -> ComparisonEnvelope {
    let summary = summarize(&evaluations);
    let comparison_albums = evaluations
        .iter()
        .map(|evaluation| ComparisonAlbum {
            gallery_id: evaluation.gallery_id,
            family_id: evaluation.family_id.clone(),
            role: evaluation.role.clone(),
            scored: evaluation.scored,
            track_partition: evaluation.track_partition.clone(),
            block_partition: evaluation.block_partition.clone(),
            scene_pairs: evaluation.scene_pairs.clone(),
        })
        .collect();
    ComparisonEnvelope {
        corpus_schema_version: 1,
        corpus_id: "atsumi-internal-duplicate-dev-v1".into(),
        hash_profile: HashProfileFingerprint::from(&HashProfile::current()),
        input_fingerprint: "fixture-report-input".into(),
        evaluations: comparison_albums,
        summary,
    }
}
