//! Vault-wide full-text search over notes and saved translations.
//!
//! File bodies stay in an in-process cache keyed by path, mtime, and length.
//! A query still walks the tree so new files appear, but it rereads a file only
//! when that stamp changed. A thousand papers therefore do not reread every
//! note on each keystroke.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024; // skip very large md files
const MAX_DEPTH: usize = 16;
const MAX_FILES: usize = 20_000;
const SNIPPET_CHARS: usize = 200;

struct IndexedDoc {
    modified: Option<SystemTime>,
    len: u64,
    path: String,
    paper_path: Option<String>,
    title: String,
    text_lower: String,
    lines: Vec<String>,
}

struct VaultIndex {
    vault: PathBuf,
    docs: HashMap<PathBuf, IndexedDoc>,
}

fn search_cache() -> &'static Mutex<Option<VaultIndex>> {
    static CACHE: Mutex<Option<VaultIndex>> = Mutex::new(None);
    &CACHE
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VaultSearchArgs {
    pub vault_path: String,
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// Vault-relative md file, e.g. `papers/x/NOTES.md`.
    pub path: String,
    /// Vault-relative paper folder when the hit is inside `papers/…`; else omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_path: Option<String>,
    pub title: String,
    pub snippet: String,
    /// 1-based line of the first matching line (0 when unknown).
    pub line: u32,
    pub score: i64,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VaultSearchResult {
    pub hits: Vec<SearchHit>,
    /// True when more hits existed than `limit`.
    pub truncated: bool,
}

/// Search the Vault's Markdown files for all whitespace-separated terms (AND).
pub fn vault_search(args: VaultSearchArgs) -> Result<VaultSearchResult, AppError> {
    let vault = crate::core::fs::resolve_vault(&args.vault_path)?;

    let terms: Vec<String> = args
        .query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if terms.is_empty() {
        return Ok(VaultSearchResult {
            hits: Vec::new(),
            truncated: false,
        });
    }
    let limit = args.limit.unwrap_or(60).clamp(1, 200);

    let mut files: Vec<PathBuf> = Vec::new();
    collect_search_files(&vault, 0, &mut files);

    let mut guard = search_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let index = guard.get_or_insert_with(|| VaultIndex {
        vault: vault.clone(),
        docs: HashMap::new(),
    });
    if index.vault != vault {
        index.vault = vault.clone();
        index.docs.clear();
    }
    let present: HashMap<&Path, ()> = files.iter().map(|file| (file.as_path(), ())).collect();
    index
        .docs
        .retain(|path, _| present.contains_key(path.as_path()));
    for file in &files {
        refresh_doc(&vault, file, index);
    }

    let mut hits: Vec<SearchHit> = Vec::new();
    for file in &files {
        if let Some(hit) = hit_from_doc(index.docs.get(file), &terms) {
            hits.push(hit);
        }
    }
    drop(guard);
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    let truncated = hits.len() > limit;
    hits.truncate(limit);
    Ok(VaultSearchResult { hits, truncated })
}

fn refresh_doc(vault: &Path, file: &Path, index: &mut VaultIndex) {
    let Ok(meta) = fs::metadata(file) else {
        index.docs.remove(file);
        return;
    };
    if meta.len() > MAX_FILE_BYTES {
        index.docs.remove(file);
        return;
    }
    let modified = meta.modified().ok();
    if let Some(existing) = index.docs.get(file) {
        if existing.len == meta.len() && existing.modified == modified {
            return;
        }
    }
    let Ok(raw) = fs::read_to_string(file) else {
        index.docs.remove(file);
        return;
    };
    let (body, title) = searchable_body(file, &raw);
    let Ok(rel_path) = file.strip_prefix(vault) else {
        return;
    };
    let rel = rel_path.to_string_lossy().replace('\\', "/");
    let lines: Vec<String> = body.lines().map(str::to_string).collect();
    index.docs.insert(
        file.to_path_buf(),
        IndexedDoc {
            modified,
            len: meta.len(),
            paper_path: paper_path_for(&rel),
            path: rel,
            title,
            text_lower: body.to_lowercase(),
            lines,
        },
    );
}

fn hit_from_doc(doc: Option<&IndexedDoc>, terms: &[String]) -> Option<SearchHit> {
    let doc = doc?;
    if !terms
        .iter()
        .all(|term| doc.text_lower.contains(term.as_str()))
    {
        return None;
    }
    let mut line = 0u32;
    let mut snippet = String::new();
    for (i, raw) in doc.lines.iter().enumerate() {
        let lower = raw.to_lowercase();
        if terms.iter().any(|term| lower.contains(term.as_str())) {
            line = (i + 1) as u32;
            snippet = make_snippet(raw, terms);
            break;
        }
    }
    let title_lower = doc.title.to_lowercase();
    let mut score: i64 = 0;
    for term in terms {
        if title_lower.contains(term.as_str()) {
            score += 50;
        }
        score += doc.text_lower.matches(term.as_str()).count().min(20) as i64;
    }
    let fname = doc.path.rsplit('/').next().unwrap_or("");
    if fname.eq_ignore_ascii_case("NOTES.md") || fname.eq_ignore_ascii_case("PAPER.md") {
        score += 5;
    }
    Some(SearchHit {
        paper_path: doc.paper_path.clone(),
        path: doc.path.clone(),
        title: doc.title.clone(),
        snippet,
        line,
        score,
    })
}

fn collect_search_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // `.src/` contains rebuildable Agentero artifacts. Keep the
            // translation cache searchable while leaving metadata, layout
            // indexes, and other internal files hidden from vault search.
            if name == ".src" {
                let sidecar = path.join("layout-translate.json");
                if sidecar.is_file() {
                    out.push(sidecar);
                }
                continue;
            }
            // `source/` is reserved for raw MinerU/LaTeX/layout inputs.
            if name == "source" {
                continue;
            }
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            collect_search_files(&path, depth + 1, out);
        } else if is_search_file(&name) {
            out.push(path);
            if out.len() >= MAX_FILES {
                return;
            }
        }
    }
}

