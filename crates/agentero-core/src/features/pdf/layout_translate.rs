//! Full-text translation of layout body regions.
//!
//! Reads `{paper}/source/layout.json`, translates text / abstract / header /
//! caption regions through the local Pi agent, and writes
//! `{paper}/.src/layout-translate.json` in the viewer's sidecar schema.
//! Cached items whose source text still matches are reused.

use crate::error::AppError;
use crate::features::pdf::pi_agent;
use crate::fs::json_store;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const TRANSLATE_SCHEMA: u64 = 1;
const BATCH_CHARS: usize = 4500;
const DEFAULT_CONCURRENCY: usize = 4;

pub fn default_concurrency() -> usize {
    DEFAULT_CONCURRENCY
}

#[derive(Debug, Clone)]
struct Unit {
    id: String,
    page_index: u64,
    kind: String,
    reading_order: u64,
    bbox: Value,
    source: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutTranslateOutcome {
    pub paper_path: String,
    pub from_cache: bool,
    pub translated: usize,
    pub skipped: usize,
    pub failed: usize,
    pub provider: String,
    pub target_lang: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn translate_paper_dir(
    vault: &Path,
    paper_rel: &str,
    target_lang: &str,
    source_lang: &str,
    provider: &str,
    concurrency: usize,
    force: bool,
) -> Result<Option<LayoutTranslateOutcome>, AppError> {
    let paper_dir = vault.join(paper_rel);
    let raw_path = paper_dir.join("source").join("layout.json");
    if !raw_path.is_file() {
        return Ok(None);
    }
    let provider_id = provider.trim().to_ascii_lowercase();
    if provider_id.is_empty() {
        return Err(AppError::message("translation provider cannot be empty"));
    }
    if provider_id != "agent"
        && !crate::features::translate::FREE_PROVIDERS.contains(&provider_id.as_str())
    {
        return Err(AppError::message(format!(
            "unsupported layout translation provider: {provider_id}"
        )));
    }
    let service_key = if provider_id == "agent" {
        "agent:pi:default".to_string()
    } else {
        provider_id.clone()
    };
    let units = load_units(&raw_path)?;
    if units.is_empty() {
        write_sidecar(
            &paper_dir,
            &provider_id,
            source_lang,
            target_lang,
            &service_key,
            &[],
        )?;
        return Ok(Some(empty_outcome(
            paper_rel,
            !force,
            &provider_id,
            target_lang,
        )));
    }

    let sidecar_path = paper_dir.join(".src").join("layout-translate.json");
    let service_key_for_cache = service_key.clone();
    let cached = if force {
        Vec::new()
    } else {
        load_cache(
            &sidecar_path,
            &provider_id,
            source_lang,
            target_lang,
            &service_key_for_cache,
        )
    };
    let mut translated = Vec::new();
    let mut pending = Vec::new();
    let mut skipped = 0usize;
    for unit in units {
        if crate::features::translate::looks_mostly_cjk(&unit.source) {
            skipped += 1;
            translated.push(item_json(&unit, &unit.source));
            continue;
        }
        if let Some(hit) = cached.iter().find(|item| {
            item.get("id").and_then(Value::as_str) == Some(unit.id.as_str())
                && item.get("source").and_then(Value::as_str) == Some(unit.source.as_str())
        }) {
            translated.push(hit.clone());
            continue;
        }
        pending.push(unit);
    }
    if pending.is_empty() {
        write_sidecar(
            &paper_dir,
            &provider_id,
            source_lang,
            target_lang,
            &service_key,
            &translated,
        )?;
        return Ok(Some(LayoutTranslateOutcome {
            paper_path: paper_rel.to_string(),
            from_cache: true,
            translated: translated.len(),
            skipped,
            failed: 0,
            provider: provider_id,
            target_lang: target_lang.to_string(),
            error: None,
        }));
    }

    let mut failed = 0usize;
    let mut first_error: Option<String> = None;
    let workers = concurrency.clamp(1, 8);
    let groups = batches(&pending);
    let first_pass = translate_batches(
        &groups,
        workers,
        source_lang,
        target_lang,
        &provider_id,
        vault,
    )
    .await;
    let mut retry_units = Vec::new();
    for (batch, result) in groups.iter().zip(first_pass) {
        match result {
            Ok(texts) => {
                for (unit, text) in batch.iter().zip(texts) {
                    let text = text.trim();
                    if text.is_empty() {
                        retry_units.push(unit.clone());
                    } else {
                        translated.push(item_json(unit, text));
                    }
                }
            }
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(err.to_string());
                }
                if is_retryable_translation_error(&err) {
                    retry_units.extend(batch.iter().cloned());
                } else {
                    failed += batch.len();
                }
            }
        }
    }
    if !retry_units.is_empty() {
        let singles: Vec<Vec<Unit>> = retry_units.iter().cloned().map(|unit| vec![unit]).collect();
        let retried = translate_batches(
            &singles,
            workers,
            source_lang,
            target_lang,
            &provider_id,
            vault,
        )
        .await;
        for (unit, result) in retry_units.iter().zip(retried) {
            match result {
                Ok(texts) => match texts.first().map(|text| text.trim()).filter(|text| !text.is_empty()) {
                    Some(text) => translated.push(item_json(unit, text)),
                    None => failed += 1,
                },
                Err(err) => {
                    if first_error.is_none() {
                        first_error = Some(err.to_string());
                    }
                    failed += 1;
                }
            }
        }
    }
    translated.sort_by(|a, b| {
        let page = json_u64(a, "pageIndex").cmp(&json_u64(b, "pageIndex"));
        page.then(json_u64(a, "readingOrder").cmp(&json_u64(b, "readingOrder")))
    });
    write_sidecar(
        &paper_dir,
        &provider_id,
        source_lang,
        target_lang,
        &service_key,
        &translated,
    )?;
    Ok(Some(LayoutTranslateOutcome {
        paper_path: paper_rel.to_string(),
        from_cache: false,
        translated: translated.len(),
        skipped,
        failed,
        provider: provider_id,
        target_lang: target_lang.to_string(),
        error: first_error,
    }))
}

