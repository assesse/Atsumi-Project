use std::collections::{BTreeMap, BTreeSet};

use crate::domain::{
    DuplicatePageHash, DuplicatePagePair, HashProfile, InternalDuplicateGroup, InternalGroupRecord,
    InternalMatchKind, InternalPageEvidence, INTERNAL_DUPLICATE_ALGORITHM_VERSION,
};

use super::duplicate_analyzer::HashedArtifact;

const DETAIL_HASH_BYTES: usize = 128;
const DETAIL_HASH_BITS: u32 = 1024;
const MIN_PAIR_RUN_ROWS: usize = 2;
/// Retain a small, deterministic Pareto frontier at each monotonic edge. A
/// single longest/minimum-gap predecessor can hide a stronger alignment that
/// crosses one missing scene.
const PAIR_RUN_PATHS_PER_EDGE: usize = 4;
/// A fixed cap keeps structural adoption linear in the number of candidate
/// edges with a bounded factor, even for a dense 499-page artifact.
const MAX_STRUCTURAL_PAIR_RUNS: usize = 512;
/// A fixed beam keeps the structural search deterministic and bounded while
/// retaining alternatives to an early, stitched monotonic run.
const STRUCTURAL_BEAM_WIDTH: usize = 8;
/// Keep a small number of equally covered alternatives so a triangle can
/// complete after an initially lower-ranked, but structurally cleaner run.
const STRUCTURAL_STATES_PER_COVERAGE: usize = 2;
/// Treat tiny score differences as a tie when ranking two complete multiway
/// hypotheses.  Hash-derived scores vary slightly across otherwise coherent
/// edition rows; a material weak link should still beat extra coverage.
const MULTIWAY_QUALITY_TIE_EPSILON: f64 = 0.012;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InternalDetection {
    pub groups: Vec<InternalGroupRecord>,
    pub compared_pairs: u64,
}

#[derive(Clone)]
struct PreparedInternalPage {
    original: DuplicatePageHash,
    detail_bytes: Option<[u8; DETAIL_HASH_BYTES]>,
}

#[derive(Clone)]
struct Edge {
    left: usize,
    right: usize,
    evidence: DuplicatePagePair,
}

#[derive(Clone)]
struct PairRun {
    edges: Vec<usize>,
    average_similarity: f64,
    exact_count: usize,
    cumulative_gap: usize,
}

#[derive(Clone, PartialEq)]
struct MonotonicPath {
    predecessor: Option<(usize, usize)>,
    length: usize,
    similarity_sum: f64,
    minimum_similarity: f64,
    exact_count: usize,
    cumulative_gap: usize,
    offset_sum: usize,
}

#[derive(Clone, Default)]
struct SceneBlock {
    rows: Vec<BTreeSet<usize>>,
    page_rows: BTreeMap<usize, usize>,
}

#[derive(Clone, Default)]
struct StructuralBlock {
    scene: SceneBlock,
    tracks: TrackAssignments,
    /// The paired page sequences accepted for this block.  They are retained
    /// only to establish deterministic album-level track scopes after the
    /// local rows and tracks are complete.
    pair_constraints: Vec<(Vec<usize>, Vec<usize>)>,
    /// Local ordinal -> (scope block index, album-level ordinal).
    track_scopes: BTreeMap<u32, (usize, u32)>,
}

#[derive(Clone, Default)]
struct StructuralState {
    blocks: Vec<StructuralBlock>,
    page_blocks: BTreeMap<usize, usize>,
    accepted_edges: BTreeSet<usize>,
    accepted_alignment_gap: usize,
    quality: StructuralQuality,
}

#[derive(Clone, Copy, Default)]
struct StructuralQuality {
    direct_pairs: usize,
    possible_pairs: usize,
    direct_similarity_sum: f64,
    minimum_direct_similarity: f64,
    triangle_surplus: usize,
    track_count: usize,
    track_fragment_penalty: usize,
    track_gap: usize,
}

type StructuralBlockSignature = (Vec<Vec<usize>>, Vec<(usize, u32)>);
type StructuralStateSignature = (Vec<usize>, Vec<StructuralBlockSignature>);

/// Output-level evidence measures shared by the two independently-produced
/// edition candidates.  The selector deliberately compares only rendered
/// rows: an internal search state is not a user-visible result until every
/// row has a direct-evidence medoid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CandidateQuality {
    row_count: usize,
    covered_pages: usize,
    max_row_width: usize,
    direct_pairs: usize,
    possible_pairs: usize,
    medoid_safe_rows: usize,
    block_count: usize,
    locally_offset_safe_blocks: usize,
}

struct DirectEvidenceIndex {
    similarities: BTreeMap<(usize, usize), f64>,
}

impl DirectEvidenceIndex {
    fn from_edges(edges: &[Edge]) -> Self {
        Self {
            similarities: edges
                .iter()
                .map(|edge| ((edge.left, edge.right), edge.evidence.visual_similarity))
                .collect(),
        }
    }

    fn similarity(&self, left: usize, right: usize) -> Option<f64> {
        let key = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        self.similarities.get(&key).copied()
    }
}

impl SceneBlock {
    fn sort_and_index_rows(&mut self) -> bool {
        self.rows
            .sort_by_key(|row| row.iter().next().copied().unwrap_or(usize::MAX));
        self.page_rows.clear();
        for (row_index, row) in self.rows.iter().enumerate() {
            for page in row {
                if self.page_rows.insert(*page, row_index).is_some() {
                    return false;
                }
            }
        }
        true
    }

    fn merge_rows(&mut self, left_row: usize, right_row: usize) -> bool {
        if left_row == right_row || left_row >= self.rows.len() || right_row >= self.rows.len() {
            return false;
        }
        let (keep, remove) = if left_row < right_row {
            (left_row, right_row)
        } else {
            (right_row, left_row)
        };
        let removed = self.rows.remove(remove);
        self.rows[keep].extend(removed);
        self.sort_and_index_rows()
    }
}

#[derive(Default, Clone)]
struct TrackAssignments {
    page_tracks: BTreeMap<usize, u32>,
    tracks: BTreeMap<u32, BTreeSet<usize>>,
    next_track: u32,
}

/// Hash features remain unchanged. Pair edges are aligned monotonically, then only runs that
/// share at least two ordered rows can attach a new edition track to a scene block.
#[allow(dead_code)]
pub(crate) fn detect_internal_groups(
    run_id: &str,
    artifact: &HashedArtifact,
    profile: &HashProfile,
) -> InternalDetection {
    detect_internal_groups_with_progress(run_id, artifact, profile, |_, _| {})
}

/// Runs the unchanged internal detector while reporting bounded pair-comparison
/// progress. The callback is observational only and is invoked at roughly 1%
/// intervals, plus the terminal comparison, so it cannot influence evidence or
/// threshold decisions.
pub(crate) fn detect_internal_groups_with_progress(
    run_id: &str,
    artifact: &HashedArtifact,
    profile: &HashProfile,
    mut on_pair_progress: impl FnMut(u64, u64),
) -> InternalDetection {
    let prepared = artifact
        .pages
        .iter()
        .cloned()
        .map(|original| PreparedInternalPage {
            detail_bytes: decode_detail_hash(&original.detail_d_hash_hex),
            original,
        })
        .collect::<Vec<_>>();
    let mut edges = Vec::new();
    let mut compared_pairs = 0_u64;
    let page_count = u64::try_from(prepared.len()).unwrap_or(u64::MAX);
    let total_pairs = page_count.saturating_mul(page_count.saturating_sub(1)) / 2;
    let report_interval = total_pairs.div_ceil(100).max(1);
    let mut next_report = report_interval;
    for left in 0..prepared.len() {
        for right in (left + 1)..prepared.len() {
            compared_pairs = compared_pairs.saturating_add(1);
            if let Some(evidence) = compare_prepared(&prepared[left], &prepared[right], profile) {
                // Blank/divider exact bytes are retained as standalone exact groups, never as
                // sequence bridges between editions.
                if !evidence.low_information {
                    edges.push(Edge {
                        left,
                        right,
                        evidence,
                    });
                }
            }
            if compared_pairs >= next_report || compared_pairs == total_pairs {
                on_pair_progress(compared_pairs, total_pairs);
                next_report = compared_pairs.saturating_add(report_interval);
            }
        }
    }
    // Keep the established DSU candidate independent from the structural
    // search.  The latter can repair a graph-wide over-union, but it must not
    // replace a bounded, directly-supported legacy edition layout merely
    // because it found a different monotonic decomposition.
    let structural_runs = monotonic_runs(&edges);
    let structural_blocks = merge_runs(&structural_runs, &edges, &prepared);
    let structural_groups = rows_to_groups(run_id, artifact, &prepared, &edges, structural_blocks);

    let legacy_runs = legacy_monotonic_runs(&edges);
    let legacy_blocks = legacy_structural_blocks(&legacy_runs, &edges, &prepared);
    let legacy_groups = rows_to_groups(run_id, artifact, &prepared, &edges, legacy_blocks);
    let mut groups = select_edition_candidate(legacy_groups, structural_groups, &prepared, &edges);

    let scene_pages = groups
        .iter()
        .flat_map(|record| record.group.pages.iter().map(|page| page.source_page))
        .collect::<BTreeSet<_>>();
    let mut exact_classes = BTreeMap::<&str, Vec<usize>>::new();
    for (index, page) in prepared.iter().enumerate() {
        exact_classes
            .entry(page.original.artifact_sha256.as_str())
            .or_default()
            .push(index);
    }
    let mut block_number = groups
        .iter()
        .map(|record| record.group.block_id.clone())
        .collect::<BTreeSet<_>>()
        .len() as u32;
    for indices in exact_classes.values().filter(|indices| indices.len() >= 2) {
        if indices
            .iter()
            .any(|index| scene_pages.contains(&prepared[*index].original.source_page_number.get()))
        {
            continue;
        }
        block_number = block_number.saturating_add(1);
        let pages = indices
            .iter()
            .map(|index| evidence_for_exact(&prepared[*index].original))
            .collect();
        groups.push(group_record(
            run_id,
            artifact,
            block_number,
            0,
            InternalMatchKind::Exact,
            1.0,
            pages,
        ));
    }
    groups.sort_by_key(|record| {
        (
            record.group.gallery_id,
            record.group.block_id.clone(),
            record.group.sequence_index,
        )
    });
    InternalDetection {
        groups,
        compared_pairs,
    }
}

