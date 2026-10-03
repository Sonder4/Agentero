//! Pi coding agent runner for headless layout translation.
//!
//! Spawns `pi --mode json` with the user's configured default provider and
//! model (no `--provider` / `--model` override). Tools and project context
//! files are disabled so a translation cannot edit the vault or spend tokens
//! on AGENTS.md.

use crate::error::AppError;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::timeout;

const PI_TIMEOUT: Duration = Duration::from_secs(180);

pub async fn translate_with_pi(prompt: &str, cwd: &Path) -> Result<String, AppError> {
    let program = pi_program();
    let mut command = Command::new(&program);
    command
        .arg("--mode")
        .arg("json")
        .arg("--print")
        .arg("--no-session")
        .arg("--no-tools")
        .arg("--no-extensions")
        .arg("--no-skills")
        .arg("--no-context-files")
        .arg("--thinking")
        .arg("off")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
    }

    let mut child = command
        .spawn()
        .map_err(|e| AppError::message(format!("failed to start pi ({program}): {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| AppError::message(format!("write pi prompt: {e}")))?;
    }
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::message("pi stdout was not piped"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::message("pi stderr was not piped"))?;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let status = match timeout(PI_TIMEOUT, child.wait()).await {
        Ok(result) => {
            result.map_err(|e| AppError::message(format!("pi translation failed: {e}")))?
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(AppError::message(
                "pi translation timed out after 180s; process terminated",
            ));
        }
    };
    let stdout = stdout_task
        .await
        .map_err(|e| AppError::message(format!("read pi stdout task: {e}")))?
        .map_err(|e| AppError::message(format!("read pi stdout: {e}")))?;
    let stderr = stderr_task
        .await
        .map_err(|e| AppError::message(format!("read pi stderr task: {e}")))?
        .map_err(|e| AppError::message(format!("read pi stderr: {e}")))?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        let tail = stderr.chars().rev().take(400).collect::<String>();
        let tail = tail.chars().rev().collect::<String>();
        return Err(AppError::message(format!("pi exited {status}: {tail}")));
    }
    parse_pi_text(&stdout)
}

fn pi_program() -> String {
    if let Ok(explicit) = std::env::var("AGENTERO_PI") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if cfg!(windows) {
        "pi.cmd".to_string()
    } else {
        "pi".to_string()
    }
}

fn parse_pi_text(stdout: &[u8]) -> Result<String, AppError> {
    let text = String::from_utf8_lossy(stdout);
    let mut last = String::new();
    let mut last_error = String::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("agent_end") {
            continue;
        }
        let Some(messages) = value.get("messages").and_then(Value::as_array) else {
            continue;
        };
        for message in messages.iter().rev() {
            if message.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            if let Some(error) = message.get("errorMessage").and_then(Value::as_str) {
                last_error = error.to_string();
            }
            let Some(parts) = message.get("content").and_then(Value::as_array) else {
                continue;
            };
            let mut assembled = String::new();
            for part in parts {
                if part.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        if !assembled.is_empty() {
                            assembled.push('\n');
                        }
                        assembled.push_str(text);
                    }
                }
            }
            if !assembled.trim().is_empty() {
                last = assembled;
                break;
            }
        }
    }
    let last = last.trim().to_string();
    if !last.is_empty() {
        Ok(last)
    } else if !last_error.is_empty() {
        Err(AppError::message(format!(
            "pi translation error: {last_error}"
        )))
    } else {
        Err(AppError::message("pi returned no translation"))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_pi_text;

    #[test]
    fn reads_the_assistant_text_from_agent_end() {
        let line = r#"{"type":"agent_end","messages":[{"role":"user","content":[{"type":"text","text":"src"}]},{"role":"assistant","content":[{"type":"text","text":"译文"}]}]}"#;
        assert_eq!(parse_pi_text(line.as_bytes()).unwrap(), "译文");
    }

    #[test]
    fn reports_provider_errors() {
        let line = r#"{"type":"agent_end","messages":[{"role":"assistant","content":[],"errorMessage":"401: invalid key"}]}"#;
        assert_eq!(
            parse_pi_text(line.as_bytes()).unwrap_err().to_string(),
            "pi translation error: 401: invalid key"
        );
    }
}
