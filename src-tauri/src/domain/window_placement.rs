use serde::{Deserialize, Serialize};

use super::ValidationError;

pub const DEFAULT_WINDOW_WIDTH: u32 = 1_280;
pub const DEFAULT_WINDOW_HEIGHT: u32 = 820;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPlacementSnapshot {
    pub revision: u64,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for WindowPlacementSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            x: None,
            y: None,
            width: DEFAULT_WINDOW_WIDTH,
            height: DEFAULT_WINDOW_HEIGHT,
            maximized: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowPlacement {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl WindowPlacementSnapshot {
    pub fn updated(&self, placement: WindowPlacement) -> Result<Self, ValidationError> {
        if placement.width == 0 {
            return Err(ValidationError::new("width", "must be greater than zero"));
        }
        if placement.height == 0 {
            return Err(ValidationError::new("height", "must be greater than zero"));
        }
        if placement.width > 32_768 {
            return Err(ValidationError::new("width", "must be at most 32768"));
        }
        if placement.height > 32_768 {
            return Err(ValidationError::new("height", "must be at most 32768"));
        }

        Ok(Self {
            revision: self
                .revision
                .checked_add(1)
                .ok_or_else(|| ValidationError::new("revision", "cannot be incremented"))?,
            x: placement.x,
            y: placement.y,
            width: placement.width,
            height: placement.height,
            maximized: placement.maximized,
        })
    }
}