fn monotonic_runs(edges: &[Edge]) -> Vec<PairRun> {
    let positions = edges
        .iter()
        .enumerate()
        .map(|(index, edge)| ((edge.left, edge.right), index))
        .collect::<BTreeMap<_, _>>();
    // Preserve the established longest-chain extraction as the primary path.
    // The bounded Pareto paths below supplement it only where an alignment
    // pays a real missing-scene gap; emitting every high-score prefix would
    // fragment ordinary complete two-way editions.
    let mut best = vec![(1_usize, None::<usize>, 0_usize); edges.len()];
    for (index, edge) in edges.iter().enumerate() {
        for left_gap in 1..=3 {
            for right_gap in 1..=3 {
                let Some(left) = edge.left.checked_sub(left_gap) else {
                    continue;
                };
                let Some(right) = edge.right.checked_sub(right_gap) else {
                    continue;
                };
                let Some(&previous) = positions.get(&(left, right)) else {
                    continue;
                };
                let (length, _, gap) = best[previous];
                let candidate = (length + 1, Some(previous), gap + left_gap + right_gap - 2);
                if candidate.0 > best[index].0
                    || (candidate.0 == best[index].0 && candidate.2 < best[index].2)
                {
                    best[index] = candidate;
                }
            }
        }
    }
    let predecessors = best
        .iter()
        .filter_map(|(_, previous, _)| *previous)
        .collect::<BTreeSet<_>>();
    let mut split_seen = BTreeSet::new();
    let mut runs = Vec::new();
    for terminal in 0..edges.len() {
        if predecessors.contains(&terminal) || best[terminal].0 < MIN_PAIR_RUN_ROWS {
            continue;
        }
        let mut chain = Vec::new();
        let mut cursor = Some(terminal);
        while let Some(index) = cursor {
            chain.push(index);
            cursor = best[index].1;
        }
        chain.reverse();
        for run in split_pair_run(chain, edges) {
            if run_has_local_edition_offset(&run, edges) && split_seen.insert(run.edges.clone()) {
                runs.push(run);
            }
        }
    }

    let mut paths = Vec::<Vec<MonotonicPath>>::with_capacity(edges.len());
    for edge in edges {
        let mut candidates = vec![MonotonicPath {
            predecessor: None,
            length: 1,
            similarity_sum: edge.evidence.visual_similarity,
            minimum_similarity: edge.evidence.visual_similarity,
            exact_count: usize::from(edge.evidence.exact_sha256),
            cumulative_gap: 0,
            offset_sum: edge.right.abs_diff(edge.left),
        }];
        for left_gap in 1..=3 {
            for right_gap in 1..=3 {
                let Some(left) = edge.left.checked_sub(left_gap) else {
                    continue;
                };
                let Some(right) = edge.right.checked_sub(right_gap) else {
                    continue;
                };
                let Some(&previous) = positions.get(&(left, right)) else {
                    continue;
                };
                for (previous_rank, path) in paths[previous].iter().enumerate() {
                    candidates.push(MonotonicPath {
                        predecessor: Some((previous, previous_rank)),
                        length: path.length + 1,
                        similarity_sum: path.similarity_sum + edge.evidence.visual_similarity,
                        minimum_similarity: path
                            .minimum_similarity
                            .min(edge.evidence.visual_similarity),
                        exact_count: path.exact_count + usize::from(edge.evidence.exact_sha256),
                        cumulative_gap: path.cumulative_gap + left_gap + right_gap - 2,
                        offset_sum: path.offset_sum + edge.right.abs_diff(edge.left),
                    });
                }
            }
        }
        paths.push(select_monotonic_paths(candidates));
    }
    for (terminal, terminal_paths) in paths.iter().enumerate() {
        for (path_rank, path) in terminal_paths.iter().enumerate() {
            if path.cumulative_gap == 0 {
                continue;
            }
            let chain = monotonic_path_edges(&paths, terminal, path_rank);
            if chain.len() < MIN_PAIR_RUN_ROWS {
                continue;
            }
            for run in split_pair_run(chain, edges) {
                if run_has_local_edition_offset(&run, edges) && split_seen.insert(run.edges.clone())
                {
                    runs.push(run);
                }
            }
        }
    }
    runs.sort_by(|left, right| {
        right
            .edges
            .len()
            .cmp(&left.edges.len())
            .then_with(|| right.average_similarity.total_cmp(&left.average_similarity))
            .then_with(|| {
                pair_run_minimum_similarity(right, edges)
                    .total_cmp(&pair_run_minimum_similarity(left, edges))
            })
            .then_with(|| right.exact_count.cmp(&left.exact_count))
            .then_with(|| left.cumulative_gap.cmp(&right.cumulative_gap))
            .then_with(|| left.edges.cmp(&right.edges))
    });
    // Prefix and suffix variants of a longer identical alignment otherwise
    // let a two-row high-similarity fragment outrank its complete edition
    // run. Keep bounded alternate paths, but not strict subruns of the same
    // direct-edge sequence.
    runs.truncate(MAX_STRUCTURAL_PAIR_RUNS.saturating_mul(2));
    runs = discard_strict_pair_subruns(runs);
    runs.truncate(MAX_STRUCTURAL_PAIR_RUNS);
    runs
}

fn discard_strict_pair_subruns(runs: Vec<PairRun>) -> Vec<PairRun> {
    runs.iter()
        .enumerate()
        .filter_map(|(index, run)| {
            let is_strict_subrun = runs.iter().enumerate().any(|(other_index, other)| {
                other_index != index
                    && other.edges.len() > run.edges.len()
                    && other
                        .edges
                        .windows(run.edges.len())
                        .any(|window| window == run.edges)
            });
            (!is_strict_subrun).then(|| run.clone())
        })
        .collect()
}

/// The pre-structural longest-chain extractor is intentionally retained as a
/// candidate source.  Do not feed it the Pareto/gapped alternatives above:
/// doing so would change the baseline we use to decide whether structural
/// reconstruction is actually warranted.
fn legacy_monotonic_runs(edges: &[Edge]) -> Vec<PairRun> {
    let positions = edges
        .iter()
        .enumerate()
        .map(|(index, edge)| ((edge.left, edge.right), index))
        .collect::<BTreeMap<_, _>>();
    let mut best = vec![(1_usize, None::<usize>, 0_usize); edges.len()];
    for (index, edge) in edges.iter().enumerate() {
        for left_gap in 1..=3 {
            for right_gap in 1..=3 {
                let Some(left) = edge.left.checked_sub(left_gap) else {
                    continue;
                };
                let Some(right) = edge.right.checked_sub(right_gap) else {
                    continue;
                };
                let Some(&previous) = positions.get(&(left, right)) else {
                    continue;
                };
                let (length, _, gap) = best[previous];
                let candidate = (length + 1, Some(previous), gap + left_gap + right_gap - 2);
                if candidate.0 > best[index].0
                    || (candidate.0 == best[index].0 && candidate.2 < best[index].2)
                {
                    best[index] = candidate;
                }
            }
        }
    }
    let predecessors = best
        .iter()
        .filter_map(|(_, previous, _)| *previous)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut runs = Vec::new();
    for terminal in 0..edges.len() {
        if predecessors.contains(&terminal) || best[terminal].0 < MIN_PAIR_RUN_ROWS {
            continue;
        }
        let mut chain = Vec::new();
        let mut cursor = Some(terminal);
        while let Some(index) = cursor {
            chain.push(index);
            cursor = best[index].1;
        }
        chain.reverse();
        if seen.insert(chain.clone()) {
            runs.extend(split_pair_run(chain, edges));
        }
    }
    runs.sort_by(|left, right| {
        right
            .edges
            .len()
            .cmp(&left.edges.len())
            .then_with(|| right.average_similarity.total_cmp(&left.average_similarity))
            .then_with(|| right.exact_count.cmp(&left.exact_count))
            .then_with(|| left.cumulative_gap.cmp(&right.cumulative_gap))
            .then_with(|| left.edges.cmp(&right.edges))
    });
    runs
}

fn select_monotonic_paths(candidates: Vec<MonotonicPath>) -> Vec<MonotonicPath> {
    let mut paths = Vec::new();
    for candidate_index in 0..candidates.len() {
        if candidates.iter().enumerate().any(|(other_index, other)| {
            other_index != candidate_index
                && monotonic_path_dominates(other, &candidates[candidate_index])
        }) {
            continue;
        }
        paths.push(candidates[candidate_index].clone());
    }
    paths.sort_by(compare_monotonic_paths);
    let lane_size = PAIR_RUN_PATHS_PER_EDGE / 2;
    let mut selected = paths
        .iter()
        .filter(|path| path.cumulative_gap == 0)
        .take(lane_size)
        .cloned()
        .collect::<Vec<_>>();
    selected.extend(
        paths
            .iter()
            .filter(|path| path.cumulative_gap > 0)
            .take(lane_size)
            .cloned(),
    );
    for path in paths {
        if selected.len() == PAIR_RUN_PATHS_PER_EDGE {
            break;
        }
        if !selected.contains(&path) {
            selected.push(path);
        }
    }
    selected
}

fn monotonic_path_dominates(left: &MonotonicPath, right: &MonotonicPath) -> bool {
    let left_average = left.similarity_sum / left.length as f64;
    let right_average = right.similarity_sum / right.length as f64;
    left.length >= right.length
        && left_average >= right_average
        && left.minimum_similarity >= right.minimum_similarity
        && left.exact_count >= right.exact_count
        && left.cumulative_gap <= right.cumulative_gap
        && (left.length > right.length
            || left_average > right_average
            || left.minimum_similarity > right.minimum_similarity
            || left.exact_count > right.exact_count
            || left.cumulative_gap < right.cumulative_gap)
}

fn compare_monotonic_paths(left: &MonotonicPath, right: &MonotonicPath) -> std::cmp::Ordering {
    let left_average = left.similarity_sum / left.length as f64;
    let right_average = right.similarity_sum / right.length as f64;
    right_average
        .total_cmp(&left_average)
        .then_with(|| right.minimum_similarity.total_cmp(&left.minimum_similarity))
        .then_with(|| right.exact_count.cmp(&left.exact_count))
        .then_with(|| right.length.cmp(&left.length))
        .then_with(|| left.cumulative_gap.cmp(&right.cumulative_gap))
        .then_with(|| left.predecessor.cmp(&right.predecessor))
}

fn monotonic_path_edges(
    paths: &[Vec<MonotonicPath>],
    terminal: usize,
    path_rank: usize,
) -> Vec<usize> {
    let mut chain = Vec::new();
    let mut cursor = Some((terminal, path_rank));
    while let Some((edge, rank)) = cursor {
        chain.push(edge);
        cursor = paths[edge][rank].predecessor;
    }
    chain.reverse();
    chain
}

fn pair_run_minimum_similarity(run: &PairRun, edges: &[Edge]) -> f64 {
    run.edges
        .iter()
        .map(|index| edges[*index].evidence.visual_similarity)
        .min_by(f64::total_cmp)
        .unwrap_or(0.0)
}

/// A raw monotonic chain may pass through a page as the right endpoint of one
/// edition relation and the left endpoint of the next (A↔B followed by B↔C).
/// Those are separate pairwise tracks, not one A/B/C endpoint sequence.
fn split_pair_run(chain: Vec<usize>, edges: &[Edge]) -> Vec<PairRun> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut left_seen = BTreeSet::new();
    let mut right_seen = BTreeSet::new();
    for edge_index in chain {
        let edge = &edges[edge_index];
        if !current.is_empty()
            && (right_seen.contains(&edge.left) || left_seen.contains(&edge.right))
        {
            segments.push(pair_run(std::mem::take(&mut current), edges));
            left_seen.clear();
            right_seen.clear();
        }
        left_seen.insert(edge.left);
        right_seen.insert(edge.right);
        current.push(edge_index);
    }
    if !current.is_empty() {
        segments.push(pair_run(current, edges));
    }
    segments
        .into_iter()
        .filter(|run| run.edges.len() >= 2)
        .collect()
}

fn pair_run(chain: Vec<usize>, edges: &[Edge]) -> PairRun {
    let exact_count = chain
        .iter()
        .filter(|&&index| edges[index].evidence.exact_sha256)
        .count();
    let average_similarity = chain
        .iter()
        .map(|&index| edges[index].evidence.visual_similarity)
        .sum::<f64>()
        / chain.len() as f64;
    let cumulative_gap = chain
        .windows(2)
        .map(|pair| {
            let previous = &edges[pair[0]];
            let next = &edges[pair[1]];
            next.left.saturating_sub(previous.left + 1)
                + next.right.saturating_sub(previous.right + 1)
        })
        .sum();
    PairRun {
        edges: chain,
        average_similarity,
        exact_count,
        cumulative_gap,
    }
}

