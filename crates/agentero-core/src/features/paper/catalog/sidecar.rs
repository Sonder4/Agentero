//! Per-paper metadata sidecar: `{paper}/.src/metadata.json`.
//!
//! Catalog SQLite stays the fast query surface, but every authoritative field
//! (tags, is_read, urls, …) is projected into this file so a catalog row can
//! be rebuilt from disk alone. This keeps the vault self-contained (backup /
//! sync tools only need plain files) and honors local-first.

use super::papers::PaperRecord;
use std::fs;
use std::path::Path;

pub const SIDECAR_FILE: &str = "metadata.json";
pub const SIDECAR_DIR: &str = ".src";

/// Rebuildable files that used to be mixed into `source/`.  Keep this list in
/// one place so the one-way migration and its conflict policy stay aligned
/// with the runtime writers.
const LEGACY_GENERATED_FILES: &[(&str, &str)] = &[
    ("source/layout-index.json", "layout-index.json"),
    ("source/agentero-cite.json", "citations.json"),
    ("source/layout-translate.json", "layout-translate.json"),
    ("source/layout-translate-glossary.json", "glossary.json"),
    ("source/layout-translate-state.json", "state.json"),
];

/// Move one rebuildable artifact into `.src`, preserving both copies when a
/// destination already contains different bytes.  A byte-identical duplicate
/// is safe to remove, which makes repeated catalog rescans idempotent.
fn migrate_generated_file(legacy: &Path, target: &Path) -> usize {
    if !legacy.is_file() {
        return 0;
    }
    match (target.is_file(), fs::read(legacy)) {
        (true, Ok(old_bytes)) => match fs::read(target) {
            Ok(new_bytes) if new_bytes == old_bytes => match fs::remove_file(legacy) {
                Ok(()) => 1,
                Err(e) => {
                    log::warn!(
                        target: "agentero::catalog",
                        "failed to remove duplicate generated file {}: {e}",
                        legacy.display()
                    );
                    0
                }
            },
            Ok(_) => {
                log::warn!(
                    target: "agentero::catalog",
                    "generated-file migration conflict; preserving both {} and {}",
                    legacy.display(),
                    target.display()
                );
                0
            }
            Err(e) => {
                log::warn!(
                    target: "agentero::catalog",
                    "failed to read canonical generated file {}: {e}",
                    target.display()
                );
                0
            }
        },
        (true, Err(e)) => {
            log::warn!(
                target: "agentero::catalog",
                "failed to read legacy generated file {}: {e}",
                legacy.display()
            );
            0
        }
        (false, Ok(_)) => {
            let Some(parent) = target.parent() else {
                return 0;
            };
            if let Err(e) = fs::create_dir_all(parent).and_then(|_| fs::rename(legacy, target)) {
                log::warn!(
                    target: "agentero::catalog",
                    "failed to migrate generated file {} -> {}: {e}",
                    legacy.display(),
                    target.display()
                );
                0
            } else {
                1
            }
        }
        (false, Err(e)) => {
            log::warn!(
                target: "agentero::catalog",
                "failed to inspect legacy generated file {}: {e}",
                legacy.display()
            );
            0
        }
    }
}

