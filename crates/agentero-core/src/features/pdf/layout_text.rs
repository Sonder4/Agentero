//! PDFium text-layer layout for headless CLI analysis.
//!
//! The desktop app detects figures with an ONNX model inside a WebView. The
//! CLI cannot host that model, so this module reconstructs reading-order body
//! blocks from the PDF text layer and writes the same sidecars the viewer
//! already consumes:
//!
//! - `{paper}/source/layout.json` — raw regions (`schemaVersion` 3)
//! - `{paper}/.src/layout-index.json` — sidebar index (`schemaVersion` 1)
//!
//! Image-only pages produce no blocks. An existing model sidecar is left
//! untouched unless the caller forces a rewrite.

use crate::error::AppError;
use crate::fs::json_store;
use liteparse_pdfium::{CharBox, Library, RectF};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const LAYOUT_SCHEMA: u64 = 3;
const INDEX_SCHEMA: u64 = 1;
const MIN_BLOCK_CHARS: usize = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Bbox {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Debug, Clone)]
struct Glyph {
    ch: char,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Debug, Clone)]
struct Line {
    text: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// One extracted region, already in the viewer's `PdfLayoutRegion` shape.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Region {
    id: String,
    page_index: u32,
    kind: &'static str,
    label: &'static str,
    score: f64,
    reading_order: u32,
    rect: Bbox,
    bbox: Bbox,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutAnalyzeOutcome {
    pub paper_path: String,
    pub from_cache: bool,
    pub region_count: usize,
    pub translatable_count: usize,
    pub pages: u32,
}

/// Write layout sidecars for one paper folder. Returns `Ok(None)` when the
/// folder has no local PDF.
pub fn analyze_paper_dir(
    vault: &Path,
    paper_rel: &str,
    force: bool,
) -> Result<Option<LayoutAnalyzeOutcome>, AppError> {
    let paper_dir = vault.join(paper_rel);
    if !force && sidecar_ready(&paper_dir) {
        let regions = count_regions(&paper_dir);
        return Ok(Some(LayoutAnalyzeOutcome {
            paper_path: paper_rel.to_string(),
            from_cache: true,
            region_count: regions,
            translatable_count: regions,
            pages: 0,
        }));
    }

    let Some(pdf_path) = crate::features::paper::capabilities::find_local_pdf(&paper_dir) else {
        return Ok(None);
    };
    let bytes = fs::read(&pdf_path).map_err(|e| AppError::message(format!("read pdf: {e}")))?;
    let (regions, pages) = extract_regions(&bytes)?;
    write_sidecars(&paper_dir, &regions)?;
    let translatable = regions.iter().filter(|r| is_translatable(r.kind)).count();
    Ok(Some(LayoutAnalyzeOutcome {
        paper_path: paper_rel.to_string(),
        from_cache: false,
        region_count: regions.len(),
        translatable_count: translatable,
        pages,
    }))
}

fn sidecar_ready(paper_dir: &Path) -> bool {
    let raw = paper_dir.join("source").join("layout.json");
    let index = paper_dir.join(".src").join("layout-index.json");
    raw.is_file() && index.is_file() && count_regions(paper_dir) > 0
}

fn count_regions(paper_dir: &Path) -> usize {
    let path = paper_dir.join("source").join("layout.json");
    let Ok(text) = fs::read_to_string(path) else {
        return 0;
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("regions").and_then(Value::as_array).map(|a| a.len()))
        .unwrap_or(0)
}

pub fn is_translatable(kind: &str) -> bool {
    matches!(kind, "text" | "abstract" | "header" | "figure_title")
}

fn extract_regions(pdf: &[u8]) -> Result<(Vec<Region>, u32), AppError> {
    let lib = Library::init();
    let doc = lib
        .load_document_from_bytes(pdf, None)
        .map_err(|e| AppError::message(format!("open pdf: {e:?}")))?;
    let page_count = doc.page_count();
    if page_count <= 0 {
        return Ok((Vec::new(), 0));
    }

    let mut regions = Vec::new();
    let mut order = 0u32;
    for index in 0..page_count {
        let page = match doc.page(index) {
            Ok(page) => page,
            Err(_) => continue,
        };
        let Some(view_box) = page.view_box() else {
            continue;
        };
        let (page_width, page_height) = page.viewport_size(&view_box);
        if page_width <= 0.0 || page_height <= 0.0 {
            continue;
        }
        let text_page = match page.text() {
            Ok(text) => text,
            Err(_) => continue,
        };
        let total = text_page.char_count();
        if total <= 0 {
            continue;
        }
        let mut glyphs = Vec::with_capacity(total as usize);
        for i in 0..total {
            let ch = text_page.char_at_unchecked(i);
            let Some(unicode) = char::from_u32(ch.unicode()) else {
                continue;
            };
            if unicode == '\0' || unicode == '\u{FFFE}' {
                continue;
            }
            let Some(box_) = ch.char_box() else {
                continue;
            };
            let Some(glyph) = glyph_in_viewport(&page, &view_box, page_width, page_height, unicode, &box_)
            else {
                continue;
            };
            glyphs.push(glyph);
        }
        let lines = cluster_lines(glyphs);
        let blocks = cluster_blocks(lines, page_width as f64);
        let page_index = index as u32;
        for block in blocks {
            let text = normalize_block(&block.text);
            if text.chars().count() < MIN_BLOCK_CHARS {
                continue;
            }
            let kind = classify_block(&text, block.y, block.h, page_height as f64);
            let id = format!("p{page_index}-r{order}");
            let bbox = Bbox {
                x: clamp01(block.x / page_width as f64),
                y: clamp01(block.y / page_height as f64),
                w: clamp01(block.w / page_width as f64),
                h: clamp01(block.h / page_height as f64),
            };
            let rect = Bbox {
                x: block.x,
                y: block.y,
                w: block.w,
                h: block.h,
            };
            regions.push(Region {
                id,
                page_index,
                kind,
                label: kind,
                score: 1.0,
                reading_order: order,
                rect,
                bbox,
                text,
            });
            order += 1;
        }
    }
    Ok((regions, page_count as u32))
}

fn glyph_in_viewport(
    page: &liteparse_pdfium::Page<'_, '_>,
    view_box: &RectF,
    page_width: f32,
    page_height: f32,
    ch: char,
    box_: &CharBox,
) -> Option<Glyph> {
    let bounds = RectF {
        left: box_.left as f32,
        top: box_.top as f32,
        right: box_.right as f32,
        bottom: box_.bottom as f32,
    };
    let view = page.bounds_to_viewport(view_box, &bounds);
    let w = (view.right - view.left) as f64;
    let h = (view.bottom - view.top) as f64;
    if w <= 0.05 || h <= 0.05 || w > page_width as f64 || h > page_height as f64 * 0.5 {
        return None;
    }
    Some(Glyph {
        ch,
        x: view.left as f64,
        y: view.top as f64,
        w,
        h,
    })
}

fn cluster_lines(glyphs: Vec<Glyph>) -> Vec<Line> {
    let mut glyphs = glyphs;
    glyphs.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut lines: Vec<Vec<Glyph>> = Vec::new();
    for glyph in glyphs {
        let placed = lines.iter_mut().rev().take(8).find(|line| {
            let (y, h) = line_band(line);
            let overlap = (y + h).min(glyph.y + glyph.h) - y.max(glyph.y);
            overlap > glyph.h.min(h) * 0.45
        });
        if let Some(line) = placed {
            line.push(glyph);
        } else {
            lines.push(vec![glyph]);
        }
    }
    let mut out = Vec::with_capacity(lines.len());
    for mut line in lines {
        line.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        for segment in split_line_columns(line) {
            if let Some(built) = build_line(&segment) {
                out.push(built);
            }
        }
    }
    out.sort_by(|a, b| {
        column_of(a)
            .cmp(&column_of(b))
            .then(a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
    });
    out
}

fn split_line_columns(glyphs: Vec<Glyph>) -> Vec<Vec<Glyph>> {
    if glyphs.len() < 4 {
        return vec![glyphs];
    }
    let mut gaps: Vec<(usize, f64)> = Vec::new();
    for index in 1..glyphs.len() {
        let gap = glyphs[index].x - (glyphs[index - 1].x + glyphs[index - 1].w);
        if gap > 18.0 {
            gaps.push((index, gap));
        }
    }
    let Some(&(cut, _)) = gaps.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    else {
        return vec![glyphs];
    };
    if cut < 2 || glyphs.len() - cut < 2 {
        return vec![glyphs];
    }
    let (left, right) = glyphs.split_at(cut);
    vec![left.to_vec(), right.to_vec()]
}

fn build_line(line: &[Glyph]) -> Option<Line> {
    let mut text = String::new();
    let mut prev_right = None;
    let widths: Vec<f64> = line
        .iter()
        .map(|g| g.w)
        .filter(|w| *w > 0.4 && *w < 40.0)
        .collect();
    let typical = if widths.is_empty() {
        4.0
    } else {
        let mut sorted = widths;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    };
    let space = (typical * 0.22).max(1.0);
    for glyph in line {
        if let Some(right) = prev_right {
            if glyph.x - right > space && !text.ends_with(' ') && glyph.ch != ' ' {
                text.push(' ');
            }
        }
        if glyph.ch == '\n' || glyph.ch == '\r' {
            continue;
        }
        text.push(glyph.ch);
        prev_right = Some(glyph.x + glyph.w);
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    let x = line.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
    let y = line.iter().map(|g| g.y).fold(f64::INFINITY, f64::min);
    let right = line.iter().map(|g| g.x + g.w).fold(0.0, f64::max);
    let bottom = line.iter().map(|g| g.y + g.h).fold(0.0, f64::max);
    Some(Line {
        text,
        x,
        y,
        w: (right - x).max(0.0),
        h: (bottom - y).max(0.0),
    })
}

fn line_band(line: &[Glyph]) -> (f64, f64) {
    let y = line.iter().map(|g| g.y).fold(f64::INFINITY, f64::min);
    let bottom = line.iter().map(|g| g.y + g.h).fold(0.0, f64::max);
    (y, (bottom - y).max(0.1))
}

fn column_of(line: &Line) -> u8 {
    if line.x > 280.0 {
        1
    } else {
        0
    }
}

fn cluster_blocks(lines: Vec<Line>, page_width: f64) -> Vec<Line> {
    let mut blocks: Vec<Line> = Vec::new();
    for line in lines {
        let attach = blocks.last_mut().is_some_and(|block| {
            let gap = line.y - (block.y + block.h);
            let same_column = (line.x - block.x).abs() < page_width * 0.18
                || column_of(&line) == column_of(block);
            let close = gap >= -2.0 && gap < line.h.max(8.0) * 0.85;
            same_column && close
        });
        if attach {
            let block = blocks.last_mut().expect("checked");
            if block.text.ends_with('-')
                && line
                    .text
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase())
            {
                block.text.pop();
                block.text.push_str(&line.text);
            } else {
                block.text.push(' ');
                block.text.push_str(&line.text);
            }
            let right = (block.x + block.w).max(line.x + line.w);
            let bottom = (block.y + block.h).max(line.y + line.h);
            block.x = block.x.min(line.x);
            block.y = block.y.min(line.y);
            block.w = right - block.x;
            block.h = bottom - block.y;
        } else {
            blocks.push(line);
        }
    }
    blocks
}

fn looks_like_heading(text: &str) -> bool {
    let chars = text.chars().count();
    chars > 0
        && chars <= 90
        && !text.ends_with('.')
        && !text.ends_with(',')
        && text.split_whitespace().count() <= 14
}

fn classify_block(text: &str, y: f64, h: f64, page_height: f64) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("abstract") && text.chars().count() < 2400 {
        return "abstract";
    }
    if lower.starts_with("fig.")
        || lower.starts_with("figure ")
        || lower.starts_with("table ")
        || lower.starts_with("algorithm ")
    {
        return "figure_title";
    }
    if looks_like_heading(text) && h < page_height * 0.08 && y < page_height * 0.92 {
        return "header";
    }
    "text"
}

fn normalize_block(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clamp01(value: f64) -> f64 {
    if !value.is_finite() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn write_sidecars(paper_dir: &Path, regions: &[Region]) -> Result<(), AppError> {
    let generated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let raw = json!({
        "schemaVersion": LAYOUT_SCHEMA,
        "source": { "mode": "embedpdf-layout", "generatedAt": generated_at },
        "regions": regions,
    });
    let source_dir = paper_dir.join("source");
    fs::create_dir_all(&source_dir)?;
    json_store(&source_dir.join("layout.json"), &raw)?;

    let items: Vec<Value> = regions
        .iter()
        .filter(|region| matches!(region.kind, "header"))
        .map(|region| {
            json!({
                "id": region.id,
                "stableKey": format!("p{}:section:{}", region.page_index + 1, region.text.chars().take(80).collect::<String>()),
                "kind": "header",
                "section": "section",
                "page": region.page_index + 1,
                "pageIndex": region.page_index,
                "bbox": region.bbox,
                "score": 1.0,
                "title": region.text,
                "layoutRegionId": region.id,
            })
        })
        .collect();
    let index = json!({
        "schemaVersion": INDEX_SCHEMA,
        "source": {
            "mode": "sidebar",
            "from": "layout.json",
            "generatedAt": generated_at,
            "minScore": 0.3,
        },
        "items": items,
    });
    let src_dir = paper_dir.join(".src");
    fs::create_dir_all(&src_dir)?;
    json_store(&src_dir.join("layout-index.json"), &index)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_abstract_caption_and_heading() {
        assert_eq!(classify_block("Abstract We study", 40.0, 12.0, 800.0), "abstract");
        assert_eq!(
            classify_block("Figure 2: results on the benchmark.", 500.0, 10.0, 800.0),
            "figure_title"
        );
        assert_eq!(classify_block("Introduction", 70.0, 16.0, 800.0), "header");
        assert_eq!(
            classify_block("This sentence continues the paragraph and ends.", 200.0, 11.0, 800.0),
            "text"
        );
    }
}