/// Reconstruct the original graph-wide DSU result without borrowing any of
/// the structural candidate's alternate paths or local merge constraints.
fn legacy_merge_runs(runs: &[PairRun], edges: &[Edge]) -> Vec<SceneBlock> {
    let page_count = edges
        .iter()
        .flat_map(|edge| [edge.left, edge.right])
        .max()
        .map_or(0, |index| index + 1);
    let mut parent = (0..page_count).collect::<Vec<_>>();
    let mut qualifying = BTreeSet::new();
    for run in runs {
        if run.edges.len() >= MIN_PAIR_RUN_ROWS && legacy_run_has_local_edition_offset(run, edges) {
            qualifying.extend(run.edges.iter().copied());
        }
    }
    for edge_index in qualifying {
        let edge = &edges[edge_index];
        union(&mut parent, edge.left, edge.right);
    }
    let mut components = BTreeMap::<usize, BTreeSet<usize>>::new();
    for page in 0..page_count {
        let root = find(&mut parent, page);
        components.entry(root).or_default().insert(page);
    }
    let mut rows = components
        .into_values()
        .filter(|row| row.len() >= 2)
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.iter().next().copied().unwrap_or(usize::MAX));
    let mut blocks = Vec::<SceneBlock>::new();
    for row in rows {
        if let Some(block) = blocks.last_mut() {
            if block
                .rows
                .last()
                .is_some_and(|previous| rows_follow(previous, &row))
            {
                let row_index = block.rows.len();
                for page in &row {
                    block.page_rows.insert(*page, row_index);
                }
                block.rows.push(row);
                continue;
            }
        }
        let page_rows = row.iter().map(|page| (*page, 0)).collect();
        blocks.push(SceneBlock {
            rows: vec![row],
            page_rows,
        });
    }
    blocks.retain(|block| block.rows.len() >= MIN_PAIR_RUN_ROWS);
    blocks
}

/// This is the v3 offset check.  Its absence of a lower bound is part of the
/// retained baseline; the structural candidate keeps its stricter check.
fn legacy_run_has_local_edition_offset(run: &PairRun, edges: &[Edge]) -> bool {
    let offset_sum = run
        .edges
        .iter()
        .map(|index| edges[*index].right.abs_diff(edges[*index].left))
        .sum::<usize>();
    let average_offset = offset_sum / run.edges.len();
    average_offset <= run.edges.len().saturating_mul(3).saturating_add(3)
}

fn rows_follow(previous: &BTreeSet<usize>, next: &BTreeSet<usize>) -> bool {
    previous
        .iter()
        .flat_map(|left| next.iter().map(move |right| (*left, *right)))
        .filter(|(left, right)| *right > *left && *right - *left <= 3)
        .count()
        >= MIN_PAIR_RUN_ROWS
}

fn find(parent: &mut [usize], index: usize) -> usize {
    if parent[index] != index {
        let root = find(parent, parent[index]);
        parent[index] = root;
    }
    parent[index]
}

fn union(parent: &mut [usize], left: usize, right: usize) {
    let left_root = find(parent, left);
    let right_root = find(parent, right);
    if left_root != right_root {
        parent[right_root] = left_root;
    }
}

/// Adopt complete, score-ordered pair runs only when every page can retain a
/// single row and a single monotonic edition track.  This deliberately avoids
/// a graph-wide DSU: a locally plausible edge must not bridge two otherwise
/// independent scene/edition structures.
fn merge_runs(
    runs: &[PairRun],
    edges: &[Edge],
    prepared: &[PreparedInternalPage],
) -> Vec<StructuralBlock> {
    let evidence_index = DirectEvidenceIndex::from_edges(edges);
    let mut states = vec![StructuralState::default()];
    for run in runs {
        if run.edges.len() < MIN_PAIR_RUN_ROWS || !run_has_local_edition_offset(run, edges) {
            continue;
        }
        let mut next = Vec::with_capacity(states.len().saturating_mul(2));
        for state in states {
            let mut accepted = state.clone();
            if try_adopt_run(
                run,
                edges,
                prepared,
                &mut accepted.blocks,
                &mut accepted.page_blocks,
            ) {
                accepted.accepted_alignment_gap = accepted
                    .accepted_alignment_gap
                    .saturating_add(run.cumulative_gap);
                accepted.accepted_edges.extend(run.edges.iter().copied());
                accepted.quality = structural_quality(&accepted.blocks, &evidence_index, prepared);
                next.push(accepted);
            }
            next.push(state);
        }
        next.sort_by(|left, right| compare_structural_states(left, right, edges, &evidence_index));
        let mut seen = BTreeSet::new();
        next.retain(|state| seen.insert(structural_state_signature(state)));
        // Keep the best state for each coverage frontier.  Otherwise a set
        // of early stitched runs can occupy the entire fixed beam merely by
        // claiming pages before the smaller atomic runs have been processed.
        let mut coverage_frontiers = BTreeMap::<usize, usize>::new();
        next.retain(|state| {
            let retained = coverage_frontiers
                .entry(state.page_blocks.len())
                .or_default();
            if *retained >= STRUCTURAL_STATES_PER_COVERAGE {
                return false;
            }
            *retained += 1;
            true
        });
        next.truncate(STRUCTURAL_BEAM_WIDTH);
        states = next;
    }
    let mut blocks = states
        .into_iter()
        .min_by(|left, right| compare_structural_states(left, right, edges, &evidence_index))
        .map(|state| state.blocks)
        .unwrap_or_default();

    blocks = blocks
        .into_iter()
        .filter_map(|mut block| {
            (prune_weak_structural_tracks(&mut block)
                && block.scene.rows.len() >= MIN_PAIR_RUN_ROWS
                && block
                    .scene
                    .rows
                    .iter()
                    .all(|row| row.len() >= MIN_PAIR_RUN_ROWS)
                && block.tracks.is_consistent(&block.scene.page_rows, prepared))
            .then_some(block)
        })
        .collect();
    blocks.sort_by_key(|block| {
        block
            .scene
            .rows
            .first()
            .and_then(|row| row.iter().next().copied())
            .unwrap_or(usize::MAX)
    });
    for block in &mut blocks {
        block.tracks = std::mem::take(&mut block.tracks).canonicalize(prepared);
    }

    // Reuse album track scopes only across an ordered, directly-supported
    // boundary.  This allows a later scene block to lose a row or a relation
    // without requiring an exact graph-shape match, and keeps row/block
    // identity separate from the track scope.
    for block_index in 0..blocks.len() {
        let continuation = (0..block_index).rev().find(|previous| {
            blocks_have_ordered_track_continuity(&blocks[*previous], &blocks[block_index], prepared)
        });
        let scopes = continuation.map_or_else(
            || own_track_scopes(&blocks[block_index], block_index),
            |previous| inherited_track_scopes(&blocks[previous], &blocks[block_index]),
        );
        blocks[block_index].track_scopes = scopes;
    }
    blocks
}

/// A locally coherent tail may contain one well-supported edition plus a few
/// weak fragments.  Retain the supported tracks instead of throwing away the
/// complete block: two rows require both pages per track, while longer blocks
/// require 75% coverage.  After pruning, orphaned cells and rows are removed
/// and all track indexes/constraints are rebuilt deterministically.
fn prune_weak_structural_tracks(block: &mut StructuralBlock) -> bool {
    loop {
        let minimum_track_rows = structural_minimum_track_rows(block.scene.rows.len());
        let retained_tracks = block
            .tracks
            .tracks
            .iter()
            .filter_map(|(track, pages)| (pages.len() >= minimum_track_rows).then_some(*track))
            .collect::<BTreeSet<_>>();
        if retained_tracks.len() < MIN_PAIR_RUN_ROWS {
            return false;
        }

        let prior_row_count = block.scene.rows.len();
        block.scene.rows.iter_mut().for_each(|row| {
            row.retain(|page| {
                block
                    .tracks
                    .page_tracks
                    .get(page)
                    .is_some_and(|track| retained_tracks.contains(track))
            });
        });
        block
            .scene
            .rows
            .retain(|row| row.len() >= MIN_PAIR_RUN_ROWS);
        if block.scene.rows.len() < MIN_PAIR_RUN_ROWS || !block.scene.sort_and_index_rows() {
            return false;
        }

        let retained_pages = block
            .scene
            .page_rows
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        block.tracks.tracks.retain(|track, pages| {
            pages.retain(|page| retained_pages.contains(page));
            retained_tracks.contains(track) && !pages.is_empty()
        });
        block.tracks.page_tracks = block
            .tracks
            .tracks
            .iter()
            .flat_map(|(track, pages)| pages.iter().map(|page| (*page, *track)))
            .collect();
        block.tracks.next_track = block.tracks.tracks.len() as u32;

        if prior_row_count == block.scene.rows.len()
            && block
                .tracks
                .tracks
                .values()
                .all(|pages| pages.len() >= structural_minimum_track_rows(block.scene.rows.len()))
        {
            rebuild_pruned_pair_constraints(block);
            return block.tracks.tracks.len() >= MIN_PAIR_RUN_ROWS;
        }
    }
}

fn structural_minimum_track_rows(rows: usize) -> usize {
    if rows <= MIN_PAIR_RUN_ROWS {
        MIN_PAIR_RUN_ROWS
    } else {
        rows.saturating_mul(3).saturating_add(3) / 4
    }
}

fn rebuild_pruned_pair_constraints(block: &mut StructuralBlock) {
    let retained_pages = block
        .scene
        .page_rows
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    block.pair_constraints = std::mem::take(&mut block.pair_constraints)
        .into_iter()
        .filter_map(|(mut left, mut right)| {
            left.retain(|page| retained_pages.contains(page));
            right.retain(|page| retained_pages.contains(page));
            let left_track = block.tracks.track_for_pages(&left)?;
            let right_track = block.tracks.track_for_pages(&right)?;
            (left.len() >= MIN_PAIR_RUN_ROWS
                && right.len() >= MIN_PAIR_RUN_ROWS
                && left_track != right_track)
                .then_some((left, right))
        })
        .collect();
}

fn blocks_have_ordered_track_continuity(
    previous: &StructuralBlock,
    next: &StructuralBlock,
    prepared: &[PreparedInternalPage],
) -> bool {
    previous.tracks.tracks.len() >= 2
        && next.tracks.tracks.len() >= 2
        && has_connected_track_runs(previous)
        && has_connected_track_runs(next)
        && track_range_end(previous, prepared) < track_range_start(next, prepared)
}

fn has_connected_track_runs(block: &StructuralBlock) -> bool {
    let mut linked = BTreeSet::new();
    for (left, right) in &block.pair_constraints {
        let Some(left_track) = block.tracks.track_for_pages(left) else {
            return false;
        };
        let Some(right_track) = block.tracks.track_for_pages(right) else {
            return false;
        };
        if left_track == right_track {
            return false;
        }
        linked.extend([left_track, right_track]);
    }
    linked.len() == block.tracks.tracks.len()
}

fn track_range_start(block: &StructuralBlock, prepared: &[PreparedInternalPage]) -> u32 {
    block
        .tracks
        .tracks
        .values()
        .flat_map(|pages| pages.iter())
        .map(|page| prepared[*page].original.source_page_number.get())
        .min()
        .unwrap_or(u32::MAX)
}

fn track_range_end(block: &StructuralBlock, prepared: &[PreparedInternalPage]) -> u32 {
    block
        .tracks
        .tracks
        .values()
        .flat_map(|pages| pages.iter())
        .map(|page| prepared[*page].original.source_page_number.get())
        .max()
        .unwrap_or(0)
}

fn own_track_scopes(block: &StructuralBlock, block_index: usize) -> BTreeMap<u32, (usize, u32)> {
    block
        .tracks
        .tracks
        .keys()
        .copied()
        .map(|ordinal| (ordinal, (block_index, ordinal)))
        .collect()
}

