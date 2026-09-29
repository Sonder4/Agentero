//! Desktop WebView lifecycle for provider pages.

use super::bridge::{bootstrap_script, validate_event, PageEvent, PageEventKind, BRIDGE_VERSION};
use super::models::WebAiStatus;
use super::providers;
use super::view::{profile_dir, WebAiBounds};
use agentero_core::features::web_ai::PreparedAttachment;
use agentero_core::features::web_ai::WebAiStore;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, Webview, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, Window, Wry,
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

    fn eval_value(&self, expression: String) -> Result<bool, String> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.webview()
            .with_webview(move |webview| {
                let result = eval_bool(&webview, &expression);
                let _ = sender.send(result);
            })
            .map_err(|e| e.to_string())?;
        receiver
            .recv_timeout(std::time::Duration::from_secs(8))
            .map_err(|_| "provider page did not answer".to_string())?
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
        let (managed, fallback) = match main.add_child(builder, rect.position, rect.size) {
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
        let shown = entry.view.eval_value(format!(
            "window.__AGENTERO_WEB_AI__?.appendText({encoded}) === true"
        ))?;
        Ok(shown)
    }

    pub fn attach_file(
        &self,
        provider_id: &str,
        attachment: &PreparedAttachment,
    ) -> Result<bool, String> {
        if !providers::supports_pdf(provider_id) {
            return Ok(false);
        }
        let state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        if !state.views.contains_key(provider_id) {
            return Ok(false);
        }
        let path = attachment.path.display().to_string();
        let name = attachment
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment.pdf")
            .to_string();
        drop(state);
        assign_provider_file(self, provider_id, &path)?;
        let encoded_name = serde_json::to_string(&name).map_err(|e| e.to_string())?;
        let expression =
            format!("window.__AGENTERO_WEB_AI__?.attachmentShown({encoded_name}) === true");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            let shown = {
                let state = self.state.lock().map_err(|_| "web AI state poisoned")?;
                let Some(entry) = state.views.get(provider_id) else {
                    return Ok(false);
                };
                entry.view.eval_value(expression.clone())?
            };
            if shown || std::time::Instant::now() >= deadline {
                return Ok(shown);
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    pub fn reveal_file_input(&self, provider_id: &str) -> Result<bool, String> {
        let state = self.state.lock().map_err(|_| "web AI state poisoned")?;
        let Some(entry) = state.views.get(provider_id) else {
            return Ok(false);
        };
        entry
            .view
            .eval_value("window.__AGENTERO_WEB_AI__?.revealFileInput() === true".into())
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
        log::warn!(
            target: "agentero::web_ai",
            "auth popup denied provider={provider_id} host={} path={}",
            url.host_str().unwrap_or("<unknown>"),
            url.path()
        );
        return tauri::webview::NewWindowResponse::Deny;
    }
    let label = format!("web-ai-auth-{}-{}", provider_id, Uuid::new_v4().simple());
    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title("Web AI")
        .inner_size(520.0, 720.0)
        // Tauri copies the opener's WebView2 environment on Windows here.
        // `window_features` also preserves the opener's platform-specific
        // configuration on macOS/Linux instead of creating an unrelated
        // browser context for authentication popups.
        .window_features(features);
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
        log::warn!(
            target: "agentero::web_ai",
            "navigation denied provider={provider_id} host={} path={}",
            url.host_str().unwrap_or("<unknown>"),
            url.path()
        );
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

fn assign_provider_file(
    controller: &WebAiController,
    provider_id: &str,
    path: &str,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let mut last_error = "provider file input is not open".to_string();
    loop {
        let revealed = {
            let state = controller
                .state
                .lock()
                .map_err(|_| "web AI state poisoned")?;
            let Some(entry) = state.views.get(provider_id) else {
                return Err("provider WebView is not open".into());
            };
            entry
                .view
                .eval_value("window.__AGENTERO_WEB_AI__?.revealFileInput() === true".into())
                .unwrap_or(false)
        };
        let assigned = {
            let state = controller
                .state
                .lock()
                .map_err(|_| "web AI state poisoned")?;
            let Some(entry) = state.views.get(provider_id) else {
                return Err("provider WebView is not open".into());
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let file_path = path.to_string();
            entry
                .view
                .webview()
                .with_webview(move |webview| {
                    let _ = sender.send(set_file_input(&webview, &file_path));
                })
                .map_err(|e| e.to_string())?;
            drop(state);
            receiver
                .recv_timeout(std::time::Duration::from_secs(8))
                .map_err(|_| "provider page did not accept the file".to_string())?
        };
        if assigned.is_ok() {
            return Ok(());
        }
        last_error = assigned.err().unwrap_or(last_error);
        if !revealed || std::time::Instant::now() >= deadline {
            return Err(last_error);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn eval_bool(webview: &tauri::webview::PlatformWebview, expression: &str) -> Result<bool, String> {
    #[cfg(windows)]
    {
        cdp_eval_bool(webview, expression)
    }
    #[cfg(not(windows))]
    {
        let _ = (webview, expression);
        Err("confirming a provider attachment is only implemented on Windows".into())
    }
}

fn set_file_input(webview: &tauri::webview::PlatformWebview, path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        cdp_set_file(webview, path)
    }
    #[cfg(not(windows))]
    {
        let _ = (webview, path);
        Err("attaching a PDF is only implemented on Windows".into())
    }
}

#[cfg(windows)]
fn cdp_eval_bool(
    webview: &tauri::webview::PlatformWebview,
    expression: &str,
) -> Result<bool, String> {
    let value = cdp_call(
        webview,
        "Runtime.evaluate",
        &serde_json::json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true,
        }),
    )?;
    Ok(value
        .get("result")
        .and_then(|result| result.get("value"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false))
}

#[cfg(windows)]
fn cdp_set_file(webview: &tauri::webview::PlatformWebview, path: &str) -> Result<(), String> {
    let _ = cdp_call(
        webview,
        "Page.setInterceptFileChooserDialog",
        &serde_json::json!({ "enabled": true }),
    );
    let document = cdp_call(
        webview,
        "DOM.getDocument",
        &serde_json::json!({ "depth": 0 }),
    )?;
    let root = document
        .pointer("/root/nodeId")
        .and_then(|id| id.as_i64())
        .ok_or_else(|| "provider document is unavailable".to_string())?;
    let nodes = cdp_call(
        webview,
        "DOM.querySelectorAll",
        &serde_json::json!({ "nodeId": root, "selector": "input[type='file']" }),
    )?;
    let node_id = nodes
        .get("nodeIds")
        .and_then(|ids| ids.as_array())
        .and_then(|ids| ids.iter().find_map(|id| id.as_i64()))
        .ok_or_else(|| "provider file input is not open".to_string())?;
    cdp_call(
        webview,
        "DOM.setFileInputFiles",
        &serde_json::json!({ "nodeId": node_id, "files": [path] }),
    )?;
    let _ = cdp_call(
        webview,
        "Page.setInterceptFileChooserDialog",
        &serde_json::json!({ "enabled": false }),
    );
    Ok(())
}

#[cfg(windows)]
fn cdp_call(
    webview: &tauri::webview::PlatformWebview,
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2CallDevToolsProtocolMethodCompletedHandler,
        ICoreWebView2CallDevToolsProtocolMethodCompletedHandler_Impl,
    };
    use windows_core::{implement, Interface, HRESULT, PCWSTR};

    let core = unsafe { webview.controller().CoreWebView2() }.map_err(|e| e.to_string())?;
    let webview11 = core
        .cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_11>()
        .map_err(|_| "this WebView2 build cannot set provider files".to_string())?;
    let method = windows_core::HSTRING::from(method);
    let parameters = windows_core::HSTRING::from(params.to_string());
    let (sender, receiver) = std::sync::mpsc::channel();

    #[implement(ICoreWebView2CallDevToolsProtocolMethodCompletedHandler)]
    struct ProtocolDone(std::sync::Mutex<Option<std::sync::mpsc::Sender<Result<String, String>>>>);

    impl ICoreWebView2CallDevToolsProtocolMethodCompletedHandler_Impl for ProtocolDone_Impl {
        fn Invoke(&self, error: HRESULT, result: &PCWSTR) -> windows_core::Result<()> {
            let outcome = if error.is_ok() {
                Ok(unsafe { result.to_string() }.unwrap_or_default())
            } else {
                Err(error.to_string())
            };
            if let Ok(mut slot) = self.0.lock() {
                if let Some(sender) = slot.take() {
                    let _ = sender.send(outcome);
                }
            }
            Ok(())
        }
    }

    let handler: ICoreWebView2CallDevToolsProtocolMethodCompletedHandler =
        ProtocolDone(std::sync::Mutex::new(Some(sender))).into();
    let started = unsafe {
        let webview = webview11
            .cast::<ICoreWebView2>()
            .map_err(|e| e.to_string())?;
        webview.CallDevToolsProtocolMethod(&method, &parameters, &handler)
    };
    started.map_err(|e| e.to_string())?;
    // The completion callback is posted to this same UI thread. Waiting here
    // would block that callback, so pump messages until it arrives.
    let body = recv_while_pumping(receiver, std::time::Duration::from_secs(8))?;
    if body.trim().is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn recv_while_pumping(
    receiver: std::sync::mpsc::Receiver<Result<String, String>>,
    timeout: std::time::Duration,
) -> Result<String, String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    let deadline = std::time::Instant::now() + timeout;
    let mut message = MSG::default();
    loop {
        if let Ok(result) = receiver.try_recv() {
            return result;
        }
        if std::time::Instant::now() >= deadline {
            return Err("provider page did not answer".into());
        }
        let pending = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) };
        if pending.as_bool() {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

fn host_window(app: &AppHandle) -> Option<Window<Wry>> {
    // A plain `Window`, not `WebviewWindow`. Once this function attaches the
    // provider child, `get_webview_window("main")` returns None. Do not guess
    // another window: feature, doc, and auth popups are not hosts.
    app.get_window("main")
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
