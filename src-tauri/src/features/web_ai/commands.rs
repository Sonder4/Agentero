use super::bridge::PageEvent;
use super::controller::WebAiController;
use super::models::{
    WebAiConversationBinding, WebAiPaperRef, WebAiProvider, WebAiStatus, WebAiTransferResult,
};
use super::providers;
use super::view::WebAiBounds;
use crate::core::error::ApiResult;
use crate::features::system::settings::AppSettingsStore;
use crate::integration::mcp::tunnel::{McpTunnelController, McpTunnelPhase, McpTunnelStatus};
use crate::integration::mcp::McpController;
use agentero_core::features::web_ai::{Binding, ProviderId};
use chrono::Utc;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiOpenArgs {
    pub provider_id: String,
    pub bounds: Option<WebAiBounds>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiProviderArgs {
    pub provider_id: String,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiBoundsArgs {
    pub provider_id: String,
    pub bounds: WebAiBounds,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiBindArgs {
    pub provider_id: String,
    pub paper: WebAiPaperRef,
    pub conversation_url: String,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiTransferTextArgs {
    pub provider_id: String,
    pub text: String,
    pub paper_id: Option<String>,
    pub page: Option<u32>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiTransferFileArgs {
    pub provider_id: String,
    pub path: String,
    pub paper_id: Option<String>,
    pub page: Option<u32>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiContextArgs {
    pub provider_id: String,
    pub text: String,
    pub paper_id: Option<String>,
    pub page: Option<u32>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiRenameArgs {
    pub provider_id: String,
    pub paper: WebAiPaperRef,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiProjectArgs {
    pub provider_id: String,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiCopyArgs {
    pub provider_id: String,
    pub vault_path: String,
    pub paper_path: String,
    pub paper_id: String,
    pub answer: String,
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiConnectorStatus {
    pub provider_id: String,
    pub state: String,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiPageEventArgs {
    pub event: PageEvent,
}

fn provider_id(value: &str) -> Result<ProviderId, agentero_core::error::AppError> {
    match providers::normalize_provider_id(value) {
        Some("chatgpt") => Ok(ProviderId::Chatgpt),
        Some("gemini") => Ok(ProviderId::Gemini),
        Some("deepseek") => Ok(ProviderId::Deepseek),
        Some("kimi") => Ok(ProviderId::Kimi),
        Some("glm") => Ok(ProviderId::Glm),
        _ => Err(agentero_core::error::AppError::message(
            "unknown web AI provider",
        )),
    }
}

fn transfer_file(
    controller: &WebAiController,
    args: WebAiTransferFileArgs,
    kind: agentero_core::features::web_ai::AttachmentKind,
) -> Result<WebAiTransferResult, String> {
    let prepared = agentero_core::features::web_ai::prepare_attachment(Path::new(&args.path), kind)
        .map_err(|e| e.to_string())?;
    match controller.attach_file(&args.provider_id, &prepared) {
        Ok(true) => {
            cleanup_attachment(&prepared.path);
            Ok(WebAiTransferResult {
                provider_id: args.provider_id,
                draft_ready: false,
                attachment_ready: true,
                requires_send: true,
                manual_file: None,
                message: None,
                paper_id: args.paper_id,
                page: args.page,
            })
        }
        Ok(false) | Err(_) => Ok(WebAiTransferResult {
            provider_id: args.provider_id,
            draft_ready: false,
            attachment_ready: false,
            requires_send: true,
            manual_file: Some(prepared.path.display().to_string()),
            message: Some("choose the prepared file in the provider page".into()),
            paper_id: args.paper_id,
            page: args.page,
        }),
    }
}

fn cleanup_attachment(path: &Path) {
    if let Some(request_dir) = path.parent() {
        let _ = std::fs::remove_dir_all(request_dir);
    }
}

fn to_binding(binding: Binding) -> WebAiConversationBinding {
    WebAiConversationBinding {
        provider_id: binding.provider_id.as_str().into(),
        vault_id: binding.vault_key,
        paper_id: binding.paper_id,
        conversation_url: binding.canonical_url,
        conversation_id: binding.conversation_id,
        title: binding.title,
        updated_at: binding.updated_at,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_providers() -> Result<ApiResult<Vec<WebAiProvider>>, String> {
    Ok(ApiResult::ok(providers::list()))
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_status(
    controller: State<'_, Arc<WebAiController>>,
    provider_id: Option<String>,
) -> Result<ApiResult<Vec<WebAiStatus>>, String> {
    Ok(ApiResult::ok(controller.statuses(provider_id.as_deref())))
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_open(
    app: AppHandle,
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiOpenArgs,
) -> Result<ApiResult<WebAiStatus>, String> {
    match controller.open(&app, &args.provider_id, args.bounds).await {
        Ok(status) => Ok(ApiResult::ok(status)),
        Err(error) => Ok(ApiResult::err(agentero_core::error::AppError::message(
            error,
        ))),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_close(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiProviderArgs,
) -> Result<ApiResult<bool>, String> {
    controller.close(&args.provider_id).map(ApiResult::ok)
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_view(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiViewArgs,
) -> Result<ApiResult<bool>, String> {
    controller
        .set_visible(&args.provider_id, args.visible)
        .map(ApiResult::ok)
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WebAiViewArgs {
    pub provider_id: String,
    pub visible: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_set_bounds(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiBoundsArgs,
) -> Result<ApiResult<bool>, String> {
    controller
        .set_bounds(&args.provider_id, args.bounds)
        .map(ApiResult::ok)
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_bind_conversation(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiBindArgs,
) -> Result<ApiResult<WebAiConversationBinding>, String> {
    let provider = provider_id(&args.provider_id).map_err(|e| e.to_string())?;
    match controller.store().upsert_binding(&Binding {
        vault_key: args.paper.vault_id.clone(),
        paper_id: args.paper.paper_id.clone(),
        provider_id: provider,
        conversation_id: super::providers::conversation_id_from_url(
            &args.provider_id,
            &args.conversation_url,
        )
        .ok_or_else(|| "invalid conversation URL".to_string())?,
        canonical_url: super::providers::canonical_conversation_url(
            &args.provider_id,
            &args.conversation_url,
        )
        .ok_or_else(|| "invalid conversation URL".to_string())?,
        project_id: None,
        title: args.title,
        updated_at: Utc::now().to_rfc3339(),
    }) {
        Ok(()) => Ok(ApiResult::ok(to_binding(
            controller
                .store()
                .get_binding(&args.paper.vault_id, &args.paper.paper_id, provider)
                .map_err(|e| e.to_string())?
                .expect("binding inserted"),
        ))),
        Err(error) => Ok(ApiResult::err(error)),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_unbind_conversation(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiBindArgs,
) -> Result<ApiResult<bool>, String> {
    let provider = provider_id(&args.provider_id).map_err(|e| e.to_string())?;
    controller
        .store()
        .delete_binding(&args.paper.vault_id, &args.paper.paper_id, provider)
        .map(ApiResult::ok)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_transfer_text(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiTransferTextArgs,
) -> Result<ApiResult<WebAiTransferResult>, String> {
    let ready = controller.append_text(&args.provider_id, &args.text)?;
    let mut result = WebAiTransferResult {
        provider_id: args.provider_id,
        draft_ready: ready,
        attachment_ready: false,
        requires_send: true,
        manual_file: None,
        message: (!ready).then(|| "provider WebView is not open".into()),
        paper_id: args.paper_id,
        page: args.page,
    };
    if ready {
        result.message = None;
    }
    Ok(ApiResult::ok(result))
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_transfer_selection(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiTransferTextArgs,
) -> Result<ApiResult<WebAiTransferResult>, String> {
    web_ai_transfer_text(controller, args).await
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_transfer_image(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiTransferFileArgs,
) -> Result<ApiResult<WebAiTransferResult>, String> {
    transfer_file(
        &controller,
        args,
        agentero_core::features::web_ai::AttachmentKind::Image,
    )
    .map(ApiResult::ok)
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_transfer_pdf(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiTransferFileArgs,
) -> Result<ApiResult<WebAiTransferResult>, String> {
    transfer_file(
        &controller,
        args,
        agentero_core::features::web_ai::AttachmentKind::Pdf,
    )
    .map(ApiResult::ok)
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_prepare_context(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiContextArgs,
) -> Result<ApiResult<WebAiTransferResult>, String> {
    web_ai_transfer_text(
        controller,
        WebAiTransferTextArgs {
            provider_id: args.provider_id,
            text: args.text,
            paper_id: args.paper_id,
            page: args.page,
        },
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_conversation_rename(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiRenameArgs,
) -> Result<ApiResult<bool>, String> {
    let provider = provider_id(&args.provider_id).map_err(|e| e.to_string())?;
    let Some(mut binding) = controller
        .store()
        .get_binding(&args.paper.vault_id, &args.paper.paper_id, provider)
        .map_err(|e| e.to_string())?
    else {
        return Ok(ApiResult::ok(false));
    };
    binding.title = args.title;
    binding.updated_at = Utc::now().to_rfc3339();
    controller
        .store()
        .upsert_binding(&binding)
        .map(|_| ApiResult::ok(true))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_project_prepare(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiProjectArgs,
) -> Result<ApiResult<bool>, String> {
    Ok(ApiResult::ok(
        controller
            .statuses(Some(&args.provider_id))
            .iter()
            .any(|s| s.view == "ready"),
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_project_create(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiProjectArgs,
) -> Result<ApiResult<bool>, String> {
    web_ai_project_prepare(controller, args).await
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_copy_to_notes(args: WebAiCopyArgs) -> Result<ApiResult<bool>, String> {
    let answer = args.answer.trim();
    if providers::normalize_provider_id(&args.provider_id).is_none()
        || args.vault_path.trim().is_empty()
        || args.paper_path.trim().is_empty()
        || args.paper_id.trim().is_empty()
        || answer.is_empty()
    {
        return Ok(ApiResult::ok(false));
    }
    let body = format!("## Web AI\n\n{answer}\n");
    crate::integration::mcp::notes::write_notes(
        Path::new(&args.vault_path),
        &args.paper_path,
        &args.paper_id,
        &body,
        crate::integration::mcp::notes::WriteMode::Append,
    )
    .map(|_| ApiResult::ok(true))
    .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_connector_status(
    tunnel: State<'_, Arc<McpTunnelController>>,
) -> Result<ApiResult<WebAiConnectorStatus>, String> {
    Ok(ApiResult::ok(connector_status(&tunnel.status())))
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_connector_start(
    app: AppHandle,
    tunnel: State<'_, Arc<McpTunnelController>>,
    mcp: State<'_, Arc<McpController>>,
    store: State<'_, AppSettingsStore>,
) -> Result<ApiResult<WebAiConnectorStatus>, String> {
    let Some((tunnel_id, api_key)) = store.mcp_tunnel_config() else {
        return Ok(ApiResult::err(agentero_core::error::AppError::message(
            "Configure Tunnel ID and Runtime API key first.",
        )));
    };
    let mcp_status = mcp.status();
    let Some(mcp_url) = mcp_status.url else {
        return Ok(ApiResult::err(agentero_core::error::AppError::message(
            "MCP server is not listening.",
        )));
    };
    let status = tunnel.start(mcp_url, tunnel_id, api_key).await;
    let result = connector_status(&status);
    let _ = app.emit("web-ai:connector", &result);
    Ok(ApiResult::ok(result))
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_connector_pair(
    app: AppHandle,
    tunnel: State<'_, Arc<McpTunnelController>>,
) -> Result<ApiResult<WebAiConnectorStatus>, String> {
    let status = tunnel.status();
    let mut result = connector_status(&status);
    if status.phase == McpTunnelPhase::Ready {
        result.state = "pairing".into();
        result.message =
            Some("Open ChatGPT Connector settings and add the Agentero MCP endpoint.".into());
    }
    let _ = app.emit("web-ai:connector", &result);
    Ok(ApiResult::ok(result))
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_connector_disconnect(
    app: AppHandle,
    tunnel: State<'_, Arc<McpTunnelController>>,
) -> Result<ApiResult<WebAiConnectorStatus>, String> {
    tunnel.stop();
    let result = connector_status(&tunnel.status());
    let _ = app.emit("web-ai:connector", &result);
    Ok(ApiResult::ok(result))
}

fn connector_status(status: &McpTunnelStatus) -> WebAiConnectorStatus {
    let state = match status.phase {
        McpTunnelPhase::Ready => "ready",
        McpTunnelPhase::Starting => "starting",
        McpTunnelPhase::BinaryMissing => "binary-missing",
        McpTunnelPhase::Error => "error",
        McpTunnelPhase::Stopped => "stopped",
    };
    WebAiConnectorStatus {
        provider_id: "chatgpt".into(),
        state: state.into(),
        message: status.last_error.clone(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_reset_provider(
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiProviderArgs,
) -> Result<ApiResult<bool>, String> {
    let provider = provider_id(&args.provider_id).map_err(|e| e.to_string())?;
    let _ = controller.close(&args.provider_id);
    let removed = controller
        .store()
        .delete_provider(provider)
        .map_err(|e| e.to_string())?;
    let profile = super::view::profile_dir(&args.provider_id);
    let _ = std::fs::remove_dir_all(profile);
    Ok(ApiResult::ok(removed > 0))
}

#[tauri::command]
#[specta::specta]
pub async fn web_ai_page_event(
    app: AppHandle,
    controller: State<'_, Arc<WebAiController>>,
    args: WebAiPageEventArgs,
) -> Result<ApiResult<bool>, String> {
    let status = controller.apply_page_event(&args.event)?;
    let accepted = status.is_some();
    if let Some(status) = status {
        let _ = app.emit("web-ai:navigation", &args.event);
        let _ = app.emit("web-ai:state", &status);
    }
    Ok(ApiResult::ok(accepted))
}