fn inherited_track_scopes(
    previous: &StructuralBlock,
    next: &StructuralBlock,
) -> BTreeMap<u32, (usize, u32)> {
    let previous_tracks = previous.tracks.tracks.keys().copied().collect::<Vec<_>>();
    let next_tracks = next.tracks.tracks.keys().copied().collect::<Vec<_>>();
    next_tracks
        .iter()
        .enumerate()
        .map(|(next_index, next_track)| {
            let previous_index = next_index.saturating_mul(previous_tracks.len().saturating_sub(1))
                / next_tracks.len().saturating_sub(1).max(1);
            let previous_track = previous_tracks[previous_index];
            let scope = previous.track_scopes[&previous_track];
            (*next_track, scope)
        })
        .collect()
}

fn compare_structural_states(
    left: &StructuralState,
    right: &StructuralState,
    edges: &[Edge],
    evidence_index: &DirectEvidenceIndex,
) -> std::cmp::Ordering {
    let left_quality = structural_quality_for_state(left, evidence_index);
    let right_quality = structural_quality_for_state(right, evidence_index);
    // A state may only make a row wider when direct evidence remains dense.
    // Multiway closure is a binary signal here: its raw count rewards a
    // spurious wide row more than a correctly separated edition.  A material
    // weak direct link can break a tie between two complete multiway states,
    // but ordinary score noise must defer to coverage.
    compare_direct_support_density(right_quality, left_quality)
        .then_with(|| {
            left_quality
                .track_fragment_penalty
                .cmp(&right_quality.track_fragment_penalty)
        })
        .then_with(|| {
            (right_quality.triangle_surplus > 0).cmp(&(left_quality.triangle_surplus > 0))
        })
        .then_with(|| compare_multiway_direct_evidence(right_quality, left_quality))
        .then_with(|| right.page_blocks.len().cmp(&left.page_blocks.len()))
        .then_with(|| right.accepted_edges.len().cmp(&left.accepted_edges.len()))
        .then_with(|| {
            accepted_visual_similarity_sum(right, edges)
                .total_cmp(&accepted_visual_similarity_sum(left, edges))
        })
        .then_with(|| {
            direct_support_average(right_quality).total_cmp(&direct_support_average(left_quality))
        })
        .then_with(|| {
            right_quality
                .minimum_direct_similarity
                .total_cmp(&left_quality.minimum_direct_similarity)
        })
        .then_with(|| {
            accepted_exact_edge_count(right, edges).cmp(&accepted_exact_edge_count(left, edges))
        })
        .then_with(|| left_quality.track_gap.cmp(&right_quality.track_gap))
        .then_with(|| {
            left.accepted_alignment_gap
                .cmp(&right.accepted_alignment_gap)
        })
        .then_with(|| left_quality.track_count.cmp(&right_quality.track_count))
        .then_with(|| structural_fragment_count(right).cmp(&structural_fragment_count(left)))
        .then_with(|| structural_block_count(left).cmp(&structural_block_count(right)))
        .then_with(|| structural_state_signature(left).cmp(&structural_state_signature(right)))
}

fn structural_quality_for_state(
    state: &StructuralState,
    _evidence_index: &DirectEvidenceIndex,
) -> StructuralQuality {
    state.quality
}

fn compare_direct_support_density(
    left: StructuralQuality,
    right: StructuralQuality,
) -> std::cmp::Ordering {
    match (left.possible_pairs, right.possible_pairs) {
        (0, 0) => std::cmp::Ordering::Equal,
        (0, _) => std::cmp::Ordering::Less,
        (_, 0) => std::cmp::Ordering::Greater,
        _ => (left.direct_pairs.saturating_mul(right.possible_pairs))
            .cmp(&right.direct_pairs.saturating_mul(left.possible_pairs)),
    }
}

fn compare_multiway_direct_evidence(
    left: StructuralQuality,
    right: StructuralQuality,
) -> std::cmp::Ordering {
    if left.triangle_surplus == 0 || right.triangle_surplus == 0 {
        return std::cmp::Ordering::Equal;
    }
    let difference = left.minimum_direct_similarity - right.minimum_direct_similarity;
    if difference.abs() <= MULTIWAY_QUALITY_TIE_EPSILON {
        std::cmp::Ordering::Equal
    } else {
        left.minimum_direct_similarity
            .total_cmp(&right.minimum_direct_similarity)
    }
}

fn direct_support_average(quality: StructuralQuality) -> f64 {
    if quality.direct_pairs > 0 {
        quality.direct_similarity_sum / quality.direct_pairs as f64
    } else {
        0.0
    }
}

fn structural_quality(
    blocks: &[StructuralBlock],
    evidence_index: &DirectEvidenceIndex,
    prepared: &[PreparedInternalPage],
) -> StructuralQuality {
    let mut quality = StructuralQuality::default();
    for block in blocks.iter().filter(|block| !block.scene.rows.is_empty()) {
        for row in &block.scene.rows {
            let pages = row.iter().copied().collect::<Vec<_>>();
            let required_pairs = pages.len().saturating_mul(pages.len().saturating_sub(1)) / 2;
            quality.possible_pairs = quality.possible_pairs.saturating_add(required_pairs);
            let direct_before = quality.direct_pairs;
            for left in 0..pages.len() {
                for right in left + 1..pages.len() {
                    if let Some(similarity) = evidence_index.similarity(pages[left], pages[right]) {
                        quality.direct_pairs = quality.direct_pairs.saturating_add(1);
                        quality.direct_similarity_sum += similarity;
                        quality.minimum_direct_similarity =
                            if direct_before == 0 && quality.direct_pairs == 1 {
                                similarity
                            } else {
                                quality.minimum_direct_similarity.min(similarity)
                            };
                    }
                }
            }
            let direct_in_row = quality.direct_pairs.saturating_sub(direct_before);
            quality.triangle_surplus = quality
                .triangle_surplus
                .saturating_add(direct_in_row.saturating_sub(pages.len().saturating_sub(1)));
        }
        quality.track_count = quality.track_count.max(block.tracks.tracks.len());
        for track in block.tracks.tracks.values() {
            quality.track_fragment_penalty = quality
                .track_fragment_penalty
                .saturating_add(3_usize.saturating_sub(track.len()));
            let mut pages = track
                .iter()
                .filter_map(|page| {
                    block
                        .scene
                        .page_rows
                        .get(page)
                        .map(|row| (*row, prepared[*page].original.source_page_number.get()))
                })
                .collect::<Vec<_>>();
            pages.sort();
            for pair in pages.windows(2) {
                quality.track_gap = quality
                    .track_gap
                    .saturating_add(pair[1].1.saturating_sub(pair[0].1.saturating_add(1)) as usize);
            }
        }
    }
    quality
}

fn accepted_visual_similarity_sum(state: &StructuralState, edges: &[Edge]) -> f64 {
    state
        .accepted_edges
        .iter()
        .map(|index| edges[*index].evidence.visual_similarity)
        .sum()
}

fn accepted_exact_edge_count(state: &StructuralState, edges: &[Edge]) -> usize {
    state
        .accepted_edges
        .iter()
        .filter(|index| edges[**index].evidence.exact_sha256)
        .count()
}

fn structural_fragment_count(state: &StructuralState) -> usize {
    state
        .blocks
        .iter()
        .map(|block| block.scene.rows.len())
        .sum()
}

fn structural_block_count(state: &StructuralState) -> usize {
    state
        .blocks
        .iter()
        .filter(|block| !block.scene.rows.is_empty())
        .count()
}

fn structural_state_signature(state: &StructuralState) -> StructuralStateSignature {
    let blocks = state
        .blocks
        .iter()
        .filter(|block| !block.scene.rows.is_empty())
        .map(|block| {
            (
                block
                    .scene
                    .rows
                    .iter()
                    .map(|row| row.iter().copied().collect())
                    .collect(),
                block
                    .tracks
                    .page_tracks
                    .iter()
                    .map(|(page, track)| (*page, *track))
                    .collect(),
            )
        })
        .collect();
    (state.accepted_edges.iter().copied().collect(), blocks)
}

fn try_adopt_run(
    run: &PairRun,
    edges: &[Edge],
    prepared: &[PreparedInternalPage],
    blocks: &mut Vec<StructuralBlock>,
    page_blocks: &mut BTreeMap<usize, usize>,
) -> bool {
    let pairs = run
        .edges
        .iter()
        .map(|&edge_index| {
            let edge = &edges[edge_index];
            (edge.left, edge.right)
        })
        .collect::<Vec<_>>();
    let pages = pairs
        .iter()
        .flat_map(|(left, right)| [*left, *right])
        .collect::<BTreeSet<_>>();
    if pairs.len() < MIN_PAIR_RUN_ROWS || pages.len() != pairs.len().saturating_mul(2) {
        return false;
    }
    let occupied_blocks = pages
        .iter()
        .filter_map(|page| page_blocks.get(page).copied())
        .collect::<BTreeSet<_>>();
    let existing_block = occupied_blocks.first().copied();
    let mut candidate = match combine_blocks(&occupied_blocks, blocks) {
        Some(candidate) => candidate,
        None => return false,
    };

    for (left, right) in &pairs {
        match (
            candidate.scene.page_rows.get(left).copied(),
            candidate.scene.page_rows.get(right).copied(),
        ) {
            (Some(left_row), Some(right_row)) if left_row != right_row => {
                // A candidate can join independently-established blocks, but
                // it may never collapse two rows already claimed by the same
                // block.  That is the boundary that prevents a locally
                // similar scene from becoming a transitive bridge.
                if page_blocks.get(left) == page_blocks.get(right)
                    || !candidate.scene.merge_rows(left_row, right_row)
                {
                    return false;
                }
            }
            (Some(_), Some(_)) => {}
            (Some(row), None) => {
                candidate.scene.rows[row].insert(*right);
            }
            (None, Some(row)) => {
                candidate.scene.rows[row].insert(*left);
            }
            (None, None) => {
                candidate.scene.rows.push(BTreeSet::from([*left, *right]));
            }
        }
    }
    if !candidate.scene.sort_and_index_rows() {
        return false;
    }

    let (left, right): (Vec<_>, Vec<_>) = pairs.iter().copied().unzip();
    if !candidate
        .tracks
        .add_pair_constraint(&left, &right, &candidate.scene.page_rows, prepared)
        || !candidate
            .tracks
            .is_consistent(&candidate.scene.page_rows, prepared)
    {
        return false;
    }
    let block_index = existing_block.unwrap_or_else(|| {
        blocks.push(StructuralBlock::default());
        blocks.len() - 1
    });
    blocks[block_index] = candidate;
    for merged_block in occupied_blocks {
        if merged_block != block_index {
            blocks[merged_block] = StructuralBlock::default();
        }
    }
    for page in blocks[block_index].scene.page_rows.keys() {
        page_blocks.insert(*page, block_index);
    }
    blocks[block_index].pair_constraints.push((left, right));
    true
}

fn combine_blocks(
    block_indices: &BTreeSet<usize>,
    blocks: &[StructuralBlock],
) -> Option<StructuralBlock> {
    let mut combined = StructuralBlock::default();
    for block_index in block_indices {
        let block = blocks.get(*block_index)?;
        if block.scene.rows.is_empty() {
            return None;
        }
        combined.scene.rows.extend(block.scene.rows.iter().cloned());
        combined
            .pair_constraints
            .extend(block.pair_constraints.iter().cloned());
        combined.tracks.append_namespace(&block.tracks);
    }
    if !combined.scene.rows.is_empty() && !combined.scene.sort_and_index_rows() {
        return None;
    }
    Some(combined)
}

