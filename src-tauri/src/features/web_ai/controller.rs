//! Desktop WebView lifecycle for provider pages.

use super::bridge::{bootstrap_script, validate_event, PageEvent, PageEventKind, BRIDGE_VERSION};
use super::models::WebAiStatus;
use super::providers;
use super::view::{profile_dir, WebAiBounds};
use agentero_core::features::web_ai::PreparedAttachment;
use agentero_core::features::web_ai::WebAiStore;
use base64::Engine;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::Mutex;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, Webview, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, Wry,
};
use url::Url;
use uuid::Uuid;

enum ManagedView {
    Child(Box<Webview<Wry>>),
    Window(Box<WebviewWindow<Wry>>),
}

impl ManagedView {
    fn webview(&self) -> &Webview<Wry> {
        match self {
            Self::Child(view) => view.as_ref(),
            Self::Window(window) => window.as_ref().as_ref(),
        }
    }

    fn close(&self) -> tauri::Result<()> {
        self.webview().close()
    }

    fn hide(&self) -> tauri::Result<()> {
        self.webview().hide()
    }

    fn show(&self) -> tauri::Result<()> {
        self.webview().show()
    }

    fn set_bounds(&self, bounds: PhysicalRect<i32, u32>) -> tauri::Result<()> {
        self.webview().set_bounds(tauri::Rect {
            position: tauri::Position::Physical(bounds.position),
            size: tauri::Size::Physical(bounds.size),
        })
    }

    fn eval(&self, script: String) -> tauri::Result<()> {
        self.webview().eval(script)
    }
}

struct ViewEntry {
    provider_id: String,
    nonce: String,
    view: ManagedView,
}

struct ControllerState {
    views: HashMap<String, ViewEntry>,
    statuses: HashMap<String, WebAiStatus>,
}

pub struct WebAiController {
    state: Mutex<ControllerState>,
    store: WebAiStore,
}