fn empty_outcome(
    paper_rel: &str,
    from_cache: bool,
    provider: &str,
    target: &str,
) -> LayoutTranslateOutcome {
    LayoutTranslateOutcome {
        paper_path: paper_rel.to_string(),
        from_cache,
        translated: 0,
        skipped: 0,
        failed: 0,
        provider: provider.to_string(),
        target_lang: target.to_string(),
        error: None,
    }
}

fn load_units(path: &Path) -> Result<Vec<Unit>, AppError> {
    let text = fs::read_to_string(path)
        .map_err(|e| AppError::message(format!("read layout.json: {e}")))?;
    let raw: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::message(format!("invalid layout.json: {e}")))?;
    let Some(regions) = raw.get("regions").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut units = Vec::new();
    for region in regions {
        let kind = region.get("kind").and_then(Value::as_str).unwrap_or("");
        if !crate::features::pdf::layout_text::is_translatable(kind) {
            continue;
        }
        let source = region
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if source.chars().count() < 2 {
            continue;
        }
        let Some(id) = region.get("id").and_then(Value::as_str) else {
            continue;
        };
        units.push(Unit {
            id: id.to_string(),
            page_index: region.get("pageIndex").and_then(Value::as_u64).unwrap_or(0),
            kind: kind.to_string(),
            reading_order: region
                .get("readingOrder")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            bbox: region.get("bbox").cloned().unwrap_or(json!({})),
            source,
        });
    }
    units.sort_by_key(|unit| (unit.page_index, unit.reading_order));
    Ok(units)
}

