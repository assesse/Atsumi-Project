use serde::{Deserialize, Serialize};

use super::ValidationError;

pub const EXPLORATION_DATA_RESET_CONFIRMATION: &str = "RESET_EXPLORATION_DATA";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplorationDataResetRequest {
    pub confirmation: String,
}

impl ExplorationDataResetRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.confirmation == EXPLORATION_DATA_RESET_CONFIRMATION {
            Ok(())
        } else {
            Err(ValidationError::new(
                "confirmation",
                "must explicitly confirm exploration data reset",
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationDataResetResult {
    pub favorites_removed: u64,
    pub search_history_removed: u64,
    pub auto_find_runs_removed: u64,
    pub auto_find_candidates_removed: u64,
    pub auto_find_exclusions_removed: u64,
}

pub const FACTORY_RESET_CONFIRMATION: &str = "RESET_ALL_APP_DATA";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MaintenanceAction {
    QuickRepair,
    RebuildLibrary {
        rebuild_thumbnail_data: bool,
        rebuild_duplicate_analysis: bool,
        rebuild_internal_analysis: bool,
        rebuild_auto_find_results: bool,
    },
    FactoryReset {
        confirmation: String,
    },
}

impl MaintenanceAction {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Self::FactoryReset { confirmation } = self {
            if confirmation != FACTORY_RESET_CONFIRMATION {
                return Err(ValidationError::new(
                    "confirmation",
                    "must explicitly confirm complete app data reset",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePreview {
    pub preview_id: String,
    pub action: MaintenanceAction,
    pub original_files_deleted: bool,
    pub user_decisions_preserved: bool,
    pub restart_required: bool,
    pub warnings: Vec<String>,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceResult {
    pub action: MaintenanceAction,
    pub completed_steps: Vec<String>,
    pub warnings: Vec<String>,
    pub restart_required: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_reset_requires_the_exact_confirmation() {
        assert!(MaintenanceAction::FactoryReset {
            confirmation: FACTORY_RESET_CONFIRMATION.into(),
        }
        .validate()
        .is_ok());
        assert!(MaintenanceAction::FactoryReset {
            confirmation: "RESET_EXPLORATION_DATA".into(),
        }
        .validate()
        .is_err());
    }
}
