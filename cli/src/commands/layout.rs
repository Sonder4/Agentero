//! `agentero layout *` — sidebar-aligned layout index (figures / tables / …).

use crate::error::CliError;
use crate::output::to_value;
use crate::resolve::{resolve_paper, resolve_vault, GlobalOpts};
use crate::style::{format_table, truncate_chars};
use agentero_core::features::catalog::papers;
use agentero_core::features::layout_index::{self, LayoutIndexItem};
use agentero_core::features::pdf::{layout_text, layout_translate};
use clap::{Subcommand, ValueHint};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Subcommand)]
pub enum LayoutCmd {
    /// List sidebar-aligned regions (figure / table / algorithm / formula).
    List {
        /// Vault-relative paper path or id.
        #[arg(value_hint = ValueHint::DirPath)]
        r#ref: String,
        /// Filter: figure (image+chart), image, chart, table, algorithm, formula.
        /// Repeatable; OR semantics.
        #[arg(
            long = "kind",
            value_name = "KIND",
            value_parser = ["figure", "image", "chart", "table", "algorithm", "formula", "section"]
        )]
        kinds: Vec<String>,
        /// Minimum score (0–1). Default: index minScore or 0.3.
        #[arg(long = "min-score", value_name = "N")]
        min_score: Option<f64>,
    },
    /// Get one region by id (CLI id from `layout list`).
    Get {
        /// Vault-relative paper path or id.
        #[arg(value_hint = ValueHint::DirPath)]
        r#ref: String,
        /// Region id (e.g. figure-3).
        id: String,
    },
    /// Extract reading-order layout blocks from each paper PDF.
    ///
    /// Writes `{paper}/source/layout.json` and `{paper}/.src/layout-index.json`.
    /// Existing sidecars are reused unless `--force` is set. Omit the paper ref
    /// to process every catalog paper that has a local PDF.
    Analyze {
        /// Vault-relative paper path or id. Omit for the whole vault.
        #[arg(value_hint = ValueHint::DirPath)]
        r#ref: Option<String>,
        /// Rewrite sidecars even when they already exist.
        #[arg(long = "force")]
        force: bool,
    },
    /// Translate layout body text into `{paper}/.src/layout-translate.json`.
    ///
    /// Runs `layout analyze` first when the layout sidecar is missing. Omit the
    /// paper ref to translate every catalog paper that has a local PDF.
    Translate {
        /// Vault-relative paper path or id. Omit for the whole vault.
        #[arg(value_hint = ValueHint::DirPath)]
        r#ref: Option<String>,
        /// Target language (default zh-CN).
        #[arg(long = "to", value_name = "LANG", default_value = "zh-CN")]
        to: String,
        /// Source language (default auto).
        #[arg(long = "from", value_name = "LANG", default_value = "auto")]
        from: String,
        /// Pi agent workers translating at once (1–8, default 4).
        #[arg(long = "jobs", value_name = "N", default_value_t = 4)]
        jobs: usize,
        /// Ignore an existing translation sidecar.
        #[arg(long = "force")]
        force: bool,
    },
}