/// A repeated cycle can produce a perfectly monotonic edge chain to the same
/// position in the next cycle.  Those long jumps are not another edition of
/// the current scene sequence.  Keep the bounded edition offsets that can be
/// supported by the run itself (including a small missing-page allowance),
/// while leaving the separate cycle as its own block.
fn run_has_local_edition_offset(run: &PairRun, edges: &[Edge]) -> bool {
    let offset_sum = run
        .edges
        .iter()
        .map(|index| edges[*index].right.abs_diff(edges[*index].left))
        .sum::<usize>();
    let average_offset = offset_sum / run.edges.len();
    // A run made solely from a page and its immediate neighbours is normally
    // a same-track scene progression, not another edition.  Real editions
    // occupy an ordered span at least as long as the aligned run.
    average_offset >= run.edges.len()
        && average_offset <= run.edges.len().saturating_mul(3).saturating_add(3)
}

fn legacy_structural_blocks(
    runs: &[PairRun],
    edges: &[Edge],
    prepared: &[PreparedInternalPage],
) -> Vec<StructuralBlock> {
    let mut blocks = legacy_merge_runs(runs, edges)
        .into_iter()
        .map(|scene| {
            let tracks = restore_legacy_edition_tracks(&scene, runs, edges, prepared);
            StructuralBlock {
                scene,
                tracks,
                pair_constraints: Vec::new(),
                track_scopes: BTreeMap::new(),
            }
        })
        .collect::<Vec<_>>();
    for block_index in 0..blocks.len() {
        let continuation = (0..block_index).rev().find(|previous| {
            legacy_blocks_have_ordered_track_continuity(
                &blocks[*previous],
                &blocks[block_index],
                prepared,
            )
        });
        blocks[block_index].track_scopes = continuation.map_or_else(
            || own_track_scopes(&blocks[block_index], block_index),
            |previous| inherited_track_scopes(&blocks[previous], &blocks[block_index]),
        );
    }
    blocks
}

fn legacy_blocks_have_ordered_track_continuity(
    previous: &StructuralBlock,
    next: &StructuralBlock,
    prepared: &[PreparedInternalPage],
) -> bool {
    previous.tracks.tracks.len() >= MIN_PAIR_RUN_ROWS
        && next.tracks.tracks.len() >= MIN_PAIR_RUN_ROWS
        && previous
            .tracks
            .tracks
            .values()
            .chain(next.tracks.tracks.values())
            .all(|track| track.len() >= MIN_PAIR_RUN_ROWS)
        && track_range_end(previous, prepared) < track_range_start(next, prepared)
}

/// Restore v3 tracks after the DSU has formed its components.  Keeping this
/// separate from `add_pair_constraint` is intentional: this candidate must
/// retain the exact baseline behavior, including its later output filtering.
fn restore_legacy_edition_tracks(
    block: &SceneBlock,
    runs: &[PairRun],
    edges: &[Edge],
    prepared: &[PreparedInternalPage],
) -> TrackAssignments {
    let mut assignments = TrackAssignments::default();
    for run in runs {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for &edge_index in &run.edges {
            let edge = &edges[edge_index];
            if block.page_rows.get(&edge.left) == block.page_rows.get(&edge.right)
                && block.page_rows.contains_key(&edge.left)
                && block.page_rows.contains_key(&edge.right)
            {
                left.push(edge.left);
                right.push(edge.right);
            }
        }
        if left.len() < MIN_PAIR_RUN_ROWS
            || right.len() < MIN_PAIR_RUN_ROWS
            || left.iter().collect::<BTreeSet<_>>().len() != left.len()
            || right.iter().collect::<BTreeSet<_>>().len() != right.len()
        {
            continue;
        }
        let left_tracks = left
            .iter()
            .filter_map(|page| assignments.page_tracks.get(page))
            .collect::<BTreeSet<_>>();
        let right_tracks = right
            .iter()
            .filter_map(|page| assignments.page_tracks.get(page))
            .collect::<BTreeSet<_>>();
        if !left_tracks.is_disjoint(&right_tracks) {
            continue;
        }
        let mut candidate = assignments.clone();
        if candidate.add_constraint(&left, &block.page_rows, prepared)
            && candidate.add_constraint(&right, &block.page_rows, prepared)
        {
            assignments = candidate;
        }
    }
    assignments.canonicalize(prepared)
}

fn rows_to_groups(
    run_id: &str,
    artifact: &HashedArtifact,
    prepared: &[PreparedInternalPage],
    edges: &[Edge],
    blocks: Vec<StructuralBlock>,
) -> Vec<InternalGroupRecord> {
    let mut records = Vec::new();
    for (block_index, block) in blocks.into_iter().enumerate() {
        let assignments = block.tracks;
        let mut output_rows = Vec::new();
        for row in &block.scene.rows {
            let mut indexes = row
                .iter()
                .copied()
                .filter(|index| assignments.page_tracks.contains_key(index))
                .collect::<Vec<_>>();
            indexes.sort_by_key(|index| prepared[*index].original.source_page_number);
            if indexes.len() < 2 {
                continue;
            }
            let Some(representative) = choose_row_representative(&indexes, prepared, edges) else {
                // A transitive component without one direct-evidence medoid is
                // not safe to render as a single scene row.
                continue;
            };
            output_rows.push((indexes, representative));
        }
        if output_rows.len() < 2 {
            continue;
        }
        let block_number = block_index as u32 + 1;
        for (sequence, (indexes, representative)) in output_rows.into_iter().enumerate() {
            let pages = indexes
                .iter()
                .filter_map(|&index| row_evidence(representative, index, prepared, edges))
                .collect::<Vec<_>>();
            if pages.len() != indexes.len() {
                continue;
            }
            let relation = if pages.iter().all(|page| page.exact_sha256) {
                InternalMatchKind::Exact
            } else {
                InternalMatchKind::TranslationVisual
            };
            let confidence = pages
                .iter()
                .map(|page| page.visual_similarity)
                .fold(1.0_f64, f64::min);
            let mut record = group_record(
                run_id,
                artifact,
                block_number,
                sequence as u32,
                relation,
                confidence,
                pages,
            );
            for page in &mut record.group.pages {
                let index = indexes
                    .iter()
                    .copied()
                    .find(|index| {
                        prepared[*index].original.source_page_number.get() == page.source_page
                    })
                    .expect("group page is prepared");
                let local_ordinal = assignments.page_tracks[&index];
                let (scope_block, ordinal) = block.track_scopes[&local_ordinal];
                page.edition_track_ordinal = Some(ordinal);
                page.edition_track_id = Some(format!(
                    "{}-t{ordinal}",
                    block_identifier(artifact, scope_block as u32 + 1)
                ));
            }
            records.push(record);
        }
    }
    records
}

fn select_edition_candidate(
    legacy: Vec<InternalGroupRecord>,
    structural: Vec<InternalGroupRecord>,
    prepared: &[PreparedInternalPage],
    edges: &[Edge],
) -> Vec<InternalGroupRecord> {
    let legacy_quality = candidate_quality(&legacy, prepared, edges);
    let structural_quality = candidate_quality(&structural, prepared, edges);
    if should_select_structural_candidate(legacy_quality, structural_quality) {
        structural
    } else if candidate_has_valid_edition_block(legacy_quality) {
        legacy
    } else {
        Vec::new()
    }
}

fn candidate_quality(
    groups: &[InternalGroupRecord],
    prepared: &[PreparedInternalPage],
    edges: &[Edge],
) -> CandidateQuality {
    let page_indices = prepared
        .iter()
        .enumerate()
        .map(|(index, page)| (page.original.source_page_number.get(), index))
        .collect::<BTreeMap<_, _>>();
    let evidence = DirectEvidenceIndex::from_edges(edges);
    let mut quality = CandidateQuality::default();
    let mut covered_pages = BTreeSet::new();
    let mut block_offsets = BTreeMap::<String, (usize, usize, usize)>::new();
    for record in groups {
        let indexes = record
            .group
            .pages
            .iter()
            .filter_map(|page| page_indices.get(&page.source_page).copied())
            .collect::<Vec<_>>();
        if indexes.len() != record.group.pages.len() || indexes.len() < 2 {
            continue;
        }
        quality.row_count = quality.row_count.saturating_add(1);
        quality.max_row_width = quality.max_row_width.max(indexes.len());
        quality.possible_pairs = quality.possible_pairs.saturating_add(
            indexes
                .len()
                .saturating_mul(indexes.len().saturating_sub(1))
                / 2,
        );
        covered_pages.extend(indexes.iter().copied());
        let block = block_offsets
            .entry(record.group.block_id.clone())
            .or_default();
        block.0 = block.0.saturating_add(1);
        for left in 0..indexes.len() {
            for right in left + 1..indexes.len() {
                block.1 = block
                    .1
                    .saturating_add(indexes[left].abs_diff(indexes[right]));
                block.2 = block.2.saturating_add(1);
                if evidence.similarity(indexes[left], indexes[right]).is_some() {
                    quality.direct_pairs = quality.direct_pairs.saturating_add(1);
                }
            }
        }
        if indexes.iter().copied().any(|candidate| {
            indexes
                .iter()
                .copied()
                .filter(|index| *index != candidate)
                .all(|index| evidence.similarity(candidate, index).is_some())
        }) {
            quality.medoid_safe_rows = quality.medoid_safe_rows.saturating_add(1);
        }
    }
    quality.covered_pages = covered_pages.len();
    quality.block_count = block_offsets.len();
    quality.locally_offset_safe_blocks = block_offsets
        .values()
        .filter(|(rows, offset_sum, pair_count)| {
            *rows >= MIN_PAIR_RUN_ROWS && *pair_count > 0 && offset_sum / pair_count >= *rows
        })
        .count();
    quality
}

fn candidate_has_valid_edition_block(quality: CandidateQuality) -> bool {
    quality.row_count >= MIN_PAIR_RUN_ROWS
        && quality.covered_pages >= MIN_PAIR_RUN_ROWS.saturating_mul(2)
        && quality.max_row_width >= MIN_PAIR_RUN_ROWS
        && quality.medoid_safe_rows == quality.row_count
        && quality.locally_offset_safe_blocks == quality.block_count
}

/// Structural reconstruction is an opt-in repair.  A baseline candidate wins
/// whenever it is a bounded, directly-supported edition layout.  Structural
/// rows are selected only when the baseline cannot form an edition block, or
/// when it has clearly collapsed multiple sparse rows into substantially wider
/// rows while structural reconstruction restores both row count and direct
/// support density.
fn should_select_structural_candidate(
    legacy: CandidateQuality,
    structural: CandidateQuality,
) -> bool {
    if !candidate_has_valid_edition_block(structural) {
        return false;
    }
    if !candidate_has_valid_edition_block(legacy) {
        return true;
    }
    let legacy_is_substantially_wider =
        legacy.max_row_width >= structural.max_row_width.saturating_mul(2);
    legacy_is_substantially_wider && structural.row_count > legacy.row_count
}

impl TrackAssignments {
    fn canonicalize(mut self, prepared: &[PreparedInternalPage]) -> Self {
        let mut ordered = self
            .tracks
            .iter()
            .map(|(track, pages)| {
                (
                    *track,
                    pages
                        .iter()
                        .map(|page| prepared[*page].original.source_page_number.get())
                        .min()
                        .unwrap_or(u32::MAX),
                )
            })
            .collect::<Vec<_>>();
        ordered.sort_by_key(|(_, first_page)| *first_page);
        let remap = ordered
            .into_iter()
            .enumerate()
            .map(|(ordinal, (track, _))| (track, ordinal as u32))
            .collect::<BTreeMap<_, _>>();
        self.page_tracks = self
            .page_tracks
            .into_iter()
            .map(|(page, track)| (page, remap[&track]))
            .collect();
        self.tracks = self
            .tracks
            .into_iter()
            .map(|(track, pages)| (remap[&track], pages))
            .collect();
        self.next_track = self.tracks.len() as u32;
        self
    }

