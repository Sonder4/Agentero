//! Tauri commands for remote vault (SSH/SFTP) — `docs/development/remote-vault.md`.
//!
//! Async commands return `Result<ApiResult<T>, String>` so `State` borrows are valid
//! (same pattern as `agent_probe`).

use crate::app::command_util::try_session;
use crate::core::error::{map_err, ApiResult, AppError};
use crate::core::fs::{FsDirEntry, WriteOpts};
use crate::core::log_util::{trunc, OpTimer};
use crate::features::paper::catalog::papers::{self, PaperRecord};
use crate::features::vault::CreateVaultResult;
use crate::integration::remote::{ensure_remote_vault_skills, RemoteRegistry, RemoteSessionInfo};
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

const LEGACY_GENERATED_FILES: &[(&str, &str)] = &[
    ("source/layout-index.json", "layout-index.json"),
    ("source/agentero-cite.json", "citations.json"),
    ("source/layout-translate.json", "layout-translate.json"),
    ("source/layout-translate-glossary.json", "glossary.json"),
    ("source/layout-translate-state.json", "state.json"),
];

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConnectArgs {
    /// SSH host or config alias. Use `__local_sim__` with an absolute local path for tests.
    pub host: String,
    #[serde(default)]
    pub user: Option<String>,
    pub remote_path: String,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_connect(
    registry: State<'_, Arc<RemoteRegistry>>,
    connector: State<'_, Arc<crate::integration::connector::ConnectorController>>,
    args: RemoteConnectArgs,
) -> Result<ApiResult<RemoteSessionInfo>, String> {
    let op = OpTimer::start_with(
        "remote_connect",
        format!(
            "host={} path={}",
            trunc(&args.host, 80),
            trunc(&args.remote_path, 120)
        ),
    );
    match registry
        .connect(&args.host, args.user.as_deref(), &args.remote_path)
        .await
    {
        Ok(info) => {
            // Bind Zotero Connector save target on Host (do not rely only on frontend).
            connector.set_vault(Some(info.vault_handle.clone()));
            log::info!(
                target: "agentero::op",
                "connector vault bound to {}",
                trunc(&info.vault_handle, 80)
            );
            op.finish_ok();
            Ok(ApiResult::ok(info))
        }
        Err(e) => {
            op.finish_err(&e);
            Ok(map_err(e))
        }
    }
}

/// Host entries from `~/.ssh/config` for the connect dialog's suggestions (#339).
#[tauri::command]
#[specta::specta]
pub async fn remote_ssh_config_hosts(
) -> Result<ApiResult<Vec<crate::integration::remote::ssh_config::SshConfigHost>>, String> {
    Ok(ApiResult::ok(
        crate::integration::remote::ssh_config::ssh_config_hosts(),
    ))
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSessionArgs {
    pub session_id: String,
    #[serde(default)]
    pub locale: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_disconnect(
    registry: State<'_, Arc<RemoteRegistry>>,
    connector: State<'_, Arc<crate::integration::connector::ConnectorController>>,
    args: RemoteSessionArgs,
) -> Result<ApiResult<()>, String> {
    let op = OpTimer::start_with(
        "remote_disconnect",
        format!("session={}", trunc(&args.session_id, 40)),
    );
    let handle = format!("remote:{}", args.session_id.trim());
    let bound_here = connector
        .status()
        .vault_path
        .as_deref()
        .is_some_and(|p| p == handle);
    match registry.disconnect(&args.session_id).await {
        Ok(()) => {
            if bound_here {
                connector.set_vault(None);
            }
            op.finish_ok();
            Ok(ApiResult::ok(()))
        }
        Err(e) => {
            op.finish_err(&e);
            Ok(map_err(e))
        }
    }
}

/// Seed bundled content and safely update untouched first-party skills in a
/// remote vault without overwriting user files.
#[tauri::command]
#[specta::specta]
pub async fn remote_vault_ensure(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteSessionArgs,
) -> Result<ApiResult<CreateVaultResult>, String> {
    let session = try_session!(registry, &args.session_id);
    match ensure_remote_vault_skills(&session, args.locale.as_deref()).await {
        Ok(result) => Ok(ApiResult::ok(result)),
        Err(e) => Ok(map_err(e)),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemotePathArgs {
    pub session_id: String,
    #[serde(default)]
    pub path: String,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_list(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemotePathArgs,
) -> Result<ApiResult<Vec<FsDirEntry>>, String> {
    let session = try_session!(registry, &args.session_id);
    match session.fs.list(&args.path).await {
        Ok(v) => Ok(ApiResult::ok(v)),
        Err(e) => Ok(map_err(e)),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn remote_read_text(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemotePathArgs,
) -> Result<ApiResult<String>, String> {
    let session = try_session!(registry, &args.session_id);
    match session.fs.read(&args.path).await {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => Ok(ApiResult::ok(s)),
            Err(e) => Ok(map_err(AppError::message(format!("not utf-8: {e}")))),
        },
        Err(e) => Ok(map_err(e)),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWriteTextArgs {
    pub session_id: String,
    pub path: String,
    pub content: String,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_write_text(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteWriteTextArgs,
) -> Result<ApiResult<()>, String> {
    let session = try_session!(registry, &args.session_id);
    match session
        .fs
        .write(
            &args.path,
            args.content.as_bytes(),
            WriteOpts {
                create_parents: true,
            },
        )
        .await
    {
        Ok(()) => Ok(ApiResult::ok(())),
        Err(e) => Ok(map_err(e)),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn remote_mkdir(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemotePathArgs,
) -> Result<ApiResult<()>, String> {
    let session = try_session!(registry, &args.session_id);
    match session.fs.mkdir(&args.path).await {
        Ok(()) => Ok(ApiResult::ok(())),
        Err(e) => Ok(map_err(e)),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRemoveArgs {
    pub session_id: String,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_remove(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteRemoveArgs,
) -> Result<ApiResult<()>, String> {
    let session = try_session!(registry, &args.session_id);
    match session.fs.remove(&args.path, args.recursive).await {
        Ok(()) => Ok(ApiResult::ok(())),
        Err(e) => Ok(map_err(e)),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWriteBytesArgs {
    pub session_id: String,
    pub path: String,
    pub data: Vec<u8>,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_write_bytes(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteWriteBytesArgs,
) -> Result<ApiResult<()>, String> {
    let session = try_session!(registry, &args.session_id);
    match session
        .fs
        .write(
            &args.path,
            &args.data,
            WriteOpts {
                create_parents: true,
            },
        )
        .await
    {
        Ok(()) => Ok(ApiResult::ok(())),
        Err(e) => Ok(map_err(e)),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemotePaperGetArgs {
    pub session_id: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_paper_get(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemotePaperGetArgs,
) -> Result<ApiResult<PaperRecord>, String> {
    let session = try_session!(registry, &args.session_id);
    let work = session.work_root.clone();
    let result = if let Some(path) = args
        .path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let path = path.trim_matches('/').replace('\\', "/");
        papers::get_by_path(&work, &path)
    } else if let Some(id) = args.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        papers::get_by_id(&work, id)
    } else {
        return Ok(map_err(AppError::message("path or id is required")));
    };
    match result {
        Ok(Some(row)) => Ok(ApiResult::ok(row)),
        Ok(None) => Ok(map_err(AppError::message("paper not found in catalog"))),
        Err(e) => Ok(map_err(e)),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn remote_paper_list(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteSessionArgs,
) -> Result<ApiResult<Vec<PaperRecord>>, String> {
    let session = try_session!(registry, &args.session_id);
    let work = session.work_root.clone();
    match papers::list_all(&work) {
        Ok(rows) => Ok(ApiResult::ok(rows)),
        Err(e) => Ok(map_err(e)),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemotePaperSetTagsArgs {
    pub session_id: String,
    pub path: String,
    pub tags: Vec<papers::PaperTag>,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_paper_set_tags(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemotePaperSetTagsArgs,
) -> Result<ApiResult<PaperRecord>, String> {
    let session = try_session!(registry, &args.session_id);
    let path = args.path.trim().trim_matches('/').replace('\\', "/");
    if path.is_empty() {
        return Ok(map_err(AppError::message("path is required")));
    }
    let row = match papers::set_tags(&session.work_root, &path, &args.tags) {
        Ok(r) => r,
        Err(e) => return Ok(map_err(e)),
    };
    {
        let mut cat = session.catalog.lock().await;
        if let Err(e) = cat.push(session.fs.clone()).await {
            return Ok(map_err(e));
        }
    }
    Ok(ApiResult::ok(row))
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemotePaperSetIsReadArgs {
    pub session_id: String,
    pub path: String,
    pub is_read: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_paper_set_is_read(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemotePaperSetIsReadArgs,
) -> Result<ApiResult<PaperRecord>, String> {
    let session = try_session!(registry, &args.session_id);
    let path = args.path.trim().trim_matches('/').replace('\\', "/");
    if path.is_empty() {
        return Ok(map_err(AppError::message("path is required")));
    }
    let row = match papers::set_is_read(&session.work_root, &path, args.is_read) {
        Ok(r) => r,
        Err(e) => return Ok(map_err(e)),
    };
    {
        let mut cat = session.catalog.lock().await;
        if let Err(e) = cat.push(session.fs.clone()).await {
            return Ok(map_err(e));
        }
    }
    Ok(ApiResult::ok(row))
}

/// Ensure a remote PDF (or other file) is cached under the session blob dir; return local path.
#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCacheFileArgs {
    pub session_id: String,
    pub path: String,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCacheFileResult {
    /// Absolute local path to cached bytes (ephemeral).
    pub local_path: String,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_cache_file(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteCacheFileArgs,
) -> Result<ApiResult<RemoteCacheFileResult>, String> {
    let session = try_session!(registry, &args.session_id);
    let rel = match crate::core::fs::sanitize_vault_rel(&args.path) {
        Ok(r) => r,
        Err(_) => return Ok(map_err(AppError::message("invalid path"))),
    };
    let meta = match session.fs.stat(&rel).await {
        Ok(m) => m,
        Err(e) => return Ok(map_err(e)),
    };
    let dest = crate::integration::remote::blob_cache::ensure_cached(
        &session.blob_root,
        &rel,
        meta.size,
        meta.mtime,
        || async { session.fs.read(&rel).await },
    )
    .await;
    match dest {
        Ok(dest) => Ok(ApiResult::ok(RemoteCacheFileResult {
            local_path: dest.to_string_lossy().into_owned(),
        })),
        Err(e) => Ok(map_err(e)),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCacheStatsArgs {
    /// When set, stats for that session's blob dir; otherwise all remote caches.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_cache_stats(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteCacheStatsArgs,
) -> Result<ApiResult<crate::integration::remote::blob_cache::BlobCacheStats>, String> {
    use crate::integration::remote::blob_cache;
    if let Some(sid) = args
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let session = try_session!(registry, sid);
        Ok(ApiResult::ok(blob_cache::stats_for_root(
            &session.blob_root,
        )))
    } else {
        Ok(ApiResult::ok(blob_cache::stats_all()))
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCacheClearArgs {
    /// When set, clear that session's blobs; otherwise all remote blob caches.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCacheClearResult {
    pub freed_bytes: u64,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_cache_clear(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteCacheClearArgs,
) -> Result<ApiResult<RemoteCacheClearResult>, String> {
    use crate::integration::remote::blob_cache;
    let op = OpTimer::start("remote_cache_clear");
    let result = if let Some(sid) = args
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let session = try_session!(registry, sid, op);
        blob_cache::clear_root(&session.blob_root)
    } else {
        blob_cache::clear_all()
    };
    match result {
        Ok(freed) => {
            op.finish_ok_extra(format!("freed_bytes={freed}"));
            Ok(ApiResult::ok(RemoteCacheClearResult { freed_bytes: freed }))
        }
        Err(e) => {
            op.finish_err(&e);
            Ok(map_err(e))
        }
    }
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemotePaperRescanResult {
    pub count: usize,
}

#[tauri::command]
#[specta::specta]
pub async fn remote_paper_rescan(
    registry: State<'_, Arc<RemoteRegistry>>,
    args: RemoteSessionArgs,
) -> Result<ApiResult<RemotePaperRescanResult>, String> {
    let session = try_session!(registry, &args.session_id);
    match remote_rescan_impl(&session).await {
        Ok(r) => Ok(ApiResult::ok(r)),
        Err(e) => Ok(map_err(e)),
    }
}

async fn remote_rescan_impl(
    session: &crate::integration::remote::RemoteSession,
) -> Result<RemotePaperRescanResult, AppError> {
    use papers::PaperRecord;

    let mut count = 0usize;
    let now = crate::core::time::now_rfc3339_millis();

    let mut stack = vec!["papers".to_string()];
    while let Some(dir) = stack.pop() {
        let entries = match session.fs.list(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        let legacy_metadata = entries
            .iter()
            .any(|e| e.is_file && e.name == "metadata.json");
        let canonical_metadata = entries.iter().any(|e| {
            e.is_dir && e.name == ".src"
            // The marker is checked below through VaultFs because a
            // directory listing does not expose its children here.
        });
        let canonical_metadata = canonical_metadata
            && session
                .fs
                .exists(&format!("{dir}/.src/metadata.json"))
                .await
                .unwrap_or(false);
        if legacy_metadata {
            migrate_remote_metadata(session, &dir, canonical_metadata).await;
        }
        migrate_remote_generated_files(session, &dir).await;
        let mut has_marker = false;
        for e in &entries {
            if e.is_file && matches!(e.name.as_str(), "NOTES.md" | "highlights.md" | "PAPER.md") {
                has_marker = true;
            }
            if e.is_dir && matches!(e.name.as_str(), "source" | "assets" | "marks" | ".src") {
                has_marker = true;
            }
        }
        has_marker |= canonical_metadata || legacy_metadata;
        if has_marker && dir != "papers" {
            let path = dir.clone();
            let id = path.rsplit('/').next().unwrap_or("paper").to_string();
            let existing = papers::get_by_path(&session.work_root, &path)?;
            let remote_meta = if existing.is_none() {
                session
                    .fs
                    .read(&format!("{path}/.src/metadata.json"))
                    .await
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<PaperRecord>(&bytes).ok())
            } else {
                None
            };
            let mut rec = existing.or(remote_meta).unwrap_or_else(|| {
                let mut rec = PaperRecord::local_pdf(id.clone(), id.clone()).at_path(&path);
                rec.meta_source = Some("remote_rescan".into());
                rec
            });
            if let Ok(bytes) = session.fs.read(&format!("{path}/NOTES.md")).await {
                if let Ok(text) = String::from_utf8(bytes) {
                    if let Some(line) = text.lines().find(|l| l.starts_with("# ")) {
                        rec.title = line.trim_start_matches('#').trim().to_string();
                    }
                }
            }
            rec.path = path;
            rec.updated_at = now.clone();
            papers::upsert_paper(&session.work_root, &rec)?;
            let local_metadata = session.work_root.join(&rec.path).join(".src/metadata.json");
            if let Ok(bytes) = std::fs::read(&local_metadata) {
                let _ = session
                    .fs
                    .write(
                        &format!("{}/.src/metadata.json", rec.path),
                        &bytes,
                        WriteOpts {
                            create_parents: true,
                        },
                    )
                    .await;
            }
            count += 1;
            continue;
        }
        for e in entries {
            if e.is_dir && e.name != "source" && e.name != "assets" && e.name != "marks" {
                stack.push(e.path);
            }
        }
    }

    {
        let mut cat = session.catalog.lock().await;
        cat.push(session.fs.clone()).await?;
    }

    Ok(RemotePaperRescanResult { count })
}

/// One-way migration for remote vaults. The local catalog mirror cannot use
/// the core filesystem migrator because paper files remain on the remote
/// host, so copy the bytes through `VaultFs` and remove the legacy file only
/// after the canonical write succeeds. Conflicts are retained verbatim.
async fn migrate_remote_metadata(
    session: &crate::integration::remote::RemoteSession,
    dir: &str,
    canonical_exists: bool,
) {
    let legacy = format!("{dir}/metadata.json");
    let canonical = format!("{dir}/.src/metadata.json");
    if canonical_exists {
        let old = session.fs.read(&legacy).await;
        let new = session.fs.read(&canonical).await;
        match (old, new) {
            (Ok(old), Ok(new)) if old == new => {
                if let Err(e) = session.fs.remove(&legacy, false).await {
                    log::warn!(
                        target: "agentero::remote",
                        "failed to remove duplicate remote metadata {legacy}: {e}"
                    );
                }
            }
            (Ok(_), Ok(_)) => log::warn!(
                target: "agentero::remote",
                "remote metadata migration conflict; preserving both {legacy} and {canonical}"
            ),
            (Err(e), _) | (_, Err(e)) => log::warn!(
                target: "agentero::remote",
                "failed to inspect remote metadata {legacy}: {e}"
            ),
        }
        return;
    }
    match session.fs.read(&legacy).await {
        Ok(bytes) => {
            if let Err(e) = session
                .fs
                .write(
                    &canonical,
                    &bytes,
                    WriteOpts {
                        create_parents: true,
                    },
                )
                .await
            {
                log::warn!(
                    target: "agentero::remote",
                    "failed to migrate remote metadata {legacy} -> {canonical}: {e}"
                );
                return;
            }
            if let Err(e) = session.fs.remove(&legacy, false).await {
                log::warn!(
                    target: "agentero::remote",
                    "canonical remote metadata written but legacy file retained {legacy}: {e}"
                );
            }
        }
        Err(e) => log::warn!(
            target: "agentero::remote",
            "failed to read legacy remote metadata {legacy}: {e}"
        ),
    }
}

/// Move rebuildable artifacts out of the legacy `source/` directory on a
/// remote vault. Conflicts are retained, matching the local migration policy.
async fn migrate_remote_generated_files(
    session: &crate::integration::remote::RemoteSession,
    dir: &str,
) {
    for (legacy_name, target_name) in LEGACY_GENERATED_FILES {
        let legacy = format!("{dir}/{legacy_name}");
        if !session.fs.exists(&legacy).await.unwrap_or(false) {
            continue;
        }
        let canonical = format!("{dir}/.src/{target_name}");
        if session.fs.exists(&canonical).await.unwrap_or(false) {
            let old = session.fs.read(&legacy).await;
            let new = session.fs.read(&canonical).await;
            match (old, new) {
                (Ok(old), Ok(new)) if old == new => {
                    let _ = session.fs.remove(&legacy, false).await;
                }
                (Ok(_), Ok(_)) => log::warn!(
                    target: "agentero::remote",
                    "remote generated-file migration conflict; preserving both {legacy} and {canonical}"
                ),
                _ => {}
            }
            continue;
        }
        let Ok(bytes) = session.fs.read(&legacy).await else {
            continue;
        };
        if session
            .fs
            .write(
                &canonical,
                &bytes,
                WriteOpts {
                    create_parents: true,
                },
            )
            .await
            .is_ok()
        {
            let _ = session.fs.remove(&legacy, false).await;
        }
    }
}
