//! Declarative provider registry and URL policy.
//!
//! This is the Rust counterpart of PaperReader's provider table.  DOM
//! selectors stay out of the host contract: selectors belong to a future,
//! provider-specific page bridge and are not exposed to remote origins.

use super::models::{WebAiCapabilities, WebAiProvider};
use url::Url;

#[derive(Debug, Clone, Copy)]
pub struct ProviderDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub home_url: &'static str,
    pub origins: &'static [&'static str],
    pub auth_origins: &'static [&'static str],
    pub conversation_marker: &'static str,
    pub capabilities: WebAiCapabilities,
}

const PROVIDERS: &[ProviderDefinition] = &[
    ProviderDefinition {
        id: "chatgpt",
        name: "ChatGPT",
        home_url: "https://chatgpt.com/",
        origins: &["chatgpt.com"],
        auth_origins: &["auth.openai.com"],
        conversation_marker: "/c/",
        capabilities: WebAiCapabilities {
            text: true,
            image: true,
            pdf: true,
            conversation_rename: true,
            projects: true,
            connector: true,
        },
    },
    ProviderDefinition {
        id: "gemini",
        name: "Gemini",
        home_url: "https://gemini.google.com/app",
        origins: &["gemini.google.com"],
        auth_origins: &["accounts.google.com"],
        conversation_marker: "/app/",
        capabilities: WebAiCapabilities {
            text: true,
            image: true,
            pdf: true,
            conversation_rename: false,
            projects: false,
            connector: false,
        },
    },
    ProviderDefinition {
        id: "deepseek",
        name: "DeepSeek",
        home_url: "https://chat.deepseek.com/",
        origins: &["chat.deepseek.com"],
        auth_origins: &["chat.deepseek.com"],
        conversation_marker: "/a/chat/s/",
        capabilities: WebAiCapabilities {
            text: true,
            image: true,
            pdf: false,
            conversation_rename: false,
            projects: false,
            connector: false,
        },
    },
    ProviderDefinition {
        id: "kimi",
        name: "Kimi",
        home_url: "https://www.kimi.com/",
        origins: &["www.kimi.com", "kimi.com"],
        auth_origins: &["www.kimi.com", "kimi.com", "accounts.kimi.com"],
        conversation_marker: "/chat/",
        capabilities: WebAiCapabilities {
            text: true,
            image: true,
            pdf: true,
            conversation_rename: false,
            projects: false,
            connector: false,
        },
    },
    ProviderDefinition {
        id: "glm",
        name: "GLM / Z.ai",
        home_url: "https://chat.z.ai/",
        origins: &["chat.z.ai"],
        auth_origins: &["chat.z.ai", "open.bigmodel.cn"],
        conversation_marker: "/c/",
        capabilities: WebAiCapabilities {
            text: true,
            image: true,
            pdf: false,
            conversation_rename: false,
            projects: false,
            connector: false,
        },
    },
];

/// Return the canonical provider id if it is registered.
pub fn normalize_provider_id(value: &str) -> Option<&'static str> {
    let id = value.trim();
    PROVIDERS
        .iter()
        .find(|provider| provider.id.eq_ignore_ascii_case(id))
        .map(|provider| provider.id)
}

/// Find a registered provider by id.
pub fn definition(id: &str) -> Option<&'static ProviderDefinition> {
    let id = normalize_provider_id(id)?;
    PROVIDERS.iter().find(|provider| provider.id == id)
}

/// Provider metadata safe to expose to the renderer.
pub fn list() -> Vec<WebAiProvider> {
    PROVIDERS
        .iter()
        .map(|provider| WebAiProvider {
            id: provider.id.to_string(),
            name: provider.name.to_string(),
            home_url: provider.home_url.to_string(),
            origins: provider
                .origins
                .iter()
                .map(|origin| origin.to_string())
                .collect(),
            capabilities: provider.capabilities,
        })
        .collect()
}

fn parse_url(value: &str) -> Option<Url> {
    let value = value.trim();
    let url = Url::parse(value).ok()?;
    // `url::Url` normalizes an explicit default port (`:443`) away.  The
    // provider policy still rejects every caller-supplied port, so inspect the
    // authority before relying on `Url::port()`.
    let authority = value
        .strip_prefix("https://")?
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if authority.rsplit_once(':').is_some() {
        return None;
    }
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    Some(url)
}

