//! Wire models for the login-preserving web AI host.
//!
//! The first implementation intentionally stops at the host contract.  A
//! provider page is still a remote origin and must not receive a broad Tauri
//! capability.  The future WebView controller can consume these models after
//! its identity and navigation checks are in place.

use serde::{Deserialize, Serialize};

/// A provider capability exposed to the renderer.
///
/// These are feature declarations, not proof that the current page is logged
/// in or that a particular DOM revision is available.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiCapabilities {
    pub text: bool,
    pub image: bool,
    pub pdf: bool,
    pub conversation_rename: bool,
    pub projects: bool,
    pub connector: bool,
}

/// Public provider metadata.  It deliberately contains no cookies, tokens,
/// selectors, or browser storage paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiProvider {
    pub id: String,
    pub name: String,
    pub home_url: String,
    pub origins: Vec<String>,
    pub capabilities: WebAiCapabilities,
}

/// A persisted paper/provider conversation binding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiConversationBinding {
    pub provider_id: String,
    pub vault_id: String,
    pub paper_id: String,
    pub conversation_url: String,
    pub conversation_id: String,
    pub title: Option<String>,
    pub updated_at: String,
}

/// Host status returned to the renderer.  `view` is intentionally a small
/// state vocabulary until a platform WebView controller is added.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiStatus {
    pub view: String,
    pub active_provider_id: Option<String>,
    pub active_vault_id: Option<String>,
    pub active_paper_id: Option<String>,
    pub url: Option<String>,
    pub conversation_id: Option<String>,
    pub authenticated: String,
    pub bridge_version: Option<String>,
    pub fallback: Option<String>,
    pub bindings: Vec<WebAiConversationBinding>,
}

/// Result shared by text/image/PDF transfer adapters.
///
/// `requires_send` is always true for web AI: host-side preparation must never
/// click a provider's send button.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiTransferResult {
    pub provider_id: String,
    pub draft_ready: bool,
    pub attachment_ready: bool,
    pub requires_send: bool,
    pub message: Option<String>,
    pub paper_id: Option<String>,
    pub page: Option<u32>,
}

/// Stable paper identity used by all provider-scoped commands.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiPaperRef {
    pub vault_id: String,
    pub paper_id: String,
}