/// Migrate legacy paper-root metadata and generated source files into `.src`.
///
/// This is intentionally a one-way migration. Runtime readers only inspect
/// [`sidecar_path`]; the old location is not a fallback. A destination that
/// already exists is never overwritten: identical bytes are deduplicated by
/// removing the old copy, while differing bytes are left in place and logged
/// for manual resolution.
pub fn migrate_legacy_sidecars(vault_root: &Path) -> std::io::Result<usize> {
    let papers_dir = vault_root.join("papers");
    if !papers_dir.is_dir() {
        return Ok(0);
    }
    let mut moved = 0usize;
    let mut stack = vec![papers_dir];
    while let Some(dir) = stack.pop() {
        let legacy = dir.join(SIDECAR_FILE);
        if legacy.is_file() {
            let target_dir = dir.join(SIDECAR_DIR);
            let target = target_dir.join(SIDECAR_FILE);
            match (target.is_file(), fs::read(&legacy)) {
                (true, Ok(old_bytes)) => match fs::read(&target) {
                    Ok(new_bytes) if new_bytes == old_bytes => match fs::remove_file(&legacy) {
                        Ok(()) => {
                            moved += 1;
                        }
                        Err(e) => log::warn!(
                            target: "agentero::catalog",
                            "failed to remove duplicate legacy metadata {}: {e}",
                            legacy.display()
                        ),
                    },
                    Ok(_) => log::warn!(
                        target: "agentero::catalog",
                        "metadata migration conflict; preserving both {} and {}",
                        legacy.display(),
                        target.display()
                    ),
                    Err(e) => log::warn!(
                        target: "agentero::catalog",
                        "failed to read canonical metadata {}: {e}",
                        target.display()
                    ),
                },
                (true, Err(e)) => log::warn!(
                    target: "agentero::catalog",
                    "failed to read legacy metadata {}: {e}",
                    legacy.display()
                ),
                (false, Ok(_)) => {
                    if let Err(e) =
                        fs::create_dir_all(&target_dir).and_then(|_| fs::rename(&legacy, &target))
                    {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            log::debug!(
                                target: "agentero::catalog",
                                "legacy metadata already absent {} -> {}: {e}",
                                legacy.display(),
                                target.display()
                            );
                        } else {
                            log::warn!(
                                target: "agentero::catalog",
                                "failed to migrate metadata {} -> {}: {e}",
                                legacy.display(),
                                target.display()
                            );
                        }
                    } else {
                        moved += 1;
                    }
                }
                (false, Err(e)) => log::warn!(
                    target: "agentero::catalog",
                    "failed to inspect legacy metadata {}: {e}",
                    legacy.display()
                ),
            }
        }

        // Move derived layout, translation, and citation JSON out of
        // `source/`, which is reserved for raw MinerU/LaTeX/layout inputs.
        for (legacy_rel, target_name) in LEGACY_GENERATED_FILES {
            moved += migrate_generated_file(
                &dir.join(legacy_rel),
                &dir.join(SIDECAR_DIR).join(target_name),
            );
        }

        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str());
            if path.is_dir()
                && !matches!(
                    name,
                    Some(SIDECAR_DIR | "source" | "assets" | "attachments" | "marks")
                )
            {
                stack.push(path);
            }
        }
    }
    Ok(moved)
}

fn sidecar_path(vault_root: &Path, rel_path: &str) -> std::path::PathBuf {
    vault_root
        .join(rel_path)
        .join(SIDECAR_DIR)
        .join(SIDECAR_FILE)
}

/// Project a catalog row into `{vault}/{record.path}/.src/metadata.json`.
/// Best-effort: catalog write already succeeded; a failed projection only
/// logs (next upsert rewrites it).
pub fn write_sidecar(vault_root: &Path, record: &PaperRecord) {
    let dir = vault_root.join(&record.path);
    if !dir.is_dir() {
        return;
    }
    let write = || -> std::io::Result<()> {
        let raw = serde_json::to_vec_pretty(record)?;
        fs::create_dir_all(dir.join(SIDECAR_DIR))?;
        crate::fs::atomic_write(&sidecar_path(vault_root, &record.path), &raw)
    };
    if let Err(e) = write() {
        log::warn!(
            target: "agentero::catalog",
            "failed to write sidecar for {}: {e}",
            record.path
        );
    }
}

