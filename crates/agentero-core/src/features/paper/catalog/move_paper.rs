//! Complete local paper move use case shared by desktop, Connector and CLI.

use super::{papers, plan_paper_move_under};
use crate::error::AppError;
use crate::features::vault::rename::{
    run_local_rename_transaction, WikiIndex, WikiRenameResult, WikiRenameRollback,
};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PaperMoveResult {
    /// New vault-relative path of the moved item.
    pub new_rel: String,
    /// Link-aware transaction details for UI refresh and diagnostics.
    pub link_update: WikiRenameResult,
}

/// Move the filesystem, catalog/page counts and incoming links together using
/// the existing recoverable rename transaction. The index is rebuilt before
/// planning, so headless callers may supply a fresh index and no dirty paths.
/// Moving to the current parent is an idempotent success with no changes.
pub fn move_with_index(
    vault: &Path,
    from_rel: &str,
    dest_parent_rel: &str,
    dirty_paths: &[String],
    index: &mut WikiIndex,
) -> Result<PaperMoveResult, AppError> {
    let (from, new_rel) = plan_paper_move_under(vault, from_rel, dest_parent_rel)?;
    if new_rel == from {
        return Ok(PaperMoveResult {
            link_update: WikiRenameResult {
                moved_path: new_rel.clone(),
                updated_sources: Vec::new(),
                skipped: Vec::new(),
                rollback: WikiRenameRollback::NotNeeded,
            },
            new_rel,
        });
    }
    let link_update =
        run_local_rename_transaction(vault, index, &from, &new_rel, dirty_paths, || {
            papers::move_under_path(vault, &from, &new_rel)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .map_err(|error| AppError::message(error.to_string()))?;
    crate::usage::rename_path_best_effort(&vault.to_string_lossy(), &from, &new_rel);
    Ok(PaperMoveResult {
        new_rel,
        link_update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::paper::catalog::{ensure_catalog, evict_catalog_conn};
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
        papers::set_page_counts(root.path(), &[("papers/inbox/demo".into(), 5)]).unwrap();
        root
    }

    #[test]
    fn fresh_and_existing_indexes_share_move_and_noop_semantics() {
        let fresh = fixture();
        let warm = fixture();
        let new_rel =
            super::super::move_paper_under(fresh.path(), "papers/inbox/demo", "papers/archive")
                .unwrap();
        let mut index = WikiIndex::default();
        index.rebuild(warm.path().to_str().unwrap()).unwrap();
        let moved = move_with_index(
            warm.path(),
            "papers/inbox/demo",
            "papers/archive",
            &[],
            &mut index,
        )
        .unwrap();
        assert_eq!(new_rel, moved.new_rel);
        assert_eq!(moved.link_update.updated_sources, vec!["notes/source.md"]);
        for vault in [fresh.path(), warm.path()] {
            assert!(!vault.join("papers/inbox/demo").exists());
            assert!(vault.join("papers/archive/demo/NOTES.md").is_file());
            assert!(papers::get_by_path(vault, "papers/inbox/demo")
                .unwrap()
                .is_none());
            assert_eq!(
                papers::get_by_path(vault, &new_rel).unwrap().unwrap().id,
                "demo"
            );
            assert_eq!(
                fs::read_to_string(vault.join("notes/source.md")).unwrap(),
                "[[papers/archive/demo/NOTES]]\n"
            );
            let conn = ensure_catalog(vault).unwrap();
            assert_eq!(
                conn.query_row(
                    "SELECT page_count FROM pdf_page_counts WHERE path='papers/archive/demo'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                5
            );
            let noop = move_with_index(
                vault,
                &new_rel,
                "papers/archive",
                &["notes/source.md".into()],
                &mut WikiIndex::default(),
            )
            .unwrap();
            assert_eq!(noop.new_rel, new_rel);
            assert!(noop.link_update.updated_sources.is_empty());
            assert_eq!(noop.link_update.rollback, WikiRenameRollback::NotNeeded);
            evict_catalog_conn(vault);
        }
    }

    #[test]
    fn dirty_link_source_or_moved_note_prevents_any_changes() {
        for dirty in ["notes/source.md", "papers/inbox/demo/NOTES.md"] {
            let root = fixture();
            let err = move_with_index(
                root.path(),
                "papers/inbox/demo",
                "papers/archive",
                &[dirty.into()],
                &mut WikiIndex::default(),
            )
            .unwrap_err();
            assert!(err.to_string().contains("UnsavedEdits"), "{err}");
            assert_rolled_back(root.path());
            evict_catalog_conn(root.path());
        }
    }

    #[test]
    fn page_count_commit_failure_rolls_back_catalog_files_and_links() {
        let root = fixture();
        let conn = ensure_catalog(root.path()).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_page_move BEFORE UPDATE ON pdf_page_counts BEGIN SELECT RAISE(ABORT, 'injected page move failure'); END;").unwrap();
        let err = move_with_index(
            root.path(),
            "papers/inbox/demo",
            "papers/archive",
            &[],
            &mut WikiIndex::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("injected page move failure"),
            "{err}"
        );
        assert_rolled_back(root.path());
        assert_eq!(
            conn.query_row("SELECT path FROM pdf_page_counts", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "papers/inbox/demo"
        );
        evict_catalog_conn(root.path());
    }

    fn assert_rolled_back(vault: &Path) {
        assert!(vault.join("papers/inbox/demo/NOTES.md").is_file());
        assert!(!vault.join("papers/archive/demo").exists());
        assert!(papers::get_by_path(vault, "papers/inbox/demo")
            .unwrap()
            .is_some());
        assert!(papers::get_by_path(vault, "papers/archive/demo")
            .unwrap()
            .is_none());
        assert_eq!(
            fs::read_to_string(vault.join("notes/source.md")).unwrap(),
            "[[papers/inbox/demo/NOTES]]\n"
        );
    }
}
