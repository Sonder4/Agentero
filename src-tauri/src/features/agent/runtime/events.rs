use serde::Serialize;
use tauri::{AppHandle, Emitter, EventTarget};

#[derive(Clone)]
pub struct AgentEventEmitter {
    app: AppHandle,
    window_label: String,
}

impl AgentEventEmitter {
    pub fn new(app: AppHandle, window_label: impl Into<String>) -> Self {
        Self {
            app,
            window_label: window_label.into(),
        }
    }

    pub fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) -> tauri::Result<()> {
        // Target the calling webview, not its WebviewWindow. A child webview
        // (Web AI) makes the host window fail `is_webview_window`, and the
        // frontend listens with `getCurrentWebview()` (`kind: "Webview"`).
        self.app.emit_to(
            EventTarget::webview(self.window_label.clone()),
            event,
            payload,
        )
    }
}
