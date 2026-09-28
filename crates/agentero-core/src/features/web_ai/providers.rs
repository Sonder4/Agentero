use super::{ProviderCapabilities, ProviderId, WebAiProviderInfo};
use url::Url;

#[derive(Debug, Clone, Copy)]
pub struct ProviderSpec {
    pub id: ProviderId,
    pub display_name: &'static str,
    pub homepage: &'static str,
    pub allowed_origins: &'static [&'static str],
    pub auth_origins: &'static [&'static str],
    pub conversation_markers: &'static [&'static str],
    pub composer_selectors: &'static [&'static str],
    pub attachment_selectors: &'static [&'static str],
    pub capabilities: ProviderCapabilities,
}

const ALL_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    image: true,
    pdf: true,
    projects: true,
    connector: false,
};

const CHATGPT_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    connector: true,
    ..ALL_CAPABILITIES
};

const CHATGPT: ProviderSpec = ProviderSpec {
    id: ProviderId::Chatgpt,
    display_name: "ChatGPT",
    homepage: "https://chatgpt.com/",
    allowed_origins: &["https://chatgpt.com", "https://chat.openai.com"],
    auth_origins: &["https://auth.openai.com"],
    conversation_markers: &["/c/", "/share/", "/g/"],
    composer_selectors: &["#prompt-textarea", "textarea", "[contenteditable='true']"],
    attachment_selectors: &["input[type='file']", "[data-testid*='attachment']"],
    capabilities: CHATGPT_CAPABILITIES,
};

const GEMINI: ProviderSpec = ProviderSpec {
    id: ProviderId::Gemini,
    display_name: "Gemini",
    homepage: "https://gemini.google.com/",
    allowed_origins: &["https://gemini.google.com"],
    auth_origins: &["https://accounts.google.com"],
    conversation_markers: &["/app/"],
    composer_selectors: &["[contenteditable='true']", "textarea"],
    attachment_selectors: &["input[type='file']", "[aria-label*='Upload']"],
    capabilities: ALL_CAPABILITIES,
};

const DEEPSEEK: ProviderSpec = ProviderSpec {
    id: ProviderId::Deepseek,
    display_name: "DeepSeek",
    homepage: "https://chat.deepseek.com/",
    allowed_origins: &["https://chat.deepseek.com"],
    auth_origins: &["https://chat.deepseek.com"],
    conversation_markers: &["/a/chat/", "/chat/"],
    composer_selectors: &["textarea", "[contenteditable='true']"],
    attachment_selectors: &["input[type='file']"],
    capabilities: ALL_CAPABILITIES,
};

const KIMI: ProviderSpec = ProviderSpec {
    id: ProviderId::Kimi,
    display_name: "Kimi",
    homepage: "https://www.kimi.com/",
    allowed_origins: &["https://www.kimi.com", "https://kimi.moonshot.cn"],
    auth_origins: &["https://www.kimi.com", "https://kimi.moonshot.cn"],
    conversation_markers: &["/chat/", "/share/"],
    composer_selectors: &["textarea", "[contenteditable='true']"],
    attachment_selectors: &["input[type='file']"],
    capabilities: ALL_CAPABILITIES,
};

const GLM: ProviderSpec = ProviderSpec {
    id: ProviderId::Glm,
    display_name: "GLM",
    homepage: "https://chatglm.cn/",
    allowed_origins: &["https://chatglm.cn", "https://chatglm.cn/main"],
    auth_origins: &["https://chatglm.cn"],
    conversation_markers: &["/main/detail/", "/chat/"],
    composer_selectors: &["textarea", "[contenteditable='true']"],
    attachment_selectors: &["input[type='file']"],
    capabilities: ALL_CAPABILITIES,
};

pub const fn providers() -> &'static [ProviderSpec] {
    &[CHATGPT, GEMINI, DEEPSEEK, KIMI, GLM]
}

pub fn provider(id: ProviderId) -> &'static ProviderSpec {
    providers()
        .iter()
        .find(|spec| spec.id == id)
        .expect("all ProviderId variants are registered")
}

impl ProviderSpec {
    pub fn info(&self) -> WebAiProviderInfo {
        WebAiProviderInfo {
            id: self.id,
            display_name: self.display_name.to_string(),
            homepage: self.homepage.to_string(),
            capabilities: self.capabilities,
        }
    }

    pub fn accepts_url(&self, url: &Url) -> bool {
        let origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
        self.allowed_origins
            .iter()
            .chain(self.auth_origins.iter())
            .any(|candidate| candidate.trim_end_matches('/') == origin)
    }

    pub fn canonical_conversation_url(&self, url: &Url) -> Option<String> {
        if !self.accepts_url(url)
            || !self
                .conversation_markers
                .iter()
                .any(|marker| url.path().contains(marker))
        {
            return None;
        }
        let mut canonical = url.clone();
        canonical.set_query(None);
        canonical.set_fragment(None);
        Some(canonical.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_providers_are_registered() {
        assert_eq!(providers().len(), ProviderId::ALL.len());
        for id in ProviderId::ALL {
            assert_eq!(provider(id).id, id);
        }
    }

    #[test]
    fn navigation_is_origin_and_path_restricted() {
        let spec = provider(ProviderId::Chatgpt);
        assert!(spec.accepts_url(&Url::parse("https://chatgpt.com/c/abc").unwrap()));
        assert!(spec.accepts_url(&Url::parse("https://auth.openai.com/log-in").unwrap()));
        assert!(!spec.accepts_url(&Url::parse("https://evil.example/c/abc").unwrap()));
        assert_eq!(
            spec.canonical_conversation_url(
                &Url::parse("https://chatgpt.com/c/abc?utm_source=test#x").unwrap()
            )
            .as_deref(),
            Some("https://chatgpt.com/c/abc")
        );
    }
}