/// Read `{vault}/{rel_path}/.src/metadata.json` as a catalog row.
/// `path` always comes from the on-disk location (sidecars move with their
/// folder, so any embedded path may be stale). Tolerates the older
/// Connector-era files (no `path`, bare-string tags).
pub fn read_sidecar(vault_root: &Path, rel_path: &str) -> Option<PaperRecord> {
    let raw = fs::read_to_string(sidecar_path(vault_root, rel_path)).ok()?;
    let mut record: PaperRecord = serde_json::from_str(&raw).ok()?;
    record.path = rel_path.to_string();
    Some(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::catalog::papers::PaperKind;
    use uuid::Uuid;

    fn minimal_record(path: &str) -> PaperRecord {
        serde_json::from_value(serde_json::json!({
            "path": path,
            "id": "x",
            "type": "article",
            "title": "T",
            "authors": [],
            "status": "completed",
            "added_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-02T00:00:00Z",
        }))
        .expect("minimal record")
    }

    #[test]
    fn sidecar_roundtrip_overrides_stale_path() {
        let vault = std::env::temp_dir().join(format!("agentero-sidecar-{}", Uuid::new_v4()));
        fs::create_dir_all(vault.join("papers/x")).unwrap();
        let mut record = minimal_record("papers/x");
        record.is_read = true;
        write_sidecar(&vault, &record);

        // Simulate a folder move: read back from the new location.
        fs::create_dir_all(vault.join("papers/nlp")).unwrap();
        fs::rename(vault.join("papers/x"), vault.join("papers/nlp/x")).unwrap();
        let loaded = read_sidecar(&vault, "papers/nlp/x").expect("sidecar readable");
        assert_eq!(loaded.path, "papers/nlp/x");
        assert!(loaded.is_read);
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn read_sidecar_tolerates_connector_paper_meta_shape() {
        let vault = std::env::temp_dir().join(format!("agentero-sidecar-meta-{}", Uuid::new_v4()));
        fs::create_dir_all(vault.join("papers/y/.src")).unwrap();
        // Connector-era metadata.json: no `path`, tags as bare strings.
        fs::write(
            vault.join("papers/y/.src/metadata.json"),
            r#"{"id":"y","type":"article","title":"Old","authors":["A"],"tags":["nlp"],
                "status":"completed","added_at":"t","updated_at":"t"}"#,
        )
        .unwrap();
        let loaded = read_sidecar(&vault, "papers/y").expect("legacy sidecar readable");
        assert_eq!(loaded.path, "papers/y");
        assert_eq!(loaded.paper_type, PaperKind::Doi);
        assert_eq!(loaded.tags.len(), 1);
        assert_eq!(loaded.tags[0].name, "nlp");
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn legacy_sidecar_migrates_once_and_preserves_conflicts() {
        let vault =
            std::env::temp_dir().join(format!("agentero-sidecar-migrate-{}", Uuid::new_v4()));
        let paper = vault.join("papers/x");
        fs::create_dir_all(&paper).unwrap();
        let raw = br#"{"id":"x","type":"article","title":"Old","authors":[],"status":"completed","added_at":"t","updated_at":"t"}"#;
        fs::write(paper.join(SIDECAR_FILE), raw).unwrap();
        assert!(read_sidecar(&vault, "papers/x").is_none());
        assert_eq!(migrate_legacy_sidecars(&vault).unwrap(), 1);
        assert!(!paper.join(SIDECAR_FILE).exists());
        assert_eq!(
            fs::read(paper.join(SIDECAR_DIR).join(SIDECAR_FILE)).unwrap(),
            raw
        );
        assert_eq!(migrate_legacy_sidecars(&vault).unwrap(), 0);

        let conflict = vault.join("papers/conflict");
        fs::create_dir_all(conflict.join(SIDECAR_DIR)).unwrap();
        fs::write(conflict.join(SIDECAR_FILE), b"old").unwrap();
        fs::write(conflict.join(SIDECAR_DIR).join(SIDECAR_FILE), b"new").unwrap();
        assert_eq!(migrate_legacy_sidecars(&vault).unwrap(), 0);
        assert_eq!(fs::read(conflict.join(SIDECAR_FILE)).unwrap(), b"old");
        assert_eq!(
            fs::read(conflict.join(SIDECAR_DIR).join(SIDECAR_FILE)).unwrap(),
            b"new"
        );
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn legacy_generated_artifacts_migrate_into_src_once() {
        let vault =
            std::env::temp_dir().join(format!("agentero-generated-migrate-{}", Uuid::new_v4()));
        let paper = vault.join("papers/x");
        fs::create_dir_all(paper.join("source")).unwrap();
        fs::write(paper.join("source/layout.json"), b"raw-layout").unwrap();
        for (legacy_rel, _) in LEGACY_GENERATED_FILES {
            fs::write(paper.join(legacy_rel), legacy_rel.as_bytes()).unwrap();
        }

        assert_eq!(
            migrate_legacy_sidecars(&vault).unwrap(),
            LEGACY_GENERATED_FILES.len()
        );
        for (legacy_rel, target_name) in LEGACY_GENERATED_FILES {
            assert!(!paper.join(legacy_rel).exists());
            assert_eq!(
                fs::read(paper.join(SIDECAR_DIR).join(target_name)).unwrap(),
                legacy_rel.as_bytes()
            );
        }
        assert_eq!(migrate_legacy_sidecars(&vault).unwrap(), 0);
        assert_eq!(
            fs::read(paper.join("source/layout.json")).unwrap(),
            b"raw-layout"
        );

        // A differing destination never overwrites user data or deletes the
        // old artifact; the next rescan can retry after manual resolution.
        let conflict = vault.join("papers/conflict");
        fs::create_dir_all(conflict.join("source")).unwrap();
        fs::create_dir_all(conflict.join(SIDECAR_DIR)).unwrap();
        fs::write(conflict.join("source/layout-index.json"), b"legacy-index").unwrap();
        fs::write(
            conflict.join(SIDECAR_DIR).join("layout-index.json"),
            b"new-index",
        )
        .unwrap();
        assert_eq!(migrate_legacy_sidecars(&vault).unwrap(), 0);
        assert!(conflict.join("source/layout-index.json").is_file());
        assert_eq!(
            fs::read(conflict.join(SIDECAR_DIR).join("layout-index.json")).unwrap(),
            b"new-index"
        );

        let _ = fs::remove_dir_all(&vault);
    }
}