/// Return true when a URL belongs to the registered provider origin.
pub fn is_provider_url(provider_id: &str, value: &str) -> bool {
    let Some(provider) = definition(provider_id) else {
        return false;
    };
    let Some(url) = parse_url(value) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    provider
        .origins
        .iter()
        .any(|origin| host.eq_ignore_ascii_case(origin))
}

pub fn is_provider_navigation_url(provider_id: &str, value: &str) -> bool {
    let Some(provider) = definition(provider_id) else {
        return false;
    };
    let Ok(url) = Url::parse(value.trim()) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    provider
        .origins
        .iter()
        .chain(provider.auth_origins.iter())
        .any(|origin| host.eq_ignore_ascii_case(origin))
}

fn valid_conversation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 300
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-'))
}

/// Extract the stable conversation id from a provider URL.
pub fn conversation_id_from_url(provider_id: &str, value: &str) -> Option<String> {
    let provider = definition(provider_id)?;
    let url = parse_url(value)?;
    let host = url.host_str()?;
    if !provider
        .origins
        .iter()
        .any(|origin| host.eq_ignore_ascii_case(origin))
    {
        return None;
    }
    let pathname = url.path().trim_end_matches('/');
    let rest = pathname.strip_prefix(provider.conversation_marker)?;
    let mut segments = rest.split('/');
    let id = segments.next()?.trim();
    if segments.next().is_some() || !valid_conversation_id(id) {
        return None;
    }
    Some(id.to_string())
}

/// Canonical URL used for persisted bindings.  Query and fragment state is
/// intentionally dropped because it must not select a different paper chat.
pub fn canonical_conversation_url(provider_id: &str, value: &str) -> Option<String> {
    let mut url = parse_url(value)?;
    let conversation_id = conversation_id_from_url(provider_id, value)?;
    url.set_query(None);
    url.set_fragment(None);
    let mut canonical = url.to_string();
    if canonical.ends_with('/') && !url.path().ends_with("/") {
        canonical.pop();
    }
    // Keep this check next to canonicalization so a future route change cannot
    // accidentally persist a URL without the same validated id.
    conversation_id_from_url(provider_id, &canonical).filter(|id| id == &conversation_id)?;
    Some(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_paper_reader_providers() {
        let ids: Vec<_> = list().into_iter().map(|provider| provider.id).collect();
        assert_eq!(ids, ["chatgpt", "gemini", "deepseek", "kimi", "glm"]);
    }

    #[test]
    fn provider_urls_require_https_origin_without_credentials() {
        assert!(is_provider_url("chatgpt", "https://chatgpt.com/"));
        assert!(is_provider_url("kimi", "https://KIMI.com/chat/abc"));
        assert!(!is_provider_url("chatgpt", "http://chatgpt.com/"));
        assert!(!is_provider_url(
            "chatgpt",
            "https://chatgpt.com.evil.test/"
        ));
        assert!(!is_provider_url(
            "chatgpt",
            "https://user:pass@chatgpt.com/"
        ));
        assert!(!is_provider_url("chatgpt", "https://chatgpt.com:443/"));
    }

    #[test]
    fn conversation_ids_are_route_scoped_and_canonical() {
        assert_eq!(
            conversation_id_from_url("chatgpt", "https://chatgpt.com/c/abc_123?x=1#top"),
            Some("abc_123".to_string())
        );
        assert_eq!(
            canonical_conversation_url("chatgpt", "https://chatgpt.com/c/abc_123?x=1#top"),
            Some("https://chatgpt.com/c/abc_123".to_string())
        );
        assert_eq!(
            conversation_id_from_url("gemini", "https://gemini.google.com/app/abc/extra"),
            None
        );
        assert_eq!(
            conversation_id_from_url("chatgpt", "https://chatgpt.com/c/a%2Fb"),
            None
        );
    }

    #[test]
    fn authentication_origins_are_allowed_only_for_navigation() {
        assert!(is_provider_navigation_url(
            "chatgpt",
            "https://auth.openai.com/login"
        ));
        assert!(!is_provider_url("chatgpt", "https://auth.openai.com/login"));
        assert!(!is_provider_navigation_url(
            "chatgpt",
            "https://evil.example/login"
        ));
    }
}
