//! Explicit selected-source rules. Capture/favorite/account state is never opt-in.
use super::{
    catalog::Catalog, model::{now, CloudVisibility}, CloudError, CloudService, CloudSnapshot, CloudSourceRef, Inner,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum CloudAutomaticTarget {
    Channel {
        provider: String,
        #[serde(rename = "channelId")]
        channel_id: String,
    },
    Recording {
        provider: String,
        #[serde(rename = "recordingId")]
        recording_id: String,
    },
}
impl CloudAutomaticTarget {
    pub fn provider(&self) -> &str {
        match self {
            Self::Channel { provider, .. } | Self::Recording { provider, .. } => provider,
        }
    }
    fn valid(&self) -> bool {
        if !matches!(self.provider(), "chzzk" | "soop") {
            return false;
        }
        match self {
            Self::Channel {
                provider,
                channel_id,
            } if provider == "chzzk" => {
                channel_id.len() == 32
                    && channel_id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }
            Self::Channel { channel_id, .. } => {
                !channel_id.is_empty()
                    && channel_id.len() <= 40
                    && channel_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            }
            Self::Recording {
                provider,
                recording_id,
            } => super::source::validate_reference(&CloudSourceRef {
                provider: provider.clone(),
                recording_id: recording_id.clone(),
                part_index: None,
                plan_id: None,
                unit_id: None,
            })
            .is_ok(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAutomaticRule {
    pub id: String,
    pub target: CloudAutomaticTarget,
    pub enabled: bool,
    pub visibility: CloudVisibility,
    pub generation: u64,
    pub activated_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_channel_id: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CloudError>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAutomaticSnapshot {
    pub revision: u64,
    pub rules: Vec<CloudAutomaticRule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CloudError>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomaticReadiness {
    Recording,
    Processing,
    Ready,
    Failed,
    Missing,
}
#[derive(Clone, Debug)]
pub struct AutomaticCandidate {
    pub source: CloudSourceRef,
    pub channel_id: String,
    pub started_at_ms: u64,
    pub readiness: AutomaticReadiness,
}
pub type AutomaticCandidateResolver =
    Arc<dyn Fn(&[CloudAutomaticRule]) -> Result<Vec<AutomaticCandidate>, String> + Send + Sync>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Rule {
    id: String,
    target: CloudAutomaticTarget,
    enabled: bool,
    // Rules created before the selector explicitly promised link-access uploads.
    #[serde(default = "legacy_rule_visibility")]
    visibility: CloudVisibility,
    generation: u64,
    activated_at_ms: u64,
    #[serde(default)]
    owner_channel_id: Option<String>,
}
fn legacy_rule_visibility() -> CloudVisibility { CloudVisibility::Unlisted }
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Config {
    pub revision: u64,
    rules: Vec<Rule>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            revision: 1,
            rules: Vec::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Origin {
    pub rule_id: String,
    pub generation: u64,
    pub resume_intent: bool,
    #[serde(default)]
    pub paused_by_user: bool,
}
impl Origin {
    pub(super) fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.rule_id).is_ok() && self.generation > 0
    }
}
impl Config {
    pub(super) fn valid(&self) -> bool {
        let mut ids = std::collections::HashSet::new();
        self.revision > 0
            && self.rules.len() <= 256
            && self.rules.iter().enumerate().all(|(index, rule)| {
                rule.target.valid()
                    && rule.generation > 0
                    && rule.activated_at_ms > 0
                    && uuid::Uuid::parse_str(&rule.id).is_ok()
                    && ids.insert(&rule.id)
                    && rule
                        .owner_channel_id
                        .as_deref()
                        .is_none_or(super::model::channel_id)
                    && !self.rules[..index]
                        .iter()
                        .any(|other| other.target == rule.target)
            })
    }
    pub(super) fn bind_unbound(&mut self, owner: &str) {
        let mut changed = false;
        for rule in &mut self.rules {
            if rule.enabled && rule.owner_channel_id.is_none() {
                rule.owner_channel_id = Some(owner.into());
                changed = true;
            }
        }
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
    }
}
fn matches(rule: &Rule, candidate: &AutomaticCandidate) -> bool {
    candidate.source.part_index.is_none()
        && candidate.source.plan_id.is_none()
        && candidate.source.unit_id.is_none()
        && super::source::validate_reference(&candidate.source).is_ok()
        && match &rule.target {
            CloudAutomaticTarget::Channel {
                provider,
                channel_id,
            } => {
                candidate.source.provider == *provider
                    && candidate.channel_id == *channel_id
                    && candidate.started_at_ms >= rule.activated_at_ms
            }
            CloudAutomaticTarget::Recording {
                provider,
                recording_id,
            } => {
                candidate.source.provider == *provider
                    && candidate.source.recording_id == *recording_id
            }
        }
}
fn active(rule: &Rule, owner: Option<&str>) -> bool {
    rule.enabled && owner.is_some() && rule.owner_channel_id.as_deref() == owner
}
pub(super) fn current(catalog: &Catalog, origin: &Origin, owner: &str) -> bool {
    catalog.automatic.rules.iter().any(|rule| {
        rule.id == origin.rule_id
            && rule.generation == origin.generation
            && active(rule, Some(owner))
    })
}
pub(super) struct Admission {
    pub rule_id: String,
    pub generation: u64,
    pub candidate: AutomaticCandidate,
}
pub(super) fn admit(
    catalog: &Catalog,
    admission: &Admission,
    owner: &str,
) -> Result<(Origin, CloudVisibility), CloudError> {
    let rule = catalog.automatic.rules.iter().find(|rule| {
        rule.id == admission.rule_id
            && rule.generation == admission.generation
            && active(rule, Some(owner))
            && matches(rule, &admission.candidate)
    });
    if rule.is_none() || admission.candidate.readiness != AutomaticReadiness::Ready {
        return Err(CloudError::new(
            "CLOUD_RULE_CHANGED",
            "자동 업로드 선택이 변경됐습니다.",
            false,
        ));
    }
    Ok((Origin {
        rule_id: admission.rule_id.clone(),
        generation: admission.generation,
        resume_intent: true,
        paused_by_user: false,
    }, rule.expect("validated rule").visibility))
}

pub(super) fn snapshot(inner: &Inner) -> CloudAutomaticSnapshot {
    let owner = inner
        .catalog
        .grant
        .as_ref()
        .map(|grant| grant.channel_id.as_str());
    let rules = inner.catalog.automatic.rules.iter().map(|rule| {
        let latest = inner.catalog.jobs.iter().filter(|job| {
            job.channel_id.as_str() == rule.owner_channel_id.as_deref().unwrap_or("") && (
                job.automatic.as_ref().is_some_and(|origin| origin.rule_id == rule.id && origin.generation == rule.generation)
                || matches!(&rule.target, CloudAutomaticTarget::Recording { provider, recording_id } if *provider == job.view.source.provider && *recording_id == job.view.source.recording_id))
        }).max_by_key(|job| job.view.updated_at);
        let status = if !rule.enabled { "disabled" } else if !active(rule, owner) { "waiting_account" } else if let Some(job) = latest {
            match job.view.state.as_str() { "retry_wait" => "queued", "waiting_auth" => "waiting_account", state => state }
        } else { match inner.automatic_observations.get(&rule.id) {
            Some(AutomaticReadiness::Recording) => "waiting_recording",
            Some(AutomaticReadiness::Processing | AutomaticReadiness::Ready) => "waiting_processing",
            Some(AutomaticReadiness::Failed) => "failed", Some(AutomaticReadiness::Missing) => "source_missing",
            None => if matches!(rule.target, CloudAutomaticTarget::Channel { .. }) { "watching" } else { "waiting_recording" },
        }};
        CloudAutomaticRule { id:rule.id.clone(), target:rule.target.clone(), enabled:rule.enabled, visibility:rule.visibility, generation:rule.generation, activated_at_ms:rule.activated_at_ms, owner_channel_id:rule.owner_channel_id.clone(), status:status.into(), error:latest.and_then(|job|job.view.error.clone()) }
    }).collect();
    CloudAutomaticSnapshot {
        revision: inner.catalog.automatic.revision,
        rules,
        error: inner.automatic_error.clone(),
    }
}

impl CloudService {
    pub fn automatic_set(
        &self,
        target: CloudAutomaticTarget,
        enabled: bool,
        expected_revision: u64,
    ) -> Result<CloudSnapshot, CloudError> {
        self.automatic_set_with_visibility(target, enabled, expected_revision, None)
    }
    pub fn automatic_set_with_visibility(
        &self, target: CloudAutomaticTarget, enabled: bool, expected_revision: u64,
        visibility: Option<CloudVisibility>,
    ) -> Result<CloudSnapshot, CloudError> {
        if !target.valid() {
            return Err(CloudError::new(
                "CLOUD_RULE_TARGET",
                "치지직·SOOP 채널 또는 녹화본을 선택해 주세요.",
                false,
            ));
        }
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if inner.catalog.automatic.revision != expected_revision {
            return Err(CloudError::new(
                "CLOUD_RULE_REVISION",
                "자동 업로드 설정이 변경됐습니다. 새로 확인해 주세요.",
                true,
            ));
        }
        let owner = inner
            .catalog
            .grant
            .as_ref()
            .map(|grant| grant.channel_id.clone());
        let mut candidate = inner.catalog.clone();
        let existing = candidate
            .automatic
            .rules
            .iter()
            .position(|rule| rule.target == target);
        if let Some(index) = existing {
            let rule = &mut candidate.automatic.rules[index];
            if enabled
                && rule
                    .owner_channel_id
                    .as_ref()
                    .zip(owner.as_ref())
                    .is_some_and(|(bound, current)| bound != current)
            {
                return Err(CloudError::new(
                    "CLOUD_ACCOUNT_MISMATCH",
                    "이 선택에 연결된 YouTube 채널로 로그인해 주세요.",
                    false,
                ));
            }
            let visibility = visibility.unwrap_or(rule.visibility);
            if rule.enabled == enabled && rule.visibility == visibility {
                return Ok(Self::snapshot_inner(&inner));
            }
            // Future admissions use this choice. Already admitted jobs, source
            // cutoff and generation stay unchanged for a visibility-only edit.
            rule.visibility = visibility;
            let enabled_changed = rule.enabled != enabled;
            rule.enabled = enabled;
            if enabled_changed {
                rule.generation = rule.generation.checked_add(1).ok_or_else(CloudError::storage)?;
            }
            if enabled && enabled_changed {
                rule.activated_at_ms = now();
                if rule.owner_channel_id.is_none() {
                    rule.owner_channel_id = owner;
                }
            }
            if !enabled {
                let rule_id = rule.id.clone();
                for job in &mut candidate.jobs {
                    if let Some(origin) = job
                        .automatic
                        .as_mut()
                        .filter(|origin| origin.rule_id == rule_id)
                    {
                        origin.resume_intent = false;
                        if !matches!(job.view.state.as_str(), "ready" | "failed") {
                            if let Some((id, cancel)) = &inner.active {
                                if id == &job.view.id {
                                    cancel.store(true, std::sync::atomic::Ordering::Release);
                                }
                            }
                            job.view.state = "paused".into();
                            job.view.retry_at = None;
                            job.view.error = None;
                        }
                    }
                }
            }
        } else {
            if !enabled {
                return Ok(Self::snapshot_inner(&inner));
            }
            if candidate.automatic.rules.len() >= 256 {
                return Err(CloudError::new(
                    "CLOUD_RULE_LIMIT",
                    "자동 업로드 선택이 너무 많습니다.",
                    false,
                ));
            }
            candidate.automatic.rules.push(Rule {
                id: uuid::Uuid::new_v4().to_string(),
                target,
                enabled: true,
                visibility: visibility.unwrap_or_default(),
                generation: 1,
                activated_at_ms: now(),
                owner_channel_id: owner,
            });
        }
        candidate.automatic.revision = candidate
            .automatic
            .revision
            .checked_add(1)
            .ok_or_else(CloudError::storage)?;
        self.publish(&mut inner, candidate)?;
        inner.automatic_error = None;
        self.wake.notify_all();
        Ok(Self::snapshot_inner(&inner))
    }

    /// Start once under AppState ownership. No rules/account means no source scan,
    /// HTTP, capture, planner, or browser initialization by this supervisor.
    pub fn start_automatic(
        self: &Arc<Self>,
        resolver: AutomaticCandidateResolver,
    ) -> Result<(), CloudError> {
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if inner.automatic_running {
            return Ok(());
        }
        inner.automatic_running = true;
        let service = self.clone();
        let handle = std::thread::Builder::new().name("atsumi-cloud-selected".into()).spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                loop {
                    let rules = {
                        let Ok(inner) = service.lock() else { break; };
                        if inner.closing { break; }
                        snapshot(&inner).rules.into_iter().filter(|rule| inner.persistence_error.is_none() && rule.enabled && rule.owner_channel_id.as_deref().is_some_and(|owner| inner.catalog.grant.as_ref().is_some_and(|grant| grant.channel_id == owner))).collect::<Vec<_>>()
                    };
                    if !rules.is_empty() {
                        let mut candidates = Vec::new();
                        let mut source_failed = false;
                        for provider in ["chzzk", "soop"] {
                            let selected = rules.iter().filter(|rule|rule.target.provider() == provider).cloned().collect::<Vec<_>>();
                            if selected.is_empty() { continue; }
                            match resolver(&selected) { Ok(found) => candidates.extend(found), Err(_) => source_failed = true }
                        }
                        let result = service.reconcile_automatic(candidates).and_then(|()| {
                            if source_failed { Err(CloudError::new("CLOUD_AUTO_SOURCE", "일부 선택한 녹화 상태를 확인하지 못했습니다. 다른 녹화는 계속 처리합니다.", true)) } else { Ok(()) }
                        });
                        if let Ok(mut inner) = service.lock() { inner.automatic_error = result.err(); }
                    }
                    let Ok(inner) = service.lock() else { break; };
                    if inner.closing { break; }
                    drop(service.wake.wait_timeout(inner, Duration::from_secs(3)));
                }
            }));
            if let Ok(mut inner) = service.lock() {
                inner.automatic_running = false;
                if outcome.is_err() { inner.automatic_error = Some(CloudError::new("CLOUD_AUTO_STOPPED", "자동 업로드 상태 확인이 중단됐습니다. 원본은 유지됩니다.", true)); }
            }
            service.wake.notify_all();
        }).map_err(|_| CloudError::storage());
        match handle {
            Ok(handle) => self.add_thread(handle),
            Err(error) => {
                inner.automatic_running = false;
                Err(error)
            }
        }
    }

    pub(super) fn reconcile_automatic(
        self: &Arc<Self>,
        candidates: Vec<AutomaticCandidate>,
    ) -> Result<(), CloudError> {
        if candidates.len() > 8192 {
            return Err(CloudError::new(
                "CLOUD_AUTO_SOURCE",
                "선택한 녹화 목록을 나누어 확인해 주세요.",
                true,
            ));
        }
        let (admissions, wake_worker) = {
            let mut inner = self.lock()?;
            Self::admit(&inner)?;
            let Some(owner) = inner
                .catalog
                .grant
                .as_ref()
                .map(|grant| grant.channel_id.clone())
            else {
                return Ok(());
            };
            let rules = inner.catalog.automatic.rules.clone();
            let mut observations = HashMap::new();
            let mut admissions = Vec::new();
            for rule in rules.iter().filter(|rule| active(rule, Some(&owner))) {
                for candidate in candidates
                    .iter()
                    .filter(|candidate| matches(rule, candidate))
                {
                    observations.insert(rule.id.clone(), candidate.readiness);
                    if candidate.readiness == AutomaticReadiness::Ready {
                        admissions.push(Admission {
                            rule_id: rule.id.clone(),
                            generation: rule.generation,
                            candidate: candidate.clone(),
                        });
                    }
                }
            }
            inner.automatic_observations = observations;
            let mut catalog = inner.catalog.clone();
            let mut changed = false;
            for job in &mut catalog.jobs {
                // An overlapping still-enabled explicit selection may take over
                // an off rule's paused job, but never a user's per-job pause or
                // a terminal failure. Reuse its receipt/session; no new video.
                if job.channel_id == owner
                    && matches!(job.view.state.as_str(), "paused" | "waiting_auth")
                    && job.automatic.as_ref().is_some_and(|origin| {
                        !origin.paused_by_user && !current(&inner.catalog, origin, &owner)
                    })
                {
                    if let Some(rule) = rules.iter().find(|rule| {
                        active(rule, Some(&owner))
                            && candidates.iter().any(|candidate| {
                                candidate.source.provider == job.view.source.provider
                                    && candidate.source.recording_id == job.view.source.recording_id
                                    && candidate.readiness == AutomaticReadiness::Ready
                                    && matches(rule, candidate)
                            })
                    }) {
                        job.automatic = Some(Origin {
                            rule_id: rule.id.clone(),
                            generation: rule.generation,
                            resume_intent: true,
                            paused_by_user: false,
                        });
                        job.view.error = Some(CloudError::cancelled());
                        changed = true;
                    }
                }
                let resume = job.automatic.as_ref().is_some_and(|origin| {
                    origin.resume_intent && current(&inner.catalog, origin, &owner)
                }) && job.channel_id == owner
                    && matches!(job.view.state.as_str(), "paused" | "waiting_auth")
                    && job.view.error.as_ref().is_some_and(|error| {
                        matches!(
                            error.code.as_str(),
                            "CLOUD_RESTART_PAUSED"
                                | "CLOUD_PAUSED"
                                | "CLOUD_AUTH_REQUIRED"
                                | "CLOUD_REAUTH_REQUIRED"
                        )
                    });
                if resume {
                    job.view.state = "queued".into();
                    job.view.error = None;
                    job.view.retry_at = None;
                    changed = true;
                }
            }
            if changed {
                self.publish(&mut inner, catalog)?;
            }
            (admissions, changed)
        };
        if wake_worker {
            self.start_worker()?;
        }
        for admission in admissions {
            let reference = admission.candidate.source.clone();
            self.enqueue_kind(vec![reference], true, Some(admission), CloudVisibility::Private)?;
        }
        Ok(())
    }
}