fn load_cache(
    path: &Path,
    provider: &str,
    source_lang: &str,
    target_lang: &str,
    service_key: &str,
) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(raw) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let source = raw.get("source");
    let matches = source.and_then(|s| s.get("providerId")).and_then(Value::as_str) == Some(provider)
        && source.and_then(|s| s.get("sourceLang")).and_then(Value::as_str) == Some(source_lang)
        && source.and_then(|s| s.get("targetLang")).and_then(Value::as_str) == Some(target_lang)
        && source.and_then(|s| s.get("serviceKey")).and_then(Value::as_str) == Some(service_key);
    if !matches {
        return Vec::new();
    }
    raw.get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn batches(units: &[Unit]) -> Vec<Vec<Unit>> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    let mut chars = 0usize;
    for unit in units {
        let len = unit.source.chars().count() + 12;
        if !current.is_empty() && chars + len > BATCH_CHARS {
            out.push(std::mem::take(&mut current));
            chars = 0;
        }
        chars += len;
        current.push(unit.clone());
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

async fn translate_batches(
    groups: &[Vec<Unit>],
    workers: usize,
    source_lang: &str,
    target_lang: &str,
    provider: &str,
    cwd: &Path,
) -> Vec<Result<Vec<String>, AppError>> {
    use futures_util::stream::{self, StreamExt};

    let source_lang = source_lang.to_string();
    let target_lang = target_lang.to_string();
    let provider = provider.to_string();
    let cwd = cwd.to_path_buf();
    let mut results = stream::iter(groups.iter().cloned().enumerate())
        .map(|(index, batch)| {
            let source_lang = source_lang.clone();
            let target_lang = target_lang.clone();
            let provider = provider.clone();
            let cwd = cwd.clone();
            async move {
                (
                    index,
                    translate_batch(&batch, &source_lang, &target_lang, &provider, &cwd).await,
                )
            }
        })
        .buffer_unordered(workers)
        .collect::<Vec<_>>()
        .await;
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

async fn translate_batch(
    batch: &[Unit],
    source_lang: &str,
    target_lang: &str,
    provider: &str,
    cwd: &Path,
) -> Result<Vec<String>, AppError> {
    if provider != "agent" {
        let mut translated = Vec::with_capacity(batch.len());
        for unit in batch {
            let result = crate::features::translate::translate_text(
                crate::features::translate::TranslateTextArgs {
                    text: unit.source.clone(),
                    source_lang: source_lang.to_string(),
                    target_lang: target_lang.to_string(),
                    provider: provider.to_string(),
                    api_key: None,
                    base_url: None,
                    region: None,
                    model: None,
                    custom_prompt: None,
                    timeout_ms: Some(30_000),
                },
            )
            .await?;
            translated.push(result.text);
        }
        return Ok(translated);
    }
    let prompt = translation_prompt(batch, source_lang, target_lang);
    let translated = pi_agent::translate_with_pi(&prompt, cwd).await?;
    if batch.len() == 1 {
        return Ok(vec![translated]);
    }
    split_numbered(&translated, batch.len())
}

fn translation_prompt(batch: &[Unit], source_lang: &str, target_lang: &str) -> String {
    let mut prompt = format!(
        "You are a professional academic translator. Translate the text below from {source_lang} into {target_lang}.\n\
Rules:\n\
- Write natural {target_lang}. Keep mathematics, symbols, variable names, units, code, URLs and citation markers unchanged.\n\
- Do not add, drop, summarize or explain anything. Output only the translation.\n"
    );
    if batch.len() == 1 {
        prompt.push_str("\n");
        prompt.push_str(&batch[0].source);
        return prompt;
    }
    prompt.push_str(
        "- The text has several paragraphs prefixed with [[n]]. Keep the same markers, in order, and do not merge paragraphs.\n\n",
    );
    for (index, unit) in batch.iter().enumerate() {
        prompt.push_str(&format!("[[{index}]] {}\n\n", unit.source));
    }
    prompt
}

fn split_numbered(translated: &str, expected: usize) -> Result<Vec<String>, AppError> {
    let mut parts = vec![String::new(); expected];
    let mut current: Option<usize> = None;
    for line in translated.lines() {
        if let Some(rest) = line.trim().strip_prefix("[[") {
            if let Some((n, after)) = rest.split_once("]]") {
                if let Ok(index) = n.trim().parse::<usize>() {
                    if index < parts.len() {
                        current = Some(index);
                        push_part(&mut parts[index], after.trim());
                        continue;
                    }
                }
            }
        }
        if let Some(index) = current {
            push_part(&mut parts[index], line.trim());
        }
    }
    if parts.iter().any(|part| part.trim().is_empty()) {
        return Err(AppError::message(
            "pi batch translation markers were incomplete",
        ));
    }
    Ok(parts)
}

fn is_retryable_translation_error(error: &AppError) -> bool {
    let message = error.to_string();
    !message.starts_with("pi translation error:") && !message.starts_with("pi exited ")
}

fn push_part(slot: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !slot.is_empty() {
        slot.push(' ');
    }
    slot.push_str(text);
}

fn item_json(unit: &Unit, translated: &str) -> Value {
    json!({
        "id": unit.id,
        "pageIndex": unit.page_index,
        "bbox": unit.bbox,
        "kind": unit.kind,
        "readingOrder": unit.reading_order,
        "source": unit.source,
        "translated": translated,
    })
}

fn json_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn write_sidecar(
    paper_dir: &Path,
    provider: &str,
    source_lang: &str,
    target_lang: &str,
    service_key: &str,
    items: &[Value],
) -> Result<(), AppError> {
    let generated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let sidecar = json!({
        "schemaVersion": TRANSLATE_SCHEMA,
        "source": {
            "mode": "pdf-layout-translate",
            "generatedAt": generated_at,
            "providerId": provider,
            "sourceLang": source_lang,
            "targetLang": target_lang,
            "serviceKey": service_key,
        },
        "items": items,
    });
    let dir = paper_dir.join(".src");
    fs::create_dir_all(&dir)?;
    json_store(&dir.join("layout-translate.json"), &sidecar)?;
    Ok(())
}