    fn track_for_pages(&self, pages: &[usize]) -> Option<u32> {
        let tracks = pages
            .iter()
            .map(|page| self.page_tracks.get(page).copied())
            .collect::<Option<BTreeSet<_>>>()?;
        (tracks.len() == 1).then(|| *tracks.first().expect("non-empty track set"))
    }

    fn append_namespace(&mut self, other: &Self) {
        let remap = other
            .tracks
            .keys()
            .copied()
            .map(|track| {
                let next = self.next_track;
                self.next_track = self.next_track.saturating_add(1);
                (track, next)
            })
            .collect::<BTreeMap<_, _>>();
        for (track, pages) in &other.tracks {
            self.tracks.insert(remap[track], pages.clone());
        }
        for (page, track) in &other.page_tracks {
            self.page_tracks.insert(*page, remap[track]);
        }
    }

    fn add_pair_constraint(
        &mut self,
        left: &[usize],
        right: &[usize],
        page_rows: &BTreeMap<usize, usize>,
        prepared: &[PreparedInternalPage],
    ) -> bool {
        let left_tracks = left
            .iter()
            .filter_map(|page| self.page_tracks.get(page).copied())
            .collect::<BTreeSet<_>>();
        let right_tracks = right
            .iter()
            .filter_map(|page| self.page_tracks.get(page).copied())
            .collect::<BTreeSet<_>>();
        if !left_tracks.is_disjoint(&right_tracks) {
            return false;
        }
        self.add_constraint(left, page_rows, prepared)
            && self.add_constraint(right, page_rows, prepared)
    }

    fn add_constraint(
        &mut self,
        pages: &[usize],
        page_rows: &BTreeMap<usize, usize>,
        prepared: &[PreparedInternalPage],
    ) -> bool {
        if pages.len() < 2 {
            return false;
        }
        let mut track_ids = pages
            .iter()
            .filter_map(|page| self.page_tracks.get(page).copied())
            .collect::<BTreeSet<_>>();
        let mut members = pages.iter().copied().collect::<BTreeSet<_>>();
        for track_id in &track_ids {
            if let Some(track) = self.tracks.get(track_id) {
                members.extend(track.iter().copied());
            }
        }
        let mut ordered = members
            .iter()
            .filter_map(|page| {
                page_rows.get(page).map(|row| {
                    (
                        *row,
                        prepared[*page].original.source_page_number.get(),
                        *page,
                    )
                })
            })
            .collect::<Vec<_>>();
        if ordered.len() != members.len() {
            return false;
        }
        ordered.sort();
        if ordered
            .windows(2)
            .any(|pair| pair[0].0 == pair[1].0 || pair[0].1 >= pair[1].1)
        {
            return false;
        }
        let track_id = track_ids.pop_first().unwrap_or_else(|| {
            let next = self.next_track;
            self.next_track = self.next_track.saturating_add(1);
            next
        });
        for existing in track_ids {
            self.tracks.remove(&existing);
        }
        for page in &members {
            self.page_tracks.insert(*page, track_id);
        }
        self.tracks.insert(track_id, members);
        true
    }

    fn is_consistent(
        &self,
        page_rows: &BTreeMap<usize, usize>,
        prepared: &[PreparedInternalPage],
    ) -> bool {
        if self.page_tracks.len() != page_rows.len()
            || self
                .page_tracks
                .keys()
                .any(|page| !page_rows.contains_key(page))
        {
            return false;
        }
        if self.tracks.iter().any(|(track, pages)| {
            pages.len() < 2
                || pages
                    .iter()
                    .any(|page| self.page_tracks.get(page) != Some(track))
                || !track_is_monotonic(pages, page_rows, prepared)
        }) {
            return false;
        }
        let mut row_tracks = BTreeMap::<usize, BTreeSet<u32>>::new();
        for (page, track) in &self.page_tracks {
            let Some(row) = page_rows.get(page) else {
                return false;
            };
            if !row_tracks.entry(*row).or_default().insert(*track) {
                return false;
            }
        }
        true
    }
}

fn track_is_monotonic(
    pages: &BTreeSet<usize>,
    page_rows: &BTreeMap<usize, usize>,
    prepared: &[PreparedInternalPage],
) -> bool {
    let mut ordered = pages
        .iter()
        .filter_map(|page| {
            page_rows.get(page).map(|row| {
                (
                    *row,
                    prepared[*page].original.source_page_number.get(),
                    *page,
                )
            })
        })
        .collect::<Vec<_>>();
    if ordered.len() != pages.len() {
        return false;
    }
    ordered.sort();
    !ordered
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 || pair[0].1 >= pair[1].1)
}

fn choose_row_representative(
    indexes: &[usize],
    prepared: &[PreparedInternalPage],
    edges: &[Edge],
) -> Option<usize> {
    indexes
        .iter()
        .copied()
        .filter_map(|candidate| {
            let minimum_similarity = indexes
                .iter()
                .copied()
                .filter(|index| *index != candidate)
                .map(|index| {
                    direct_evidence(candidate, index, edges).map(|edge| edge.visual_similarity)
                })
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .fold(1.0_f64, f64::min);
            Some((candidate, minimum_similarity))
        })
        .max_by(|(left_index, left_score), (right_index, right_score)| {
            left_score.total_cmp(right_score).then_with(|| {
                prepared[*right_index]
                    .original
                    .source_page_number
                    .cmp(&prepared[*left_index].original.source_page_number)
            })
        })
        .map(|(index, _)| index)
}

fn row_evidence(
    representative: usize,
    index: usize,
    prepared: &[PreparedInternalPage],
    edges: &[Edge],
) -> Option<InternalPageEvidence> {
    if representative == index {
        return Some(evidence_for_self(&prepared[index].original));
    }
    direct_evidence(representative, index, edges).map(|value| InternalPageEvidence {
        source_page: prepared[index].original.source_page_number.get(),
        exact_sha256: value.exact_sha256,
        visual_similarity: value.visual_similarity,
        detail_hash_distance: value.detail_hash_distance,
        low_information: value.low_information,
        edition_track_id: None,
        edition_track_ordinal: None,
    })
}

fn direct_evidence(left: usize, right: usize, edges: &[Edge]) -> Option<&DuplicatePagePair> {
    let (left, right) = if left < right {
        (left, right)
    } else {
        (right, left)
    };
    edges
        .iter()
        .find(|edge| edge.left == left && edge.right == right)
        .map(|edge| &edge.evidence)
}

fn evidence_for_self(page: &DuplicatePageHash) -> InternalPageEvidence {
    InternalPageEvidence {
        source_page: page.source_page_number.get(),
        exact_sha256: true,
        visual_similarity: 1.0,
        detail_hash_distance: 0,
        low_information: page.low_information,
        edition_track_id: None,
        edition_track_ordinal: None,
    }
}

fn evidence_for_exact(page: &DuplicatePageHash) -> InternalPageEvidence {
    InternalPageEvidence {
        source_page: page.source_page_number.get(),
        exact_sha256: true,
        visual_similarity: 1.0,
        detail_hash_distance: 0,
        low_information: page.low_information,
        edition_track_id: None,
        edition_track_ordinal: None,
    }
}

fn compare_prepared(
    left: &PreparedInternalPage,
    right: &PreparedInternalPage,
    profile: &HashProfile,
) -> Option<DuplicatePagePair> {
    let exact = left.original.artifact_sha256 == right.original.artifact_sha256;
    let low = left.original.low_information || right.original.low_information;
    if exact {
        return Some(pair(
            &left.original,
            &right.original,
            true,
            0,
            0,
            0,
            1.0,
            low,
        ));
    }
    if low {
        return None;
    }
    let coarse = (left.original.coarse_d_hash ^ right.original.coarse_d_hash).count_ones();
    let phash = (left.original.p_hash ^ right.original.p_hash).count_ones();
    let edge = similarity(
        left.original.edge_density,
        right.original.edge_density,
        0.20,
    );
    let content = similarity(
        left.original.non_uniform_ratio,
        right.original.non_uniform_ratio,
        0.75,
    );
    if coarse > 20 || phash > 16 || edge < 0.62 || content < 0.60 {
        return None;
    }
    let detail = hamming(left.detail_bytes.as_ref()?, right.detail_bytes.as_ref()?);
    let central = central_hamming(left.detail_bytes.as_ref()?, right.detail_bytes.as_ref()?);
    let standard = similarity(left.original.std_dev, right.original.std_dev, 96.);
    let visual = ((1.0 - coarse as f64 / 64.0) * 0.15
        + (1.0 - phash as f64 / 64.0) * 0.25
        + (1.0 - detail as f64 / DETAIL_HASH_BITS as f64) * 0.35
        + edge * 0.15
        + standard * 0.05
        + content * 0.05)
        .clamp(0.0, 1.0);
    if detail > 260 || central > 48 || visual < profile.visual_match_threshold {
        return None;
    }
    Some(pair(
        &left.original,
        &right.original,
        false,
        coarse,
        phash,
        detail,
        visual,
        false,
    ))
}
#[allow(clippy::too_many_arguments)]
fn pair(
    left: &DuplicatePageHash,
    right: &DuplicatePageHash,
    exact_sha256: bool,
    d_hash_distance: u32,
    p_hash_distance: u32,
    detail_hash_distance: u32,
    visual_similarity: f64,
    low_information: bool,
) -> DuplicatePagePair {
    DuplicatePagePair {
        parent_source_page: left.source_page_number.get(),
        candidate_source_page: right.source_page_number.get(),
        exact_sha256,
        d_hash_distance,
        p_hash_distance,
        detail_hash_distance,
        edge_similarity: if exact_sha256 {
            1.0
        } else {
            similarity(left.edge_density, right.edge_density, 0.20)
        },
        visual_similarity,
        low_information,
    }
}
fn decode_detail_hash(value: &str) -> Option<[u8; DETAIL_HASH_BYTES]> {
    if value.len() != DETAIL_HASH_BYTES * 2 {
        return None;
    };
    let mut bytes = [0; DETAIL_HASH_BYTES];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}
