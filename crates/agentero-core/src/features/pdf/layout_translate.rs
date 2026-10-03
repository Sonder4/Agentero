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
    pub skipped_regions: Vec<Value>,
    pub failed: usize,
    pub provider: String,
    pub target_lang: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// Keep the existing CLI call boundary explicit (paper, language, service and workers).
#[allow(clippy::too_many_arguments)]
pub async fn translate_paper_dir(
    vault: &Path,
    paper_rel: &str,
    target_lang: &str,
    source_lang: &str,
    provider: &str,
    concurrency: usize,
    force: bool,
    provider_config: Option<&crate::features::translate::PersistedTranslateProviderConfig>,
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
        && !crate::features::translate::COMMERCIAL_PROVIDERS.contains(&provider_id.as_str())
    {
        return Err(AppError::message(format!(
            "unsupported layout translation provider: {provider_id}"
        )));
    }
    let service_key = if provider_id == "agent" {
        let settings = crate::features::translate::load_persisted_translate_settings();
        format!(
            "agent:{}:{}",
            settings
                .as_ref()
                .map(|s| s.agent_id.trim())
                .filter(|v| !v.is_empty())
                .unwrap_or("default"),
            settings
                .as_ref()
                .map(|s| s.model_id.trim())
                .filter(|v| !v.is_empty())
                .unwrap_or("default")
        )
    } else if crate::features::translate::COMMERCIAL_PROVIDERS.contains(&provider_id.as_str()) {
        let cfg = provider_config;
        [
            provider_id.as_str(),
            cfg.map(|c| c.base_url.trim()).unwrap_or(""),
            cfg.map(|c| c.region.trim()).unwrap_or(""),
            cfg.map(|c| c.model.trim()).unwrap_or(""),
        ]
        .join(":")
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
    let mut skipped_regions = Vec::new();
    for unit in units {
        if unit.kind == "header" && is_math_accent_fragment(&unit.source) {
            skipped += 1;
            skipped_regions.push(json!({
                "id": unit.id, "pageIndex": unit.page_index,
                "source": unit.source, "reason": "math-accent-fragment",
            }));
            continue;
        }
        if is_chinese_target(target_lang)
            && crate::features::translate::looks_mostly_cjk(&unit.source)
        {
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
            &skipped_regions,
        )?;
        return Ok(Some(LayoutTranslateOutcome {
            paper_path: paper_rel.to_string(),
            from_cache: true,
            translated: translated.len(),
            skipped,
            skipped_regions,
            failed: 0,
            provider: provider_id,
            target_lang: target_lang.to_string(),
            error: None,
        }));
    }

    let mut failed = 0usize;
    let mut first_error: Option<String> = None;
    let workers = concurrency.clamp(1, 8);
    let groups = if provider_id == "agent" {
        batches(&pending)
    } else {
        pending.iter().cloned().map(|unit| vec![unit]).collect()
    };
    let first_pass = translate_batches(
        &groups,
        workers,
        source_lang,
        target_lang,
        &provider_id,
        vault,
        provider_config,
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
            provider_config,
        )
        .await;
        for (unit, result) in retry_units.iter().zip(retried) {
            match result {
                Ok(texts) => match texts
                    .first()
                    .map(|text| text.trim())
                    .filter(|text| !text.is_empty())
                {
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
        &skipped_regions,
    )?;
    Ok(Some(LayoutTranslateOutcome {
        paper_path: paper_rel.to_string(),
        from_cache: false,
        translated: translated.len(),
        skipped,
        skipped_regions,
        failed,
        provider: provider_id,
        target_lang: target_lang.to_string(),
        error: first_error,
    }))
}

fn is_chinese_target(target: &str) -> bool {
    let target = target.trim().to_ascii_lowercase();
    target == "zh" || target.starts_with("zh-") || target.starts_with("zh_")
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
        skipped_regions: Vec::new(),
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
    let language_matches = source
        .and_then(|s| s.get("sourceLang"))
        .and_then(Value::as_str)
        == Some(source_lang)
        && source
            .and_then(|s| s.get("targetLang"))
            .and_then(Value::as_str)
            == Some(target_lang);
    let exact_matches = language_matches
        && source
            .and_then(|s| s.get("providerId"))
            .and_then(Value::as_str)
            == Some(provider)
        && source
            .and_then(|s| s.get("serviceKey"))
            .and_then(Value::as_str)
            == Some(service_key);
    // The file-level source describes every item. Reusing another provider's
    // result here would relabel old translations as the newly selected service.
    if raw.get("schemaVersion").and_then(Value::as_u64) != Some(TRANSLATE_SCHEMA)
        || source.and_then(|s| s.get("mode")).and_then(Value::as_str)
            != Some("pdf-layout-translate")
        || !exact_matches
    {
        return Vec::new();
    }
    raw.get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("id").and_then(Value::as_str).is_some()
                        && item.get("source").and_then(Value::as_str).is_some()
                        && item
                            .get("translated")
                            .and_then(Value::as_str)
                            .is_some_and(|text| !text.trim().is_empty())
                })
                .cloned()
                .collect()
        })
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
    provider_config: Option<&crate::features::translate::PersistedTranslateProviderConfig>,
) -> Vec<Result<Vec<String>, AppError>> {
    use futures_util::stream::{self, StreamExt};

    let source_lang = source_lang.to_string();
    let target_lang = target_lang.to_string();
    let provider = provider.to_string();
    let cwd = cwd.to_path_buf();
    let provider_config = provider_config.cloned();
    let mut results = stream::iter(groups.iter().cloned().enumerate())
        .map(|(index, batch)| {
            let source_lang = source_lang.clone();
            let target_lang = target_lang.clone();
            let provider = provider.clone();
            let cwd = cwd.clone();
            let provider_config = provider_config.clone();
            async move {
                (
                    index,
                    translate_batch(
                        &batch,
                        &source_lang,
                        &target_lang,
                        &provider,
                        &cwd,
                        provider_config.as_ref(),
                    )
                    .await,
                )
            }
        })
        .buffer_unordered(workers)
        .collect::<Vec<_>>()
        .await;
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

fn split_source_chunks(source: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in source.split_whitespace() {
        let extra = if current.is_empty() { 0 } else { 1 };
        if !current.is_empty() && current.chars().count() + extra + word.chars().count() > max_chars
        {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

async fn translate_batch(
    batch: &[Unit],
    source_lang: &str,
    target_lang: &str,
    provider: &str,
    cwd: &Path,
    provider_config: Option<&crate::features::translate::PersistedTranslateProviderConfig>,
) -> Result<Vec<String>, AppError> {
    if provider != "agent" {
        let mut translated = Vec::with_capacity(batch.len());
        for unit in batch {
            let chunks = split_source_chunks(
                &unit.source,
                crate::features::translate::MAX_TEXT_CHARS - 200,
            );
            let mut joined = String::new();
            for chunk in chunks {
                let mut chunk_result = None;
                for attempt in 0..3 {
                    match crate::features::translate::translate_text(
                        crate::features::translate::TranslateTextArgs {
                            text: chunk.clone(),
                            source_lang: source_lang.to_string(),
                            target_lang: target_lang.to_string(),
                            provider: provider.to_string(),
                            api_key: provider_config
                                .map(|c| c.api_key.clone())
                                .filter(|v| !v.trim().is_empty()),
                            base_url: provider_config
                                .map(|c| c.base_url.clone())
                                .filter(|v| !v.trim().is_empty()),
                            region: provider_config
                                .map(|c| c.region.clone())
                                .filter(|v| !v.trim().is_empty()),
                            model: provider_config
                                .map(|c| c.model.clone())
                                .filter(|v| !v.trim().is_empty()),
                            custom_prompt: None,
                            timeout_ms: Some(30_000),
                        },
                    )
                    .await
                    {
                        Ok(result) => {
                            chunk_result = Some(result.text);
                            break;
                        }
                        Err(_) if attempt < 2 => {
                            tokio::time::sleep(std::time::Duration::from_millis(
                                300 * (1u64 << attempt),
                            ))
                            .await;
                        }
                        Err(err) => return Err(err),
                    }
                }
                if let Some(result) = chunk_result {
                    if !joined.is_empty() {
                        joined.push(' ');
                    }
                    joined.push_str(&result);
                }
            }
            translated.push(joined);
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
        prompt.push('\n');
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
    skipped_regions: &[Value],
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
        "skippedRegions": skipped_regions,
    });
    let dir = paper_dir.join(".src");
    fs::create_dir_all(&dir)?;
    json_store(&dir.join("layout-translate.json"), &sidecar)?;
    Ok(())
}

fn is_math_accent_fragment(text: &str) -> bool {
    let mut chars = text.trim().chars().filter(|c| !c.is_whitespace());
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic()
        || ('α'..='ω').contains(&first)
        || ('Α'..='Ω').contains(&first))
    {
        return false;
    }
    let accents: Vec<char> = chars.collect();
    !accents.is_empty()
        && accents.iter().all(|c| {
            matches!(c, '¯' | 'ˆ' | 'ˇ' | '˙' | '˜') || ('\u{0300}'..='\u{036f}').contains(c)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_detached_math_accents_without_skipping_short_headings() {
        assert!(is_math_accent_fragment("b ¯"));
        assert!(is_math_accent_fragment("θ\u{0304}"));
        assert!(!is_math_accent_fragment("AI"));
        assert!(!is_math_accent_fragment("1 Introduction"));
        assert!(!is_math_accent_fragment("bias b ¯"));
    }

    #[test]
    fn cache_requires_real_provider_and_language_identity() {
        let dir =
            std::env::temp_dir().join(format!("agentero-translate-cache-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");
        fs::write(
            &path,
            json!({
                "schemaVersion": 1,
                "source": {"mode": "pdf-layout-translate", "providerId": "tencenttransmart",
                    "sourceLang": "auto", "targetLang": "zh-CN", "serviceKey": "tencenttransmart"},
                "items": [{"id": "a", "source": "Hello", "translated": "你好"}]
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            load_cache(
                &path,
                "tencenttransmart",
                "auto",
                "zh-CN",
                "tencenttransmart"
            )
            .len(),
            1
        );
        assert!(load_cache(&path, "googleapi", "auto", "zh-CN", "googleapi").is_empty());
        assert!(load_cache(&path, "tencenttransmart", "auto", "en", "tencenttransmart").is_empty());
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
