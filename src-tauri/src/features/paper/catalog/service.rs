//! Desktop execution adapter for the shared core move use case.

use crate::core::error::AppError;
use crate::core::fs::resolve_vault;
use crate::features::vault::rename::WikiIndex;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

pub use agentero_core::features::paper::catalog::move_paper::PaperMoveResult;

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PaperMoveArgs {
    pub vault_path: String,
    /// Vault-relative item to move (paper folder, org folder, or file under `papers/`).
    pub from_rel: String,
    /// Vault-relative destination parent (`papers` or under `papers/`).
    pub dest_parent_rel: String,
    /// Dirty open Markdown/NOTES paths supplied by the renderer. The Host
    /// rejects a transaction that would move or rewrite one of these files.
    #[serde(default)]
    pub dirty_paths: Vec<String>,
}

pub(crate) async fn paper_move_service(
    args: PaperMoveArgs,
    index: Arc<Mutex<WikiIndex>>,
) -> Result<PaperMoveResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let vault = resolve_vault(&args.vault_path)?;
        let mut guard = index
            .lock()
            .map_err(|error| AppError::message(format!("wiki index lock: {error}")))?;
        agentero_core::features::paper::catalog::move_paper::move_with_index(
            &vault,
            &args.from_rel,
            &args.dest_parent_rel,
            &args.dirty_paths,
            &mut guard,
        )
    })
    .await
    .map_err(|error| AppError::message(format!("blocking task failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::paper::catalog::{self, papers};
    use std::fs;

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("papers/inbox/demo")).unwrap();
        fs::create_dir_all(root.path().join("notes")).unwrap();
        fs::write(root.path().join("papers/inbox/demo/NOTES.md"), "# Demo\n").unwrap();
        fs::write(
            root.path().join("notes/source.md"),
            "[[papers/inbox/demo/NOTES]]\n",
        )
        .unwrap();
        papers::upsert_paper(
            root.path(),
            &papers::PaperRecord::local_pdf("demo".into(), "Demo".into())
                .at_path("papers/inbox/demo"),
        )
        .unwrap();
        root
    }

    #[tokio::test]
    async fn desktop_adapter_and_headless_entry_produce_same_move() {
        let desktop = fixture();
        let headless = fixture();
        let index = Arc::new(Mutex::new(WikiIndex::default()));
        let args = |from: &str, dirty_paths: Vec<String>| PaperMoveArgs {
            vault_path: desktop.path().to_string_lossy().into_owned(),
            from_rel: from.into(),
            dest_parent_rel: "papers/archive".into(),
            dirty_paths,
        };
        // The desktop adapter must carry unsaved-editor protection through.
        assert!(paper_move_service(
            args("papers/inbox/demo", vec!["notes/source.md".into()]),
            Arc::clone(&index)
        )
        .await
        .is_err());
        assert!(desktop.path().join("papers/inbox/demo/NOTES.md").is_file());
        let desktop_result =
            paper_move_service(args("papers/inbox/demo", vec![]), Arc::clone(&index))
                .await
                .unwrap();
        let headless_result =
            catalog::move_paper_under(headless.path(), "papers/inbox/demo", "papers/archive")
                .unwrap();
        assert_eq!(desktop_result.new_rel, headless_result);
        for root in [desktop.path(), headless.path()] {
            assert!(root.join("papers/archive/demo/NOTES.md").is_file());
            assert!(!root.join("papers/inbox/demo").exists());
            assert_eq!(
                papers::get_by_path(root, &headless_result)
                    .unwrap()
                    .unwrap()
                    .id,
                "demo"
            );
            assert!(papers::get_by_path(root, "papers/inbox/demo")
                .unwrap()
                .is_none());
            assert_eq!(
                fs::read_to_string(root.join("notes/source.md")).unwrap(),
                "[[papers/archive/demo/NOTES]]\n"
            );
            catalog::evict_catalog_conn(root);
        }
        let noop = paper_move_service(args(&desktop_result.new_rel, vec![]), index)
            .await
            .unwrap();
        assert_eq!(noop.new_rel, desktop_result.new_rel);
        assert!(noop.link_update.updated_sources.is_empty());
    }
}
