use super::ProviderId;
use crate::error::AppError;
use crate::paths::web_ai_db_path;
use crate::sqlite::{open_standard, read_schema_version, write_schema_version, DbMsgs};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;

pub const WEB_AI_SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub vault_key: String,
    pub paper_id: String,
    pub provider_id: ProviderId,
    pub conversation_id: String,
    pub canonical_url: String,
    pub project_id: Option<String>,
    pub title: Option<String>,
    pub updated_at: String,
}

pub struct WebAiStore {
    db_path: std::path::PathBuf,
}

impl Default for WebAiStore {
    fn default() -> Self {
        Self {
            db_path: web_ai_db_path(),
        }
    }
}

impl WebAiStore {
    pub fn at(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            db_path: path.into(),
        }
    }

    pub fn ensure(&self) -> Result<(), AppError> {
        if let Some(parent) = self.db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = open_standard(&self.db_path, DbMsgs::WEB_AI)?;
        migrate(&conn)
    }

    pub fn get_binding(
        &self,
        vault_key: &str,
        paper_id: &str,
        provider_id: ProviderId,
    ) -> Result<Option<Binding>, AppError> {
        let conn = self.open()?;
        conn.query_row(
            "SELECT vault_key, paper_id, provider_id, conversation_id, canonical_url, project_id, title, updated_at
             FROM web_ai_bindings WHERE vault_key = ?1 AND paper_id = ?2 AND provider_id = ?3",
            params![vault_key, paper_id, provider_id.as_str()],
            row_to_binding,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn upsert_binding(&self, binding: &Binding) -> Result<(), AppError> {
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO web_ai_bindings
             (vault_key, paper_id, provider_id, conversation_id, canonical_url, project_id, title, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(vault_key, paper_id, provider_id) DO UPDATE SET
               conversation_id = excluded.conversation_id,
               canonical_url = excluded.canonical_url,
               project_id = excluded.project_id,
               title = excluded.title,
               updated_at = excluded.updated_at",
            params![
                binding.vault_key,
                binding.paper_id,
                binding.provider_id.as_str(),
                binding.conversation_id,
                binding.canonical_url,
                binding.project_id,
                binding.title,
                binding.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_binding(
        &self,
        vault_key: &str,
        paper_id: &str,
        provider_id: ProviderId,
    ) -> Result<bool, AppError> {
        let conn = self.open()?;
        Ok(conn.execute(
            "DELETE FROM web_ai_bindings WHERE vault_key = ?1 AND paper_id = ?2 AND provider_id = ?3",
            params![vault_key, paper_id, provider_id.as_str()],
        )? > 0)
    }

    pub fn delete_provider(&self, provider_id: ProviderId) -> Result<usize, AppError> {
        let conn = self.open()?;
        Ok(conn.execute(
            "DELETE FROM web_ai_bindings WHERE provider_id = ?1",
            [provider_id.as_str()],
        )?)
    }

    fn open(&self) -> Result<Connection, AppError> {
        self.ensure()?;
        open_standard(&self.db_path, DbMsgs::WEB_AI)
    }
}

fn migrate(conn: &Connection) -> Result<(), AppError> {
    let version = read_schema_version(conn, DbMsgs::WEB_AI).unwrap_or(0);
    if version < 1 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS web_ai_bindings (
                vault_key TEXT NOT NULL,
                paper_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL,
                canonical_url TEXT NOT NULL,
                project_id TEXT,
                title TEXT,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (vault_key, paper_id, provider_id)
            );
            CREATE INDEX IF NOT EXISTS idx_web_ai_bindings_provider
                ON web_ai_bindings(provider_id);",
        )?;
        write_schema_version(conn, WEB_AI_SCHEMA_VERSION, DbMsgs::WEB_AI)?;
    }
    Ok(())
}

fn row_to_binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<Binding> {
    let provider: String = row.get(2)?;
    let provider_id = match provider.as_str() {
        "chatgpt" => ProviderId::Chatgpt,
        "gemini" => ProviderId::Gemini,
        "deepseek" => ProviderId::Deepseek,
        "kimi" => ProviderId::Kimi,
        "glm" => ProviderId::Glm,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Binding {
        vault_key: row.get(0)?,
        paper_id: row.get(1)?,
        provider_id,
        conversation_id: row.get(3)?,
        canonical_url: row.get(4)?,
        project_id: row.get(5)?,
        title: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

trait OptionalRow<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn binding() -> Binding {
        Binding {
            vault_key: "vault".into(),
            paper_id: "paper".into(),
            provider_id: ProviderId::Chatgpt,
            conversation_id: "conversation".into(),
            canonical_url: "https://chatgpt.com/c/conversation".into(),
            project_id: None,
            title: Some("Paper chat".into()),
            updated_at: "2026-09-28T00:00:00Z".into(),
        }
    }

    #[test]
    fn binding_round_trips_and_delete_is_scoped() {
        let dir = tempdir().unwrap();
        let store = WebAiStore::at(dir.path().join("web_ai.sqlite"));
        store.upsert_binding(&binding()).unwrap();
        let loaded = store
            .get_binding("vault", "paper", ProviderId::Chatgpt)
            .unwrap();
        assert_eq!(loaded.unwrap().conversation_id, "conversation");
        assert!(store
            .delete_binding("vault", "paper", ProviderId::Chatgpt)
            .unwrap());
        assert!(store
            .get_binding("vault", "paper", ProviderId::Chatgpt)
            .unwrap()
            .is_none());
    }
}
