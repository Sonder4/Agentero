//! Tauri-free models, provider registry, storage and validation for Web AI.
//!
//! The desktop shell owns WebViews and DOM bridges. This module deliberately
//! keeps provider/session metadata and file validation independent of Tauri so
//! it can be exercised by unit tests and reused by a future headless host.

mod files;
mod providers;
mod storage;

pub use files::{prepare_attachment, AttachmentKind, PreparedAttachment, MAX_PDF_BYTES};
pub use providers::{provider, providers, ProviderSpec};
pub use storage::{Binding, WebAiStore, WEB_AI_SCHEMA_VERSION};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Stable provider identifiers used on the Rust/TypeScript boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderId {
    Chatgpt,
    Gemini,
    Deepseek,
    Kimi,
    Glm,
}

impl ProviderId {
    pub const ALL: [Self; 5] = [
        Self::Chatgpt,
        Self::Gemini,
        Self::Deepseek,
        Self::Kimi,
        Self::Glm,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chatgpt => "chatgpt",
            Self::Gemini => "gemini",
            Self::Deepseek => "deepseek",
            Self::Kimi => "kimi",
            Self::Glm => "glm",
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str((*self).as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub image: bool,
    pub pdf: bool,
    pub projects: bool,
    pub connector: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum WebAiViewState {
    Closed,
    Opening,
    Ready,
    Fallback,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AuthState {
    Unknown,
    LoggedIn,
    LoggedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum WebAiFallback {
    WebviewWindow,
    SystemBrowser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebAiProviderInfo {
    pub id: ProviderId,
    pub display_name: String,
    pub homepage: String,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiStatus {
    pub provider_id: ProviderId,
    pub view_state: WebAiViewState,
    pub url: Option<String>,
    pub conversation_id: Option<String>,
    pub bound_paper_id: Option<String>,
    pub authenticated: AuthState,
    pub bridge_version: Option<String>,
    pub fallback: Option<WebAiFallback>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiTransferResult {
    pub provider_id: ProviderId,
    pub draft_ready: bool,
    pub attachment_ready: bool,
    pub requires_send: bool,
    pub message: Option<String>,
    pub paper_id: Option<String>,
    pub page: Option<u32>,
}

impl WebAiTransferResult {
    pub fn prepared(provider_id: ProviderId, paper_id: Option<String>, page: Option<u32>) -> Self {
        Self {
            provider_id,
            draft_ready: true,
            attachment_ready: false,
            requires_send: true,
            message: None,
            paper_id,
            page,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiSelectionEvent {
    pub provider_id: ProviderId,
    pub selection_id: String,
    pub page: Option<u32>,
    pub byte_length: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_are_stable_wire_values() {
        assert_eq!(
            ProviderId::ALL.map(ProviderId::as_str),
            ["chatgpt", "gemini", "deepseek", "kimi", "glm"]
        );
        assert_eq!(
            serde_json::to_string(&ProviderId::Chatgpt).unwrap(),
            "\"chatgpt\""
        );
    }

    #[test]
    fn transfer_result_never_requests_automatic_send() {
        assert!(WebAiTransferResult::prepared(ProviderId::Gemini, None, None).requires_send);
    }
}
