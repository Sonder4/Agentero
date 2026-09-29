//! Catalog checkout / checkin for remote vaults (`docs/backend/remote.md`).
//!
//! SQLite cannot open over SFTP; Host keeps an ephemeral work copy and push-after-write.

use crate::core::error::AppError;
use crate::core::fs::{FsFileMeta, VaultFs, WriteOpts};
use crate::features::paper::catalog::{ensure_catalog, schema_version, SCHEMA_VERSION};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CATALOG_REL: &str = ".agentero/catalog.sqlite";
const CATALOG_TMP_REL: &str = ".agentero/catalog.sqlite.tmp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteFileStamp {
    pub size: u64,
    pub mtime: u64,
}

impl From<FsFileMeta> for RemoteFileStamp {
    fn from(m: FsFileMeta) -> Self {
        Self {
            size: m.size,
            mtime: m.mtime,
        }
    }
}

pub struct CatalogMirror {
    work_db: PathBuf,
    /// Stamp of the remote file at last successful GET/PUT.
    remote_stamp: RemoteFileStamp,
}

impl CatalogMirror {
    pub fn work_db_path(&self) -> &Path {
        &self.work_db
    }

    pub fn remote_stamp(&self) -> RemoteFileStamp {
        self.remote_stamp
    }

    /// Download remote catalog (or create empty + migrate if missing), store under `work_root`.
    pub async fn checkout(fs: Arc<dyn VaultFs>, work_root: &Path) -> Result<Self, AppError> {
        let work_agentero = work_root.join(".agentero");
        std::fs::create_dir_all(&work_agentero)?;
        let work_db = work_agentero.join("catalog.sqlite");

        let stamp = if fs.exists(CATALOG_REL).await? {
            let bytes = fs.read(CATALOG_REL).await?;
            let meta = fs.stat(CATALOG_REL).await?;
            std::fs::write(&work_db, &bytes)?;
            RemoteFileStamp::from(meta)
        } else {
            // Initialize empty catalog locally then push as authority on remote.
            if work_db.exists() {
                let _ = std::fs::remove_file(&work_db);
            }
            // ensure_catalog expects vault root (= work_root here)
            let _conn = ensure_catalog(work_root)?;
            // Create remote .agentero and push
            let _ = fs.mkdir(".agentero").await;
            let bytes = snapshot_catalog(&work_db)?;
            fs.write(
                CATALOG_REL,
                &bytes,
                WriteOpts {
                    create_parents: true,
                },
            )
            .await?;
            let meta = fs.stat(CATALOG_REL).await?;
            RemoteFileStamp::from(meta)
        };

        // Ensure schema is current on work copy
        {
            let conn = ensure_catalog(work_root)?;
            let ver = schema_version(&conn).unwrap_or(0);
            if ver > SCHEMA_VERSION {
                return Err(AppError::message(format!(
                    "catalog schema version {ver} is newer than this app supports ({SCHEMA_VERSION})"
                )));
            }
        }

        Ok(Self {
            work_db,
            remote_stamp: stamp,
        })
    }

    pub fn open(&self) -> Result<Connection, AppError> {
        let conn = Connection::open(&self.work_db)
            .map_err(|e| AppError::message(format!("open work catalog: {e}")))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| AppError::message(format!("pragma: {e}")))?;
        Ok(conn)
    }

    /// Optimistic lock + atomic-ish push (tmp then rename on remote when possible).
    pub async fn push(&mut self, fs: Arc<dyn VaultFs>) -> Result<(), AppError> {
        if fs.exists(CATALOG_REL).await? {
            let meta = fs.stat(CATALOG_REL).await?;
            let stamp = RemoteFileStamp::from(meta);
            if stamp != self.remote_stamp {
                return Err(AppError::message(
                    "remote catalog changed (conflict); reopen the remote vault",
                ));
            }
        }

        let bytes = snapshot_catalog(&self.work_db)?;
        // Write tmp then rename
        fs.write(
            CATALOG_TMP_REL,
            &bytes,
            WriteOpts {
                create_parents: true,
            },
        )
        .await?;
        match fs.rename(CATALOG_TMP_REL, CATALOG_REL).await {
            Ok(()) => {}
            Err(_) => {
                // Fallback: direct overwrite
                fs.write(
                    CATALOG_REL,
                    &bytes,
                    WriteOpts {
                        create_parents: true,
                    },
                )
                .await?;
                let _ = fs.remove(CATALOG_TMP_REL, false).await;
            }
        }
        let meta = fs.stat(CATALOG_REL).await?;
        self.remote_stamp = RemoteFileStamp::from(meta);
        Ok(())
    }
}