fn is_search_file(name: &str) -> bool {
    is_translation_cache(name)
        || Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn searchable_body(path: &Path, raw: &str) -> (String, String) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if is_translation_cache(name) {
        let lines = translated_lines(raw);
        let body = lines.join("\n");
        let title = if name.eq_ignore_ascii_case("layout-translate.json") {
            path.parent()
                .and_then(|dir| dir.parent())
                .and_then(|dir| dir.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("translation")
                .to_string()
        } else {
            name.trim_end_matches(".layout-translate.json")
                .trim_end_matches(".layout-translate.JSON")
                .to_string()
        };
        return (body, title);
    }
    let title = raw
        .lines()
        .find_map(|line| line.trim().strip_prefix("# ").map(|s| s.trim().to_string()))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string()
        });
    (raw.to_string(), title)
}

fn is_translation_cache(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "layout-translate.json" || lower.ends_with(".layout-translate.json")
}

fn translated_lines(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    collect_translated(&value, &mut lines);
    lines
}

fn collect_translated(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(text)) = map.get("translated") {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
            for child in map.values() {
                if child.is_object() || child.is_array() {
                    collect_translated(child, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_translated(child, out);
            }
        }
        _ => {}
    }
}

/// Center a snippet on the earliest matching term, trimmed to [`SNIPPET_CHARS`].
fn make_snippet(raw: &str, terms: &[String]) -> String {
    // Strip common leading Markdown markers for a cleaner preview.
    let cleaned = raw
        .trim()
        .trim_start_matches(['#', '>', '-', '*', ' '])
        .trim();
    let chars: Vec<char> = cleaned.chars().collect();
    if chars.len() <= SNIPPET_CHARS {
        return cleaned.to_string();
    }

    let lower = cleaned.to_lowercase();
    let byte_pos = terms
        .iter()
        .filter_map(|t| lower.find(t.as_str()))
        .min()
        .unwrap_or(0);
    let char_pos = cleaned[..byte_pos].chars().count();

    let mut start = char_pos.saturating_sub(SNIPPET_CHARS / 3);
    let end = (start + SNIPPET_CHARS).min(chars.len());
    start = end.saturating_sub(SNIPPET_CHARS);
    let mut s: String = chars[start..end].iter().collect();
    if start > 0 {
        s = format!("…{s}");
    }
    if end < chars.len() {
        s = format!("{s}…");
    }
    s
}

fn paper_path_for(rel: &str) -> Option<String> {
    if let Some(folder) = rel.strip_suffix("/.src/layout-translate.json") {
        if folder.starts_with("papers/") {
            return Some(folder.to_string());
        }
    }
    if let Some(folder) = rel.strip_suffix("/.src/citations.json") {
        if folder.starts_with("papers/") {
            return Some(folder.to_string());
        }
    }
    paper_folder_of(rel)
}