impl WebAiController {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ControllerState {
                views: HashMap::new(),
                statuses: HashMap::new(),
            }),
            store: WebAiStore::default(),
        }
    }

    pub fn store(&self) -> &WebAiStore {
        &self.store
    }

    pub fn statuses(&self, provider_id: Option<&str>) -> Vec<WebAiStatus> {
        let Ok(state) = self.state.lock() else {
            return Vec::new();
        };
        match provider_id {
            Some(id) => state.statuses.get(id).cloned().into_iter().collect(),
            None => providers::list()
                .into_iter()
                .map(|provider| {
                    state
                        .statuses
                        .get(&provider.id)
                        .cloned()
                        .unwrap_or(WebAiStatus {
                            view: "closed".into(),
                            active_provider_id: Some(provider.id),
                            active_vault_id: None,
                            active_paper_id: None,
                            url: None,
                            conversation_id: None,
                            authenticated: "unknown".into(),
                            bridge_version: None,
                            fallback: None,
                            bindings: Vec::new(),
                        })
                })
                .collect(),
        }
    }

    pub async fn open(
        self: &std::sync::Arc<Self>,
        app: &AppHandle,
        provider_id: &str,
        bounds: Option<WebAiBounds>,
    ) -> Result<WebAiStatus, String> {
        let provider = providers::definition(provider_id)
            .ok_or_else(|| "unknown web AI provider".to_string())?;
        let id = provider.id.to_string();
        if let Some(bounds) = bounds {
            if !bounds.is_valid() {
                return Err("invalid WebView bounds".into());
            }
        }

        {
            let state = self.state.lock().map_err(|_| "web AI state poisoned")?;
            if let Some(entry) = state.views.get(&id) {
                entry.view.show().map_err(|e| e.to_string())?;
                if let Some(bounds) = bounds {
                    entry
                        .view
                        .set_bounds(to_physical_rect(bounds))
                        .map_err(|e| e.to_string())?;
                }
                return Ok(state
                    .statuses
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| closed_status(&id)));
            }
        }

        let main = host_window(app).ok_or_else(|| "main window is unavailable".to_string())?;
        let nonce = Uuid::new_v4().to_string();
        let label = format!("agentero-web-ai-{id}");
        let url = provider
            .home_url
            .parse()
            .map_err(|_| "invalid provider URL")?;
        let profile = profile_dir(&id);
        std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
        let navigation_id = id.clone();
        let navigation_controller = std::sync::Arc::clone(self);
        let navigation_app = app.clone();
        let navigation_nonce = nonce.clone();
        let script = bootstrap_script(
            &id,
            &nonce,
            provider.composer_selectors,
            provider.attachment_selectors,
        );
        let popup_app = app.clone();
        let popup_id = id.clone();
        let builder = tauri::WebviewBuilder::new(label.clone(), WebviewUrl::External(url))
            .data_directory(profile.clone())
            .focused(false)
            .initialization_script(script.clone())
            .on_navigation(move |url| {
                handle_navigation(
                    &navigation_app,
                    &navigation_controller,
                    &navigation_id,
                    &navigation_nonce,
                    url,
                )
            })
            .on_new_window(move |url, features| {
                open_auth_popup(&popup_app, &popup_id, url, features)
            });

        let initial_bounds = bounds.unwrap_or(WebAiBounds {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        });
        let rect = to_physical_rect(initial_bounds);
        let (managed, fallback) = match main.as_ref().window().add_child(
            builder,
            rect.position,
            rect.size,
        ) {
            Ok(child) => (ManagedView::Child(Box::new(child)), None),
            Err(error) => {
                log::warn!(target: "agentero::web_ai", "child WebView failed for {id}: {error}");
                let fallback_label = format!("web-ai-window-{id}");
                let fallback_url = provider
                    .home_url
                    .parse()
                    .map_err(|_| "invalid provider URL")?;
                let navigation_id = id.clone();
                let navigation_controller = std::sync::Arc::clone(self);
                let navigation_app = app.clone();
                let navigation_nonce = nonce.clone();
                let popup_app = app.clone();
                let popup_id = id.clone();
                let window =
                    WebviewWindow::builder(app, fallback_label, WebviewUrl::External(fallback_url))
                        .data_directory(profile)
                        .initialization_script(bootstrap_script(
                            &id,
                            &nonce,
                            provider.composer_selectors,
                            provider.attachment_selectors,
                        ))
                        .on_navigation(move |url| {
                            handle_navigation(
                                &navigation_app,
                                &navigation_controller,
                                &navigation_id,
                                &navigation_nonce,
                                url,
                            )
                        })
                        .on_new_window(move |url, features| {
                            open_auth_popup(&popup_app, &popup_id, url, features)
                        })
                        .build()
                        .map_err(|e| e.to_string())?;
                (
                    ManagedView::Window(Box::new(window)),
                    Some("webview-window".to_string()),
                )
            }
        };

        let status = WebAiStatus {
            view: "ready".into(),
            active_provider_id: Some(id.clone()),
            active_vault_id: None,
            active_paper_id: None,
            url: Some(provider.home_url.into()),
            conversation_id: None,
            authenticated: "unknown".into(),
            bridge_version: Some(BRIDGE_VERSION.into()),
            fallback,
            bindings: Vec::new(),
        };
        let mut state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        state.views.insert(
            id.clone(),
            ViewEntry {
                provider_id: id.clone(),
                nonce,
                view: managed,
            },
        );
        state.statuses.insert(id, status.clone());
        let _ = app.emit("web-ai:state", &status);
        Ok(status)
    }

    pub fn close(&self, provider_id: &str) -> Result<bool, String> {
        let mut state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        let Some(entry) = state.views.remove(provider_id) else {
            return Ok(false);
        };
        entry.view.close().map_err(|e| e.to_string())?;
        if let Some(status) = state.statuses.get_mut(provider_id) {
            status.view = "closed".into();
        }
        Ok(true)
    }

    pub fn set_visible(&self, provider_id: &str, visible: bool) -> Result<bool, String> {
        let state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        let Some(entry) = state.views.get(provider_id) else {
            return Ok(false);
        };
        if visible {
            entry.view.show()
        } else {
            entry.view.hide()
        }
        .map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub fn set_bounds(&self, provider_id: &str, bounds: WebAiBounds) -> Result<bool, String> {
        if !bounds.is_valid() {
            return Err("invalid WebView bounds".into());
        }
        let state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        let Some(entry) = state.views.get(provider_id) else {
            return Ok(false);
        };
        entry
            .view
            .set_bounds(to_physical_rect(bounds))
            .map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub fn append_text(&self, provider_id: &str, text: &str) -> Result<bool, String> {
        if text.is_empty() || text.len() > 1_000_000 {
            return Err("text is empty or too large".into());
        }
        let state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        let Some(entry) = state.views.get(provider_id) else {
            return Ok(false);
        };
        let encoded = serde_json::to_string(text).map_err(|e| e.to_string())?;
        entry
            .view
            .eval(format!(
                "window.__AGENTERO_WEB_AI__?.appendText({encoded});"
            ))
            .map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub fn attach_file(
        &self,
        provider_id: &str,
        attachment: &PreparedAttachment,
    ) -> Result<bool, String> {
        let state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        let Some(entry) = state.views.get(provider_id) else {
            return Ok(false);
        };
        let name = attachment
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment.bin");
        let mime = match attachment.kind {
            agentero_core::features::web_ai::AttachmentKind::Pdf => "application/pdf",
            agentero_core::features::web_ai::AttachmentKind::Image => mime_for_image(name),
        };
        let encoded_name = serde_json::to_string(name).map_err(|e| e.to_string())?;
        let encoded_mime = serde_json::to_string(mime).map_err(|e| e.to_string())?;
        let encoded_hash = serde_json::to_string(&attachment.sha256).map_err(|e| e.to_string())?;
        entry.view.eval(format!(
            "window.__AGENTERO_WEB_AI__?.beginAttachment({encoded_name}, {encoded_mime}, {}, {encoded_hash});",
            attachment.size
        )).map_err(|e| e.to_string())?;

        const CHUNK_SIZE: usize = 256 * 1024;
        let mut file = File::open(&attachment.path).map_err(|e| e.to_string())?;
        let mut chunk = vec![0_u8; CHUNK_SIZE];
        loop {
            let read = file.read(&mut chunk).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            let encoded = base64::engine::general_purpose::STANDARD.encode(&chunk[..read]);
            let encoded = serde_json::to_string(&encoded).map_err(|e| e.to_string())?;
            entry
                .view
                .eval(format!(
                    "window.__AGENTERO_WEB_AI__?.appendAttachmentChunk({encoded});"
                ))
                .map_err(|e| e.to_string())?;
        }
        entry
            .view
            .eval("window.__AGENTERO_WEB_AI__?.finishAttachment();".into())
            .map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub fn validate_page_event(&self, event: &PageEvent) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        let Some(entry) = state.views.get(&event.provider_id) else {
            return false;
        };
        validate_event(event, &entry.provider_id, &entry.nonce)
    }

    pub fn apply_page_event(&self, event: &PageEvent) -> Result<Option<WebAiStatus>, String> {
        let mut state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        let Some(entry) = state.views.get(&event.provider_id) else {
            return Ok(None);
        };
        if !validate_event(event, &entry.provider_id, &entry.nonce) {
            return Ok(None);
        }
        let Some(status) = state.statuses.get_mut(&event.provider_id) else {
            return Ok(None);
        };
        match event.kind {
            super::bridge::PageEventKind::Handshake => {
                status.bridge_version = Some(BRIDGE_VERSION.into());
            }
            super::bridge::PageEventKind::Navigation => {
                if let Some(url) = event
                    .payload
                    .as_deref()
                    .filter(|url| providers::is_provider_url(&event.provider_id, url))
                {
                    status.url = Some(url.to_string());
                    status.conversation_id =
                        providers::conversation_id_from_url(&event.provider_id, url);
                }
            }
            super::bridge::PageEventKind::Selection
            | super::bridge::PageEventKind::ComposerState
            | super::bridge::PageEventKind::AttachmentState => {}
        }
        Ok(Some(status.clone()))
    }
}

impl Default for WebAiController {
    fn default() -> Self {
        Self::new()
    }
}

fn open_auth_popup(
    app: &AppHandle,
    provider_id: &str,
    url: Url,
    features: tauri::webview::NewWindowFeatures,
) -> tauri::webview::NewWindowResponse<Wry> {
    if !providers::is_provider_navigation_url(provider_id, url.as_str()) {
        return tauri::webview::NewWindowResponse::Deny;
    }
    let label = format!("web-ai-auth-{}-{}", provider_id, Uuid::new_v4().simple());
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title("Web AI")
        .inner_size(520.0, 720.0);
    #[cfg(windows)]
    {
        builder = builder.with_environment(features.opener().environment.clone());
    }
    #[cfg(not(windows))]
    {
        let _ = features;
    }
    match builder.build() {
        Ok(window) => tauri::webview::NewWindowResponse::Create { window },
        Err(error) => {
            log::warn!(target: "agentero::web_ai", "auth popup failed for {provider_id}: {error}");
            tauri::webview::NewWindowResponse::Deny
        }
    }
}

fn handle_navigation(
    app: &AppHandle,
    controller: &std::sync::Arc<WebAiController>,
    provider_id: &str,
    nonce: &str,
    url: &Url,
) -> bool {
    let allowed = providers::is_provider_navigation_url(provider_id, url.as_str());
    if !allowed {
        return false;
    }
    let event = PageEvent {
        provider_id: provider_id.to_string(),
        nonce: nonce.to_string(),
        kind: PageEventKind::Navigation,
        payload: Some(url.as_str().to_string()),
    };
    if let Ok(Some(status)) = controller.apply_page_event(&event) {
        let _ = app.emit("web-ai:navigation", &event);
        let _ = app.emit("web-ai:state", &status);
    }
    true
}

fn mime_for_image(name: &str) -> &'static str {
    let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

fn host_window(app: &AppHandle) -> Option<WebviewWindow<Wry>> {
    app.get_webview_window("main")
        .filter(|window| is_web_ai_host_label(window.label()))
        .or_else(|| {
            app.webview_windows()
                .into_values()
                .find(|window| is_web_ai_host_label(window.label()))
        })
}

pub(crate) fn is_web_ai_host_label(label: &str) -> bool {
    label != "settings"
        && !label.starts_with("agentero-web-ai-")
        && !label.starts_with("web-ai-window-")
}

fn closed_status(provider_id: &str) -> WebAiStatus {
    WebAiStatus {
        view: "closed".into(),
        active_provider_id: Some(provider_id.into()),
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

fn to_physical_rect(bounds: WebAiBounds) -> PhysicalRect<i32, u32> {
    let scale = bounds.scale_factor;
    PhysicalRect {
        position: PhysicalPosition::new(
            (bounds.x * scale).round() as i32,
            (bounds.y * scale).round() as i32,
        ),
        size: PhysicalSize::new(
            (bounds.width * scale).max(0.0).round() as u32,
            (bounds.height * scale).max(0.0).round() as u32,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::is_web_ai_host_label;

    #[test]
    fn host_label_accepts_primary_and_secondary_app_windows() {
        assert!(is_web_ai_host_label("main"));
        assert!(is_web_ai_host_label("agentero-1234"));
        assert!(!is_web_ai_host_label("settings"));
        assert!(!is_web_ai_host_label("agentero-web-ai-gemini"));
        assert!(!is_web_ai_host_label("web-ai-window-gemini"));
    }
}
