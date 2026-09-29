//! Full-text translation of layout body regions.
//!
//! Reads `{paper}/source/layout.json`, translates text / abstract / header /
//! caption regions with the free machine-translation engines, and writes
//! `{paper}/.src/layout-translate.json` in the viewer's sidecar schema.
//! Cached items whose source text still matches are reused.

use crate::error::AppError;
use crate::features::translate::{self, TranslateTextArgs, FREE_PROVIDERS};
use crate::fs::json_store;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const TRANSLATE_SCHEMA: u64 = 1;
const BATCH_CHARS: usize = 4500;

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
}

pub async fn translate_paper_dir(
    vault: &Path,
    paper_rel: &str,
    target_lang: &str,
    source_lang: &str,
    provider: Option<&str>,
    force: bool,
) -> Result<Option<LayoutTranslateOutcome>, AppError> {
    let paper_dir = vault.join(paper_rel);
    let raw_path = paper_dir.join("source").join("layout.json");
    if !raw_path.is_file() {
        return Ok(None);
    }
    let provider_id = provider.unwrap_or("tencenttransmart").to_string();
    if !FREE_PROVIDERS.contains(&provider_id.as_str()) {
        return Err(AppError::message(format!(
            "layout translation provider must be one of {}",
            FREE_PROVIDERS.join("|")
        )));
    }
    let units = load_units(&raw_path)?;
    if units.is_empty() {
        return Ok(Some(empty_outcome(
            paper_rel,
            true,
            &provider_id,
            target_lang,
        )));
    }

    let service_key = provider_id.clone();
    let sidecar_path = paper_dir.join(".src").join("layout-translate.json");
    let cached = if force {
        Vec::new()
    } else {
        load_cache(&sidecar_path, &provider_id, source_lang, target_lang, &service_key)
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
        }));
    }

    let mut failed = 0usize;
    for batch in batches(&pending) {
        match translate_batch(&batch, &provider_id, source_lang, target_lang).await {
            Ok(texts) => {
                for (unit, text) in batch.iter().zip(texts) {
                    let text = text.trim();
                    if text.is_empty() {
                        failed += 1;
                    } else {
                        translated.push(item_json(unit, text));
                    }
                }
            }
            Err(_) => {
                for unit in &batch {
                    match translate_one(&unit.source, &provider_id, source_lang, target_lang).await
                    {
                        Ok(text) if !text.trim().is_empty() => {
                            translated.push(item_json(unit, text.trim()))
                        }
                        _ => failed += 1,
                    }
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

async fn translate_batch(
    batch: &[Unit],
    provider: &str,
    source_lang: &str,
    target_lang: &str,
) -> Result<Vec<String>, AppError> {
    if batch.len() == 1 {
        return Ok(vec![
            translate_one(&batch[0].source, provider, source_lang, target_lang).await?,
        ]);
    }
    let mut payload = String::new();
    for (index, unit) in batch.iter().enumerate() {
        payload.push_str(&format!("[[{index}]] {}\n", unit.source));
    }
    let translated = translate_one(&payload, provider, source_lang, target_lang).await?;
    let mut parts = vec![String::new(); batch.len()];
    let mut current: Option<usize> = None;
    for line in translated.lines() {
        if let Some(rest) = line.trim().strip_prefix("[[") {
            if let Some((n, after)) = rest.split_once("]]") {
                if let Ok(index) = n.trim().parse::<usize>() {
                    if index < parts.len() {
                        current = Some(index);
                        let text = after.trim();
                        if !text.is_empty() {
                            push_part(&mut parts[index], text);
                        }
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
        return Err(AppError::message("batch translation markers were incomplete"));
    }
    Ok(parts)
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

async fn translate_one(
    text: &str,
    provider: &str,
    source_lang: &str,
    target_lang: &str,
) -> Result<String, AppError> {
    let mut start = 0usize;
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    while start < chars.len() {
        let end = (start + translate::MAX_TEXT_CHARS).min(chars.len());
        let slice: String = chars[start..end].iter().collect();
        let result = translate::translate_text(TranslateTextArgs {
            text: slice,
            source_lang: source_lang.to_string(),
            target_lang: target_lang.to_string(),
            provider: provider.to_string(),
            api_key: None,
            base_url: None,
            region: None,
            model: None,
            custom_prompt: None,
            timeout_ms: Some(30_000),
        })
        .await?;
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(result.text.trim());
        start = end;
    }
    Ok(out)
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