/// Read a consistent, standalone SQLite image, including committed WAL pages.
/// VACUUM INTO reads through SQLite's snapshot isolation; reading the source
/// file (even after a checkpoint) would race active writers. The temporary
/// output is owned for this call and cleaned up on success or failure.
fn snapshot_catalog(db: &Path) -> Result<Vec<u8>, AppError> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let snapshot = tempfile::NamedTempFile::new()?;
    let path = snapshot
        .path()
        .to_str()
        .ok_or_else(|| AppError::message("catalog snapshot path is not valid UTF-8"))?;
    conn.execute("VACUUM INTO ?1", [path])?;
    Ok(std::fs::read(snapshot.path())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fs::LocalFs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("agentero-cmirror-{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[tokio::test]
    async fn checkout_creates_and_push_roundtrip() {
        let remote_root = tmp();
        let work = tmp();
        let fs: Arc<dyn VaultFs> = Arc::new(LocalFs::new(&remote_root));

        let mut mirror = CatalogMirror::checkout(fs.clone(), &work).await.unwrap();
        assert!(mirror.work_db_path().is_file());
        let initial_remote = Connection::open_with_flags(
            remote_root.join(CATALOG_REL),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(schema_version(&initial_remote).unwrap(), SCHEMA_VERSION);
        drop(initial_remote);

        // Keep the WAL writer alive throughout both pushes: closing the last
        // connection would checkpoint and let a raw-main-file copy pass.
        let writer = mirror.open().unwrap();
        writer
            .execute_batch(
                "PRAGMA wal_autocheckpoint = 0;
             CREATE TABLE _probe(x INTEGER);
             INSERT INTO _probe VALUES (1);",
            )
            .unwrap();
        let mode: String = writer
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert!(
            std::fs::metadata(work.join(".agentero/catalog.sqlite-wal"))
                .unwrap()
                .len()
                > 0
        );

        // A separate snapshot sees committed data, never the in-flight insert.
        writer
            .execute_batch("BEGIN IMMEDIATE; INSERT INTO _probe VALUES (2);")
            .unwrap();
        mirror.push(fs.clone()).await.unwrap();
        let pulled_work = tmp();
        let pulled = CatalogMirror::checkout(fs.clone(), &pulled_work)
            .await
            .unwrap();
        let reader = pulled.open().unwrap();
        assert_eq!(
            reader
                .query_row("SELECT sum(x) FROM _probe", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(schema_version(&reader).unwrap(), SCHEMA_VERSION);
        drop(reader);
        writer.execute_batch("COMMIT;").unwrap();

        mirror.push(fs.clone()).await.unwrap();
        let pulled_again_work = tmp();
        let pulled_again = CatalogMirror::checkout(fs.clone(), &pulled_again_work)
            .await
            .unwrap();
        let reader = pulled_again.open().unwrap();
        assert_eq!(
            reader
                .query_row("SELECT sum(x) FROM _probe", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            reader
                .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        drop(reader);
        drop(writer);

        // Conflict detection
        std::fs::write(
            remote_root.join(".agentero/catalog.sqlite"),
            b"not-a-real-db-but-different-size-xxxxxx",
        )
        .unwrap();
        // Fix size: write longer content
        let err = mirror.push(fs).await.unwrap_err();
        assert!(err.to_string().contains("conflict") || err.to_string().contains("changed"));

        let _ = std::fs::remove_dir_all(&remote_root);
        let _ = std::fs::remove_dir_all(&work);
        let _ = std::fs::remove_dir_all(&pulled_work);
        let _ = std::fs::remove_dir_all(&pulled_again_work);
    }
}