fn hamming(left: &[u8; DETAIL_HASH_BYTES], right: &[u8; DETAIL_HASH_BYTES]) -> u32 {
    left.iter()
        .zip(right)
        .map(|(a, b)| (a ^ b).count_ones())
        .sum()
}
fn central_hamming(left: &[u8; DETAIL_HASH_BYTES], right: &[u8; DETAIL_HASH_BYTES]) -> u32 {
    let mut d = 0;
    for y in 7..25 {
        for x in 7..25 {
            let bit = y * 32 + x;
            d += u32::from(
                (left[bit / 8] & (1 << (bit % 8))) != (right[bit / 8] & (1 << (bit % 8))),
            );
        }
    }
    d
}
fn similarity(left: f64, right: f64, scale: f64) -> f64 {
    (1.0 - (left - right).abs() / scale).clamp(0.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
fn group_record(
    run_id: &str,
    artifact: &HashedArtifact,
    block_number: u32,
    sequence_index: u32,
    relation: InternalMatchKind,
    confidence: f64,
    mut pages: Vec<InternalPageEvidence>,
) -> InternalGroupRecord {
    pages.sort_by_key(|page| page.source_page);
    let block_id = block_identifier(artifact, block_number);
    InternalGroupRecord {
        run_id: run_id.into(),
        group: InternalDuplicateGroup {
            group_id: format!("{block_id}-r{sequence_index}"),
            block_id,
            sequence_index,
            revision: 0,
            entry_id: artifact.gallery.entry_id.clone(),
            gallery_id: artifact.gallery.gallery_id,
            relation,
            confidence: confidence.clamp(0., 1.),
            recommended_keep_source_page: pages
                .iter()
                .map(|page| page.source_page)
                .min()
                .unwrap_or(1),
            pages,
            resolved: false,
            created_at: String::new(),
            updated_at: String::new(),
        },
    }
}

fn block_identifier(artifact: &HashedArtifact, block_number: u32) -> String {
    format!(
        "internal-a{}-p{}-g{}-b{}",
        INTERNAL_DUPLICATE_ALGORITHM_VERSION,
        artifact
            .pages
            .first()
            .map_or(1, |page| page.profile_version),
        artifact.gallery.gallery_id.get(),
        block_number
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ArtifactSha256, DuplicateGalleryRef, GalleryId, SourcePageNumber};
    fn page(number: u32, scene: u64) -> DuplicatePageHash {
        DuplicatePageHash {
            entry_id: "entry-1".into(),
            gallery_id: GalleryId::new(1).unwrap(),
            source_page_number: SourcePageNumber::new(number).unwrap(),
            profile_version: 1,
            artifact_sha256: ArtifactSha256::new(format!("{scene:064x}")).unwrap(),
            coarse_d_hash: scene,
            p_hash: scene,
            detail_d_hash_hex: format!("{:02x}", scene.saturating_mul(37)).repeat(128),
            mean_luma: 120.0,
            std_dev: 44.0,
            non_uniform_ratio: 0.7,
            edge_density: 0.2,
            width: 100,
            height: 100,
            low_information: false,
        }
    }
    fn visual_page(number: u32, scene: u64, edition: u8) -> DuplicatePageHash {
        let mut page = page(number, scene);
        page.artifact_sha256 = ArtifactSha256::new(format!("{:064x}", 10_000 + number)).unwrap();
        page.coarse_d_hash ^= u64::from(edition) << 56;
        page.p_hash ^= u64::from(edition) << 48;
        let mut detail = page.detail_d_hash_hex.into_bytes();
        detail[edition as usize % 32] = b'f';
        page.detail_d_hash_hex = String::from_utf8(detail).unwrap();
        page.edge_density += f64::from(edition) * 0.002;
        page.std_dev += f64::from(edition) * 0.1;
        page
    }
    fn black_and_white_page(number: u32, scene: u64) -> DuplicatePageHash {
        let mut page = visual_page(number, scene, 9);
        page.coarse_d_hash ^= u64::MAX;
        page.p_hash ^= u64::MAX;
        page.detail_d_hash_hex = "ff".repeat(DETAIL_HASH_BYTES);
        page
    }
    fn separated_scene_page(number: u32, scene: usize) -> DuplicatePageHash {
        // Pairwise Hamming distance is at least three, so nonmatching scene
        // codes fail the unchanged 1024-bit detail gate.
        const CODES: [u8; 13] = [
            0x00, 0x07, 0x19, 0x1e, 0x2a, 0x2d, 0x33, 0x34, 0x4b, 0x4c, 0x52, 0x55, 0x61,
        ];
        let code = CODES[scene];
        let mut page = visual_page(number, 1, 0);
        page.coarse_d_hash = u64::from(code);
        page.p_hash = u64::from(code).rotate_left(17);
        page.detail_d_hash_hex = format!("{code:02x}").repeat(DETAIL_HASH_BYTES);
        page
    }
    fn artifact(pages: Vec<DuplicatePageHash>) -> HashedArtifact {
        HashedArtifact {
            gallery: DuplicateGalleryRef {
                gallery_id: GalleryId::new(1).unwrap(),
                entry_id: "entry-1".into(),
                title: "fixture".into(),
                artist: None,
                group: None,
                page_count: pages.len() as u32,
            },
            pages,
        }
    }
    #[test]
    fn four_editions_form_five_nway_rows() {
        let pages = (0..4)
            .flat_map(|edition| {
                (0..5).map(move |scene| page(edition * 5 + scene + 1, u64::from(scene + 1)))
            })
            .collect::<Vec<_>>();
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 5, "{:#?}", found.groups);
        for (scene, row) in found.groups.iter().enumerate() {
            assert_eq!(
                row.group
                    .pages
                    .iter()
                    .map(|p| p.source_page)
                    .collect::<Vec<_>>(),
                vec![
                    scene as u32 + 1,
                    scene as u32 + 6,
                    scene as u32 + 11,
                    scene as u32 + 16
                ]
            );
        }
        let tracks = found.groups[0]
            .group
            .pages
            .iter()
            .map(|page| (page.edition_track_id.clone(), page.edition_track_ordinal))
            .collect::<Vec<_>>();
        assert_eq!(
            tracks,
            vec![
                (Some("internal-a4-p1-g1-b1-t0".into()), Some(0)),
                (Some("internal-a4-p1-g1-b1-t1".into()), Some(1)),
                (Some("internal-a4-p1-g1-b1-t2".into()), Some(2)),
                (Some("internal-a4-p1-g1-b1-t3".into()), Some(3)),
            ]
        );
    }
    #[test]
    fn visual_only_four_editions_keep_deterministic_tracks_without_forging_exact_evidence() {
        let pages: Vec<_> = (0..4)
            .flat_map(|edition| {
                (0..5).map(move |scene| {
                    visual_page(edition * 5 + scene + 1, u64::from(scene + 1), edition as u8)
                })
            })
            .collect();
        let first =
            detect_internal_groups("run", &artifact(pages.clone()), &HashProfile::current());
        let second = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(first.groups, second.groups);
        assert_eq!(first.groups.len(), 5);
        assert!(
            first.groups.iter().all(|group| {
                group.group.relation == InternalMatchKind::TranslationVisual
                    && group.group.pages.len() == 4
                    && group
                        .group
                        .pages
                        .iter()
                        .filter(|page| page.exact_sha256)
                        .count()
                        == 1
            }),
            "{:#?}",
            first.groups
        );
        for ordinal in 0..4 {
            let track_pages = first
                .groups
                .iter()
                .filter_map(|group| {
                    group
                        .group
                        .pages
                        .iter()
                        .find(|page| page.edition_track_ordinal == Some(ordinal))
                        .map(|page| page.source_page)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                track_pages,
                (ordinal * 5 + 1..=ordinal * 5 + 5).collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn missing_scene_keeps_a_track_without_shifting_later_rows() {
        let mut pages = Vec::new();
        for edition in 0..4_u32 {
            for scene in 0..5_u32 {
                if edition == 2 && scene == 2 {
                    continue;
                }
                pages.push(visual_page(
                    edition * 5 + scene + 1,
                    u64::from(scene + 1),
                    edition as u8,
                ));
            }
        }
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 5);
        let third_row = &found.groups[2].group.pages;
        assert_eq!(
            third_row
                .iter()
                .map(|page| page.source_page)
                .collect::<Vec<_>>(),
            vec![3, 8, 18]
        );
        let track_c = found
            .groups
            .iter()
            .flat_map(|group| group.group.pages.iter())
            .filter(|page| page.edition_track_ordinal == Some(2))
            .map(|page| page.source_page)
            .collect::<Vec<_>>();
        assert_eq!(track_c, vec![11, 12, 14, 15]);
    }
    #[test]
    fn three_tracks_keep_a_missing_middle_scene_on_its_original_track() {
        let mut pages = Vec::new();
        let mut number = 1;
        for edition in 0..3_u8 {
            for scene in 0..5_u64 {
                if edition == 2 && scene == 2 {
                    continue;
                }
                pages.push(visual_page(number, scene + 1, edition));
                number += 1;
            }
        }
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 5, "{:#?}", found.groups);
        assert_eq!(
            found.groups[2]
                .group
                .pages
                .iter()
                .map(|page| page.source_page)
                .collect::<Vec<_>>(),
            vec![3, 8]
        );
        let third_track = found
            .groups
            .iter()
            .flat_map(|group| group.group.pages.iter())
            .filter(|page| page.edition_track_ordinal == Some(2))
            .map(|page| page.source_page)
            .collect::<Vec<_>>();
        assert_eq!(third_track, vec![11, 12, 13, 14]);
    }
    #[test]
    fn four_three_four_tracks_keep_the_middle_gap_without_shifting_later_rows() {
        let mut pages = Vec::new();
        let mut number = 1;
        for edition in 0..3_u8 {
            for scene in 1..=4_u64 {
                if edition == 1 && scene == 3 {
                    continue;
                }
                pages.push(visual_page(number, scene, edition));
                number += 1;
            }
        }
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 4, "{:#?}", found.groups);
        assert_eq!(
            found
                .groups
                .iter()
                .map(|group| {
                    group
                        .group
                        .pages
                        .iter()
                        .map(|page| page.source_page)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
            vec![vec![1, 5, 8], vec![2, 6, 9], vec![3, 10], vec![4, 7, 11]]
        );
        let third_track = found
            .groups
            .iter()
            .flat_map(|group| group.group.pages.iter())
            .filter(|page| page.edition_track_ordinal == Some(2))
            .map(|page| page.source_page)
            .collect::<Vec<_>>();
        assert_eq!(third_track, vec![8, 9, 10, 11]);
    }
    #[test]
    fn a_missing_middle_scene_beats_a_longer_shifted_alignment() {
        let prepared = (1..=11)
            .map(|number| PreparedInternalPage {
                detail_bytes: None,
                original: page(number, u64::from(number)),
            })
            .collect::<Vec<_>>();
        let evidence = |left, right, similarity| Edge {
            left,
            right,
            evidence: DuplicatePagePair {
                parent_source_page: left as u32 + 1,
                candidate_source_page: right as u32 + 1,
                exact_sha256: false,
                d_hash_distance: 0,
                p_hash_distance: 0,
                detail_hash_distance: 0,
                edge_similarity: similarity,
                visual_similarity: similarity,
                low_information: false,
            },
        };
        // Three tracks with an absent middle-scene page in the second track.
        // The contiguous path through 3↔7↔8 is visually strong but shifts
        // every subsequent row. The gapped alignments retain that absence.
        let edges = vec![
            evidence(0, 4, 0.95),
            evidence(0, 7, 0.95),
            evidence(0, 8, 0.80),
            evidence(1, 5, 0.95),
            evidence(1, 8, 0.95),
            evidence(1, 9, 0.80),
            evidence(2, 6, 0.80),
            evidence(3, 6, 0.95),
            evidence(3, 7, 0.80),
            evidence(3, 10, 0.95),
            evidence(4, 7, 0.95),
            evidence(4, 8, 0.80),
            evidence(5, 8, 0.95),
            evidence(5, 9, 0.80),
            evidence(6, 10, 0.95),
        ];
        let blocks = merge_runs(&monotonic_runs(&edges), &edges, &prepared);
        assert_eq!(
            blocks.len(),
            1,
            "{}",
            structural_block_count(&StructuralState {
                blocks: blocks.clone(),
                ..StructuralState::default()
            })
        );
        assert_eq!(
            blocks[0]
                .scene
                .rows
                .iter()
                .map(|row| row.iter().map(|page| page + 1).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![vec![1, 5, 8], vec![2, 6, 9], vec![4, 7, 11]],
            "{:#?}",
            blocks[0].scene.rows
        );
    }
    #[test]
    fn matching_three_track_topology_keeps_album_tracks_across_scene_blocks() {
        let mut pages = Vec::new();
        let mut number = 1;
        for scene_base in [0_u64, 3] {
            for edition in 0..3_u8 {
                for scene in 1..=3_u64 {
                    pages.push(visual_page(number, scene_base + scene, edition));
                    number += 1;
                }
            }
        }
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 6, "{:#?}", found.groups);
        assert_eq!(
            found
                .groups
                .iter()
                .map(|group| group.group.block_id.clone())
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        for ordinal in 0..3 {
            let ids = found
                .groups
                .iter()
                .flat_map(|group| group.group.pages.iter())
                .filter(|page| page.edition_track_ordinal == Some(ordinal))
                .filter_map(|page| page.edition_track_id.clone())
                .collect::<BTreeSet<_>>();
            assert_eq!(ids.len(), 1, "ordinal {ordinal}: {ids:?}");
        }
    }
    #[test]
    fn unequal_scene_blocks_keep_ordered_album_track_scopes() {
        let mut pages = Vec::new();
        let mut number = 1;
        for edition in 0..3_u8 {
            for scene in 1..=3_u64 {
                pages.push(visual_page(number, scene, edition));
                number += 1;
            }
        }
        for edition in 0..3_u8 {
            for scene in 4..=6_u64 {
                if edition == 2 && scene == 5 {
                    continue;
                }
                pages.push(visual_page(number, scene, edition));
                number += 1;
            }
        }
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 6, "{:#?}", found.groups);
        assert_eq!(
            found
                .groups
                .iter()
                .map(|group| group.group.block_id.clone())
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        let third_track_ids = found
            .groups
            .iter()
            .flat_map(|group| group.group.pages.iter())
            .filter(|page| page.edition_track_ordinal == Some(2))
            .filter_map(|page| page.edition_track_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(third_track_ids.len(), 1, "{third_track_ids:?}");
    }
    #[test]
    fn same_track_similar_consecutive_scenes_do_not_form_edition_rows() {
        let pages = (0..3_u32)
            .flat_map(|scene| {
                [
                    visual_page(scene * 2 + 1, u64::from(scene + 1), 0),
                    visual_page(scene * 2 + 2, u64::from(scene + 1), 1),
                ]
            })
            .collect();
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert!(found.groups.is_empty(), "{:#?}", found.groups);
    }
    #[test]
    fn short_spurious_run_is_not_promoted_to_a_scene_block() {
        let pages = (0..2_u32)
            .flat_map(|scene| {
                [
                    visual_page(scene + 1, u64::from(scene + 1), 0),
                    visual_page(scene + 3, u64::from(scene + 1), 1),
                ]
            })
            .collect();
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert!(found.groups.is_empty(), "{:#?}", found.groups);
    }
    #[test]
    fn strong_edition_runs_outrank_a_within_track_visual_subrun() {
        let scenes = (0..25)
            .map(|index| if index < 24 { index % 12 } else { 12 })
            .collect::<Vec<_>>();
        let pages = (0..3_u32)
            .flat_map(|edition| {
                scenes.iter().enumerate().map(move |(scene_index, scene)| {
                    separated_scene_page(edition * 25 + scene_index as u32 + 1, *scene)
                })
            })
            .collect();
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 25, "{:#?}", found.groups);
        for (scene, group) in found.groups.iter().enumerate() {
            assert_eq!(
                group
                    .group
                    .pages
                    .iter()
                    .map(|page| page.source_page)
                    .collect::<Vec<_>>(),
                vec![scene as u32 + 1, scene as u32 + 26, scene as u32 + 51]
            );
        }
    }
    #[test]
    fn split_track_postmerge_preserves_one_track_per_row() {
        let prepared = (1..=8)
            .map(|number| PreparedInternalPage {
                detail_bytes: None,
                original: page(number, u64::from(number)),
            })
            .collect::<Vec<_>>();
        let page_rows = (0..8)
            .map(|page| (page, page % 4))
            .collect::<BTreeMap<_, _>>();
        let mut assignments = TrackAssignments::default();
        assert!(assignments.add_pair_constraint(&[0, 1], &[4, 5], &page_rows, &prepared));
        assert!(assignments.add_pair_constraint(&[2, 3], &[6, 7], &page_rows, &prepared));
        assert!(assignments.add_constraint(&[0, 1, 2, 3], &page_rows, &prepared));
        assert!(assignments.is_consistent(&page_rows, &prepared));
        assert_eq!(assignments.track_for_pages(&[0, 1, 2, 3]), Some(0));
    }
    #[test]
    fn color_and_black_and_white_pages_fail_the_existing_visual_gates() {
        let color = PreparedInternalPage {
            detail_bytes: decode_detail_hash(&visual_page(1, 1, 0).detail_d_hash_hex),
            original: visual_page(1, 1, 0),
        };
        let black_and_white = PreparedInternalPage {
            detail_bytes: decode_detail_hash(&black_and_white_page(2, 1).detail_d_hash_hex),
            original: black_and_white_page(2, 1),
        };
        assert!(compare_prepared(&color, &black_and_white, &HashProfile::current()).is_none());
    }
    #[test]
    fn single_language_color_black_and_white_pages_do_not_form_rows() {
        let mut pages = Vec::new();
        for scene in 1..=3_u64 {
            pages.push(visual_page(pages.len() as u32 + 1, scene, 0));
        }
        for scene in 1..=3_u64 {
            pages.push(black_and_white_page(pages.len() as u32 + 1, scene));
        }
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert!(found.groups.is_empty(), "{:#?}", found.groups);
    }
    #[test]
    fn two_way_five_scene_alignment_remains_a_block() {
        let pages = (0..2_u8)
            .flat_map(|edition| {
                (0..5_u64).map(move |scene| {
                    visual_page(
                        u32::from(edition) * 5 + scene as u32 + 1,
                        scene + 1,
                        edition,
                    )
                })
            })
            .collect();
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 5, "{:#?}", found.groups);
        assert!(found
            .groups
            .iter()
            .all(|group| group.group.pages.len() == 2));
    }
    #[test]
    fn exact_pages_remain_detectable_without_a_structural_run() {
        let found = detect_internal_groups(
            "run",
            &artifact(vec![page(1, 77), page(2, 77)]),
            &HashProfile::current(),
        );
        assert_eq!(found.groups.len(), 1);
        assert_eq!(found.groups[0].group.relation, InternalMatchKind::Exact);
        assert!(found.groups[0]
            .group
            .pages
            .iter()
            .all(|page| page.edition_track_id.is_none()));
    }
    #[test]
    fn two_hundred_eleven_pages_keep_quadratic_comparison_count_bounded() {
        let pages = (1..=211)
            .map(|number| page(number, u64::from(number) * 17))
            .collect();
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.compared_pairs, 22_155);
    }
    #[test]
    fn pair_progress_is_bounded_and_reports_the_terminal_comparison() {
        let pages = (1..=211)
            .map(|number| page(number, u64::from(number) * 19))
            .collect();
        let mut updates = Vec::new();
        let found = detect_internal_groups_with_progress(
            "run",
            &artifact(pages),
            &HashProfile::current(),
            |compared, total| updates.push((compared, total)),
        );

        assert_eq!(found.compared_pairs, 22_155);
        assert!(updates.len() <= 101, "updates={}", updates.len());
        assert_eq!(updates.last(), Some(&(22_155, 22_155)));
        assert!(updates.windows(2).all(|window| window[0].0 < window[1].0));
    }
    #[test]
    fn hybrid_selector_preserves_a_bounded_three_or_four_track_legacy_block() {
        let legacy = CandidateQuality {
            row_count: 5,
            covered_pages: 20,
            max_row_width: 4,
            direct_pairs: 30,
            possible_pairs: 30,
            medoid_safe_rows: 5,
            block_count: 1,
            locally_offset_safe_blocks: 1,
        };
        let structural = CandidateQuality {
            max_row_width: 3,
            ..legacy
        };
        assert!(candidate_has_valid_edition_block(legacy));
        assert!(!should_select_structural_candidate(legacy, structural));
    }
    #[test]
    fn hybrid_selector_repairs_a_sparse_legacy_mega_row() {
        let legacy = CandidateQuality {
            row_count: 2,
            covered_pages: 70,
            max_row_width: 35,
            direct_pairs: 68,
            possible_pairs: 1_190,
            medoid_safe_rows: 2,
            block_count: 1,
            locally_offset_safe_blocks: 1,
        };
        let structural = CandidateQuality {
            row_count: 25,
            covered_pages: 75,
            max_row_width: 3,
            direct_pairs: 75,
            possible_pairs: 75,
            medoid_safe_rows: 25,
            block_count: 1,
            locally_offset_safe_blocks: 1,
        };
        assert!(should_select_structural_candidate(legacy, structural));
    }
    #[test]
    fn hybrid_selector_uses_a_strong_structural_block_when_legacy_is_empty() {
        let structural = CandidateQuality {
            row_count: 3,
            covered_pages: 6,
            max_row_width: 2,
            direct_pairs: 3,
            possible_pairs: 3,
            medoid_safe_rows: 3,
            block_count: 1,
            locally_offset_safe_blocks: 1,
        };
        assert!(should_select_structural_candidate(
            CandidateQuality::default(),
            structural
        ));
    }
    #[test]
    fn structural_track_pruning_keeps_a_strong_block_and_drops_a_weak_tail() {
        let mut main = StructuralBlock::default();
        main.scene.rows = (0..5)
            .map(|row| {
                let mut pages = BTreeSet::from([row, row + 5]);
                if row < 3 {
                    pages.insert(row + 10);
                }
                pages
            })
            .collect();
        assert!(main.scene.sort_and_index_rows());
        for (track, pages) in [
            (0, (0..5).collect::<BTreeSet<_>>()),
            (1, (5..10).collect::<BTreeSet<_>>()),
            (2, (10..13).collect::<BTreeSet<_>>()),
        ] {
            for page in &pages {
                main.tracks.page_tracks.insert(*page, track);
            }
            main.tracks.tracks.insert(track, pages);
        }
        assert!(prune_weak_structural_tracks(&mut main));
        assert_eq!(main.scene.rows.len(), 5);
        assert_eq!(main.tracks.tracks.len(), 2);
        assert!(main.scene.rows.iter().all(|row| row.len() == 2));

        let mut tail = StructuralBlock::default();
        tail.scene.rows = (0..5)
            .map(|row| {
                let mut pages = BTreeSet::from([row]);
                if row < 3 {
                    pages.extend([row + 5, row + 8]);
                }
                pages
            })
            .collect();
        assert!(tail.scene.sort_and_index_rows());
        for (track, pages) in [
            (0, (0..5).collect::<BTreeSet<_>>()),
            (1, (5..8).collect::<BTreeSet<_>>()),
            (2, (8..11).collect::<BTreeSet<_>>()),
        ] {
            for page in &pages {
                tail.tracks.page_tracks.insert(*page, track);
            }
            tail.tracks.tracks.insert(track, pages);
        }
        assert!(!prune_weak_structural_tracks(&mut tail));
    }
    #[test]
    fn one_shared_panel_does_not_form_a_block() {
        let mut shared_but_reencoded = page(3, 1);
        shared_but_reencoded.artifact_sha256 = ArtifactSha256::new(format!("{:064x}", 99)).unwrap();
        let found = detect_internal_groups(
            "run",
            &artifact(vec![page(1, 1), page(2, 2), shared_but_reencoded]),
            &HashProfile::current(),
        );
        assert!(found.groups.is_empty());
    }
    #[test]
    fn repeated_edition_cycles_remain_separate_blocks() {
        let pages = (0..2)
            .flat_map(|cycle| {
                (0..4).flat_map(move |edition| {
                    (0..5).map(move |scene| {
                        page(
                            cycle * 20 + edition * 5 + scene + 1,
                            u64::from(cycle * 100 + scene + 1),
                        )
                    })
                })
            })
            .collect();
        let found = detect_internal_groups("run", &artifact(pages), &HashProfile::current());
        assert_eq!(found.groups.len(), 10, "{:#?}", found.groups);
        let block_ids = found
            .groups
            .iter()
            .map(|group| group.group.block_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(block_ids.len(), 2);
        assert_eq!(
            found.groups[5]
                .group
                .pages
                .iter()
                .map(|page| page.source_page)
                .collect::<Vec<_>>(),
            vec![21, 26, 31, 36]
        );
    }
}