/// Vault-relative paper folder for a md path under `papers/…`, else None.
fn paper_folder_of(rel: &str) -> Option<String> {
    if !rel.starts_with("papers/") {
        return None;
    }
    let (parent, _file) = rel.rsplit_once('/')?;
    if parent == "papers" {
        // md directly under papers/ (not a paper folder) → open the file itself.
        return None;
    }
    Some(parent.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    #[test]
    fn finds_terms_and_maps_paper_folder() {
        let root = std::env::temp_dir().join(format!("agentero-search-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write(
            &root,
            "papers/attention/NOTES.md",
            "# Attention Is All You Need\n\nThe transformer uses self-attention.\n",
        );
        write(&root, "notes/idea.md", "Random note about cats.\n");
        write(&root, ".agentero/skip.md", "attention transformer hidden\n");

        let out = vault_search(VaultSearchArgs {
            vault_path: root.to_string_lossy().to_string(),
            query: "transformer attention".into(),
            limit: None,
        })
        .unwrap();

        assert_eq!(out.hits.len(), 1, "only the NOTES.md matches both terms");
        let hit = &out.hits[0];
        assert_eq!(hit.path, "papers/attention/NOTES.md");
        assert_eq!(hit.paper_path.as_deref(), Some("papers/attention"));
        assert_eq!(hit.title, "Attention Is All You Need");
        assert!(hit.line >= 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_query_returns_nothing() {
        let root = std::env::temp_dir().join(format!("agentero-search-e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let out = vault_search(VaultSearchArgs {
            vault_path: root.to_string_lossy().to_string(),
            query: "   ".into(),
            limit: None,
        })
        .unwrap();
        assert!(out.hits.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn searches_hidden_translation_cache_without_indexing_raw_source() {
        let root = std::env::temp_dir().join(format!(
            "agentero-search-translation-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        write(
            &root,
            "papers/attention/.src/layout-translate.json",
            r#"{"items":[{"translated":"Hidden transformer result"}]}"#,
        );
        write(
            &root,
            "papers/attention/source/layout.json",
            r#"{"regions":[{"text":"Hidden transformer result"}]}"#,
        );

        let out = vault_search(VaultSearchArgs {
            vault_path: root.to_string_lossy().to_string(),
            query: "hidden transformer".into(),
            limit: None,
        })
        .unwrap();

        assert_eq!(out.hits.len(), 1);
        assert_eq!(
            out.hits[0].path,
            "papers/attention/.src/layout-translate.json"
        );
        assert_eq!(out.hits[0].paper_path.as_deref(), Some("papers/attention"));
        let _ = fs::remove_dir_all(&root);
    }

    /// Quantified regression guard for the async command migration: build a
    /// few-hundred-file vault, record the direct-call timing baseline, then run
    /// the identical search through `run_blocking` (the async command path) and
    /// assert the heavy IO executed off the calling thread — on Windows the
    /// calling thread of a sync command is the UI message pump.
    #[test]
    fn vault_search_runs_off_the_calling_thread_with_timing_baseline() {
        let root =
            std::env::temp_dir().join(format!("agentero-search-bench-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        const FILES: usize = 300;
        for i in 0..FILES {
            let needle = if i % 3 == 0 { "transformer" } else { "cats" };
            write(
                &root,
                &format!("notes/note-{i}.md"),
                &format!(
                    "# Note {i}\n\nBody line about {needle}.\n{}\n",
                    "filler line for realistic file size.\n".repeat(40)
                ),
            );
        }

        // Baseline: direct implementation call on this thread.
        let started = std::time::Instant::now();
        let direct = vault_search(VaultSearchArgs {
            vault_path: root.to_string_lossy().to_string(),
            query: "transformer".into(),
            limit: Some(200),
        })
        .expect("direct search");
        let direct_ms = started.elapsed().as_millis();
        eprintln!(
            "bench vault_search direct: files={FILES} hits={} truncated={} elapsed_ms={direct_ms}",
            direct.hits.len(),
            direct.truncated
        );
        assert_eq!(direct.hits.len(), 100, "every third file matches");

        // Async command path: same work, must run on a blocking-pool thread.
        let caller = std::thread::current().id();
        let vault_path = root.to_string_lossy().to_string();
        let result = tauri::async_runtime::block_on(crate::core::blocking::run_blocking(
            move || {
                let worker = std::thread::current();
                let off_thread = worker.id() != caller;
                let started = std::time::Instant::now();
                let out = vault_search(VaultSearchArgs {
                    vault_path,
                    query: "transformer".into(),
                    limit: Some(200),
                })
                .expect("blocking search");
                eprintln!(
                    "bench vault_search run_blocking: worker id={:?} name={:?} hits={} elapsed_ms={}",
                    worker.id(),
                    worker.name(),
                    out.hits.len(),
                    started.elapsed().as_millis()
                );
                crate::core::error::ApiResult::ok((off_thread, out.hits.len()))
            },
        ));
        let (off_thread, hits) = result.data.expect("search result data");
        assert!(
            off_thread,
            "vault_search must execute on the blocking pool, not the calling thread"
        );
        assert_eq!(hits, direct.hits.len(), "same results through both paths");
        let _ = fs::remove_dir_all(&root);
    }
}

/// Tauri command shells for this feature.
pub mod commands;