pub async fn run(cmd: LayoutCmd, globals: &GlobalOpts) -> Result<Value, CliError> {
    match cmd {
        LayoutCmd::List {
            r#ref,
            kinds,
            min_score,
        } => list(globals, &r#ref, &kinds, min_score),
        LayoutCmd::Get { r#ref, id } => get(globals, &r#ref, &id),
        LayoutCmd::Analyze { r#ref, force } => analyze(globals, r#ref.as_deref(), force).await,
        LayoutCmd::Translate {
            r#ref,
            to,
            from,
            jobs,
            force,
        } => translate(globals, r#ref.as_deref(), &to, &from, jobs, force).await,
    }
}

fn list(
    globals: &GlobalOpts,
    ref_: &str,
    kinds: &[String],
    min_score: Option<f64>,
) -> Result<Value, CliError> {
    let vault = resolve_vault(globals)?;
    let paper = resolve_paper(&vault, ref_, globals)?;
    let data = layout_index::list_regions(&vault, &paper.path, kinds, min_score)?;
    let style = globals.style;
    let table_rows: Vec<Vec<String>> = data
        .items
        .iter()
        .map(|i| {
            vec![
                i.id.clone(),
                i.section.clone(),
                i.kind.clone(),
                i.page.to_string(),
                format!("{:.0}%", i.score * 100.0),
                truncate_chars(i.title.as_deref().unwrap_or(""), 48),
            ]
        })
        .collect();
    let lines = if data.items.is_empty() {
        vec![style.dim("(no layout regions)")]
    } else {
        format_table(
            style,
            &["ID", "SECTION", "KIND", "PAGE", "SCORE", "TITLE"],
            &table_rows,
        )
    };

    let mut out = to_value(&data)?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("lines".into(), json!(lines));
    }
    Ok(out)
}

fn get(globals: &GlobalOpts, ref_: &str, id: &str) -> Result<Value, CliError> {
    let vault = resolve_vault(globals)?;
    let paper = resolve_paper(&vault, ref_, globals)?;
    let data = layout_index::get_region(&vault, &paper.path, id.trim())?;
    let item = &data.item;
    let mut lines = vec![format!(
        "{}  {}  page {}  {}  score {:.0}%",
        item.id,
        item.section,
        item.page,
        item.kind,
        item.score * 100.0
    )];
    if let Some(t) = &item.title {
        lines.push(format!("title: {t}"));
    }
    lines.push(format!(
        "bbox: x={:.4} y={:.4} w={:.4} h={:.4}",
        item.bbox.x, item.bbox.y, item.bbox.w, item.bbox.h
    ));

    let mut out = to_value(&data)?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("lines".into(), json!(lines));
    }
    Ok(out)
}

async fn analyze(
    globals: &GlobalOpts,
    paper_ref: Option<&str>,
    force: bool,
) -> Result<Value, CliError> {
    let vault = resolve_vault(globals)?;
    let papers = target_papers(&vault, paper_ref, globals)?;
    let mut rows = Vec::new();
    let mut analyzed = 0usize;
    let mut cached = 0usize;
    let mut skipped = 0usize;
    for paper in &papers {
        match layout_text::analyze_paper_dir(&vault, &paper.path, force) {
            Ok(Some(outcome)) => {
                if outcome.from_cache {
                    cached += 1;
                } else {
                    analyzed += 1;
                }
                rows.push(outcome);
            }
            Ok(None) => skipped += 1,
            Err(err) => {
                return Err(CliError::message(format!("{}: {err}", paper.path)));
            }
        }
    }
    let mut out = json!({
        "analyzed": analyzed,
        "cached": cached,
        "skipped": skipped,
        "papers": rows,
    });
    if let Some(obj) = out.as_object_mut() {
        obj.insert(
            "lines".into(),
            json!([format!(
                "{} analyzed={analyzed} cached={cached} skipped={skipped}",
                globals.style.ok("layout")
            )]),
        );
    }
    Ok(out)
}

async fn translate(
    globals: &GlobalOpts,
    paper_ref: Option<&str>,
    target: &str,
    source: &str,
    jobs: usize,
    force: bool,
) -> Result<Value, CliError> {
    let vault = resolve_vault(globals)?;
    let papers = target_papers(&vault, paper_ref, globals)?;
    let mut rows = Vec::new();
    let mut translated = 0usize;
    let mut failed_items = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();
    for paper in &papers {
        if let Err(err) = layout_text::analyze_paper_dir(&vault, &paper.path, false) {
            errors.push(json!({ "path": paper.path, "error": err.to_string() }));
            continue;
        }
        match layout_translate::translate_paper_dir(
            &vault,
            &paper.path,
            target,
            source,
            jobs,
            force,
        )
        .await
        {
            Ok(Some(outcome)) => {
                translated += outcome.translated;
                failed_items += outcome.failed;
                rows.push(outcome);
            }
            Ok(None) => skipped += 1,
            Err(err) => errors.push(json!({ "path": paper.path, "error": err.to_string() })),
        }
    }
    let mut out = json!({
        "papers": rows.len(),
        "translated": translated,
        "failed": failed_items,
        "skipped": skipped,
        "errors": errors,
        "targetLang": target,
        "items": rows,
    });
    if let Some(obj) = out.as_object_mut() {
        obj.insert(
            "lines".into(),
            json!([format!(
                "{} papers={} translated={translated} failed={failed_items} skipped={skipped} errors={}",
                globals.style.ok("translate"),
                rows.len(),
                errors.len()
            )]),
        );
    }
    Ok(out)
}

fn target_papers(
    vault: &Path,
    paper_ref: Option<&str>,
    globals: &GlobalOpts,
) -> Result<Vec<papers::PaperRecord>, CliError> {
    if let Some(paper_ref) = paper_ref {
        return Ok(vec![resolve_paper(vault, paper_ref, globals)?]);
    }
    Ok(papers::list_all(vault)?)
}

/// Shared by `mark add --region`.
pub fn load_region(
    vault: &std::path::Path,
    paper_path: &str,
    region_id: &str,
) -> Result<LayoutIndexItem, CliError> {
    Ok(layout_index::load_region(vault, paper_path, region_id)?)
}
