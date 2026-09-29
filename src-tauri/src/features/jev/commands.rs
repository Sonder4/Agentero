//! Tauri commands for TypeSafe jEV smart highlighting.

use crate::core::error::{map_err, ApiResult, AppError};
use crate::core::fs::{resolve_paper_dir, resolve_vault};
use crate::features::jev::service::{
    jev_suggest_highlights_for_paper, read_title_from_sidecar, SuggestedHighlight,
};
use crate::features::paper::catalog::probe_paper_caps;
use crate::features::system::settings::AppSettingsStore;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JevSuggestHighlightsArgs {
    pub vault_path: String,
    /// Vault-relative paper folder, e.g. `papers/2303.17760`.
    pub path: String,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JevSuggestHighlightsResult {
    pub highlights: Vec<SuggestedHighlight>,
}

#[tauri::command]
#[specta::specta]
pub async fn jev_probe_health(store: State<'_, AppSettingsStore>) -> Result<ApiResult<()>, String> {
    let (api_key, base_url) = match store.jev_config() {
        Some(cfg) => cfg,
        None => return Ok(map_err(AppError::message("jEV API key is not configured"))),
    };

    match crate::features::jev::service::jev_probe_health(&api_key, &base_url).await {
        Ok(()) => Ok(ApiResult::ok(())),
        Err(err) => Ok(map_err(err)),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn jev_suggest_highlights(
    args: JevSuggestHighlightsArgs,
    store: State<'_, AppSettingsStore>,
) -> Result<ApiResult<JevSuggestHighlightsResult>, String> {
    let vault = match resolve_vault(&args.vault_path) {
        Ok(v) => v,
        Err(err) => return Ok(map_err(err)),
    };
    let (paper_dir, _rel) = match resolve_paper_dir(&vault, &args.path) {
        Ok(p) => p,
        Err(err) => return Ok(map_err(err)),
    };

    let caps = probe_paper_caps(&paper_dir);
    let pdf_path = match caps.pdf_path {
        Some(p) => p,
        None => {
            return Ok(map_err(AppError::message(
                "No local PDF for smart highlight",
            )))
        }
    };

    let title = read_title_from_sidecar(&paper_dir)
        .or_else(|| {
            paper_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "Untitled paper".to_string());

    let (api_key, base_url) = match store.jev_config() {
        Some(cfg) => cfg,
        None => return Ok(map_err(AppError::message("jEV API key is not configured"))),
    };

    match jev_suggest_highlights_for_paper(&pdf_path, &title, &api_key, &base_url).await {
        Ok(highlights) => Ok(ApiResult::ok(JevSuggestHighlightsResult { highlights })),
        Err(err) => Ok(map_err(err)),
    }
}
