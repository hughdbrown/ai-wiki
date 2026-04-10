use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use indexmap::IndexMap;
use lopdf::Document;

use crate::config::Config;

#[derive(Debug, PartialEq)]
pub enum PdfClassification {
    Simple,
    Book { chapter_count: usize },
    Sensitive(String),
}

fn count_top_level_outlines(doc: &Document) -> usize {
    let mut named_destinations = IndexMap::new();
    doc.get_outlines(None, None, &mut named_destinations)
        .ok()
        .and_then(|o| o)
        .map(|outlines| {
            outlines
                .iter()
                .filter(|o| matches!(o, lopdf::Outline::Destination(_)))
                .count()
        })
        .unwrap_or(0)
}

fn has_outlines(doc: &Document) -> bool {
    let mut named_destinations = IndexMap::new();
    doc.get_outlines(None, None, &mut named_destinations)
        .ok()
        .and_then(|o| o)
        .map(|outlines| !outlines.is_empty())
        .unwrap_or(false)
}

pub fn classify_pdf(path: &Path, config: &Config) -> anyhow::Result<PdfClassification> {
    let doc = Document::load(path)
        .with_context(|| format!("failed to load PDF: {}", path.display()))?;

    let page_count = doc.get_pages().len() as u32;
    let outlines_present = has_outlines(&doc);

    if outlines_present && page_count >= config.pdf.book_min_pages {
        let chapter_count = count_top_level_outlines(&doc);
        Ok(PdfClassification::Book { chapter_count })
    } else {
        Ok(PdfClassification::Simple)
    }
}

pub fn split_pdf_chapters(
    path: &Path,
    output_dir: &Path,
    config: &Config,
) -> anyhow::Result<Vec<PathBuf>> {
    let doc = Document::load(path)
        .with_context(|| format!("failed to load PDF: {}", path.display()))?;

    let mut named_destinations = IndexMap::new();
    let outlines = doc
        .get_outlines(None, None, &mut named_destinations)
        .ok()
        .and_then(|o| o)
        .unwrap_or_default();

    if outlines.is_empty() {
        // No bookmarks: return the original path as the single chunk
        return Ok(vec![path.to_path_buf()]);
    }

    let total_pages = doc.get_pages().len() as u32;

    // Build a lookup: ObjectId -> page number
    let pages = doc.get_pages(); // BTreeMap<u32, ObjectId>
    let page_id_to_num: std::collections::HashMap<lopdf::ObjectId, u32> =
        pages.iter().map(|(&num, &id)| (id, num)).collect();

    // Extract page numbers from top-level Destination outlines
    let page_starts: Vec<u32> = outlines
        .iter()
        .filter_map(|o| {
            if let lopdf::Outline::Destination(dest) = o {
                dest.page().ok().and_then(|page_obj| {
                    match page_obj.as_reference() {
                        Ok(obj_id) => page_id_to_num.get(&obj_id).copied(),
                        Err(_) => page_obj.as_i64().ok().map(|n| n as u32),
                    }
                })
            } else {
                None
            }
        })
        .collect();

    if page_starts.is_empty() {
        return Ok(vec![path.to_path_buf()]);
    }

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output dir: {}", output_dir.display()))?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("chapter");

    let mut output_paths = Vec::new();
    for (i, &start) in page_starts.iter().enumerate() {
        let end = if i + 1 < page_starts.len() {
            page_starts[i + 1].saturating_sub(1)
        } else {
            total_pages
        };

        let output_path = output_dir.join(format!("{stem}_chapter_{:03}.pdf", i + 1));
        let status = Command::new(&config.tools.qpdf_path)
            .arg(path)
            .arg("--pages")
            .arg(".")
            .arg(format!("{start}-{end}"))
            .arg("--")
            .arg(&output_path)
            .status()
            .with_context(|| format!("failed to run qpdf for chapter {}", i + 1))?;

        if !status.success() {
            return Err(anyhow::anyhow!(
                "qpdf failed for chapter {} (pages {}-{})",
                i + 1,
                start,
                end
            ));
        }
        output_paths.push(output_path);
    }

    Ok(output_paths)
}

pub fn extract_pdf_text(path: &Path, config: &Config) -> anyhow::Result<String> {
    // Try pdf_extract::extract_text first
    if let Ok(text) = pdf_extract::extract_text(path)
        && !text.trim().is_empty()
    {
        return Ok(text);
    }

    // Fallback to pdftotext (poppler) via Command
    let pdftotext_result = Command::new(&config.tools.pdftotext_path)
        .arg(path)
        .arg("-") // output to stdout
        .output();

    if let Ok(output) = pdftotext_result
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }

    // Fallback to tesseract via Command
    let temp_dir = std::env::temp_dir();
    let output_base = temp_dir.join(format!(
        "ai_wiki_ocr_{}_{}",
        std::process::id(),
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("doc")
    ));
    let tesseract_status = Command::new(&config.tools.tesseract_path)
        .arg(path)
        .arg(&output_base)
        .arg("pdf")
        .arg("txt")
        .status();

    if let Ok(status) = tesseract_status
        && status.success()
    {
        let txt_path = output_base.with_extension("txt");
        if let Ok(text) = std::fs::read_to_string(&txt_path)
            && !text.trim().is_empty()
        {
            return Ok(text);
        }
    }

    Err(anyhow::anyhow!(
        "failed to extract text from PDF: {} — all methods (pdf-extract, pdftotext, tesseract) failed",
        path.display()
    ))
}
