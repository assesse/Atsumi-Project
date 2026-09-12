pub mod browser;
pub mod browser_extension;
#[cfg(test)]
mod browser_ipc_tests;
pub mod browser_merge;
pub mod browser_store;
pub mod chat;
pub mod chat_assets;
pub mod chat_store;
pub mod commands;
pub mod model;
pub mod provider;
pub mod replay;
pub mod replay_assets;

impl From<model::StreamError> for crate::interface::ApiError {
    fn from(error: model::StreamError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
            action: Some(if error.retryable {
                crate::interface::ApiAction::Retry
            } else {
                crate::interface::ApiAction::None
            }),
            details: None,
        }
    }
}
