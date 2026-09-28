//! Provider page bridge protocol.
//!
//! The remote page only receives a small initialization script and can send
//! metadata events through one narrow command.  This module validates the
//! wire envelope before the controller interprets any provider-specific DOM
//! event.

use serde::{Deserialize, Serialize};

pub const BRIDGE_VERSION: &str = "1";
pub const MAX_EVENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageEvent {
    pub provider_id: String,
    pub nonce: String,
    pub kind: PageEventKind,
    pub payload: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum PageEventKind {
    Handshake,
    Navigation,
    Selection,
    ComposerState,
    AttachmentState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BridgeHandshake {
    pub version: String,
    pub provider_id: String,
    pub nonce: String,
}

pub fn validate_event(event: &PageEvent, expected_provider: &str, expected_nonce: &str) -> bool {
    let size_ok = serde_json::to_vec(event)
        .map(|bytes| bytes.len() <= MAX_EVENT_BYTES)
        .unwrap_or(false);
    size_ok
        && event.provider_id == expected_provider
        && !expected_nonce.is_empty()
        && event.nonce == expected_nonce
        && matches!(
            event.kind,
            PageEventKind::Handshake
                | PageEventKind::Navigation
                | PageEventKind::Selection
                | PageEventKind::ComposerState
                | PageEventKind::AttachmentState
        )
}

/// Safe host-to-page bridge bootstrap. The controller injects this only into a
/// validated provider WebView. It appends drafts and prepares file inputs, and
/// it never clicks a send or submit control.
pub fn bootstrap_script(
    provider_id: &str,
    nonce: &str,
    composer_selectors: &[&str],
    attachment_selectors: &[&str],
) -> String {
    let provider = serde_json::to_string(provider_id).unwrap_or_else(|_| "\"\"".into());
    let nonce = serde_json::to_string(nonce).unwrap_or_else(|_| "\"\"".into());
    let composers = serde_json::to_string(composer_selectors).unwrap_or_else(|_| "[]".into());
    let attachments = serde_json::to_string(attachment_selectors).unwrap_or_else(|_| "[]".into());
    format!(
        r#"(() => {{
          const providerId = {provider};
          const nonce = {nonce};
          const composerSelectors = {composers};
          const attachmentSelectors = {attachments};
          const bridgeVersion = "{BRIDGE_VERSION}";
          const first = (selectors) => {{
            for (const selector of selectors) {{
              const node = document.querySelector(selector);
              if (node) return node;
            }}
            return null;
          }};
          window.__AGENTERO_WEB_AI__ = Object.freeze({{
            version: bridgeVersion,
            providerId,
            nonce,
            appendText(text) {{
              if (typeof text !== "string" || text.length > 1_000_000) throw new Error("invalid text");
              const editor = first(composerSelectors);
              if (!editor) throw new Error("composer not found");
              editor.focus();
              if (editor instanceof HTMLTextAreaElement || editor instanceof HTMLInputElement) {{
                const prototype = editor instanceof HTMLTextAreaElement
                  ? HTMLTextAreaElement.prototype
                  : HTMLInputElement.prototype;
                const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
                setter?.call(editor, editor.value + text);
                editor.dispatchEvent(new Event("input", {{ bubbles: true }}));
              }} else {{
                document.execCommand("insertText", false, text);
              }}
              return true;
            }},
            beginAttachment(name, mime, size, sha256) {{
              if (typeof name !== "string" || typeof mime !== "string" ||
                  !Number.isSafeInteger(size) || size <= 0 || typeof sha256 !== "string")
                throw new Error("invalid attachment metadata");
              window.__AGENTERO_WEB_AI_PENDING__ = {{ name, mime, size, sha256, chunks: [] }};
            }},
            appendAttachmentChunk(chunk) {{
              const pending = window.__AGENTERO_WEB_AI_PENDING__;
              if (!pending || typeof chunk !== "string" || chunk.length > 1_500_000)
                throw new Error("attachment is not started");
              pending.chunks.push(chunk);
            }},
            finishAttachment() {{
              const pending = window.__AGENTERO_WEB_AI_PENDING__;
              const input = first(attachmentSelectors);
              if (!pending || !(input instanceof HTMLInputElement)) throw new Error("attachment input not found");
              const parts = pending.chunks.map((part) => {{
                const raw = atob(part);
                const bytes = new Uint8Array(raw.length);
                for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
                return bytes;
              }});
              const file = new File(parts, pending.name, {{ type: pending.mime }});
              const transfer = new DataTransfer();
              transfer.items.add(file);
              input.files = transfer.files;
              if (!input.files || input.files.length !== 1) throw new Error("attachment was rejected");
              input.dispatchEvent(new Event("input", {{ bubbles: true }}));
              input.dispatchEvent(new Event("change", {{ bubbles: true }}));
              delete window.__AGENTERO_WEB_AI_PENDING__;
              return true;
            }}
          }});
        }})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_requires_matching_provider_and_nonce() {
        let event = PageEvent {
            provider_id: "chatgpt".into(),
            nonce: "nonce".into(),
            kind: PageEventKind::Navigation,
            payload: None,
        };
        assert!(validate_event(&event, "chatgpt", "nonce"));
        assert!(!validate_event(&event, "gemini", "nonce"));
        assert!(!validate_event(&event, "chatgpt", "other"));
    }

    #[test]
    fn bootstrap_does_not_embed_unescaped_provider_values() {
        let script = bootstrap_script(
            "chatgpt",
            "nonce-1",
            &["#prompt-textarea"],
            &["input[type='file']"],
        );
        assert!(script.contains("#prompt-textarea"));
        assert!(script.contains("chatgpt"));
        assert!(script.contains("nonce-1"));
        assert!(!script.contains("window.__TAURI_INTERNALS__"));
        let lower = script.to_ascii_lowercase();
        assert!(!lower.contains(".click("));
        assert!(!lower.contains("submit"));
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let event = PageEvent {
            provider_id: "chatgpt".into(),
            nonce: "nonce".into(),
            kind: PageEventKind::Navigation,
            payload: Some("x".repeat(MAX_EVENT_BYTES)),
        };
        assert!(!validate_event(&event, "chatgpt", "nonce"));
    }
}
