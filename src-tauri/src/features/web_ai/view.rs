//! Platform-neutral view state and geometry helpers.

use super::models::WebAiStatus;
use agentero_core::paths::web_ai_profiles_dir;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
}

impl WebAiBounds {
    pub fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
            && self.width <= 32_000.0
            && self.height <= 32_000.0
            && self.scale_factor.is_finite()
            && self.scale_factor > 0.0
            && self.scale_factor <= 8.0
    }
}

pub fn profile_dir(provider_id: &str) -> PathBuf {
    web_ai_profiles_dir().join(provider_id)
}

pub fn initial_status(provider_id: &str) -> WebAiStatus {
    WebAiStatus {
        view: "closed".into(),
        active_provider_id: Some(provider_id.to_string()),
        active_vault_id: None,
        active_paper_id: None,
        url: None,
        conversation_id: None,
        authenticated: "unknown".into(),
        bridge_version: None,
        fallback: None,
        bindings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_reject_nan_and_unbounded_values() {
        assert!(!WebAiBounds {
            x: 0.0,
            y: 0.0,
            width: f64::NAN,
            height: 10.0,
            scale_factor: 1.0,
        }
        .is_valid());
        assert!(!WebAiBounds {
            x: 0.0,
            y: 0.0,
            width: 40_000.0,
            height: 10.0,
            scale_factor: 1.0,
        }
        .is_valid());
        assert!(WebAiBounds {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            scale_factor: 1.0,
        }
        .is_valid());
    }
}
