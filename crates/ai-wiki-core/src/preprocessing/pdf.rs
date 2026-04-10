use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use indexmap::IndexMap;
use lopdf::Document;

use crate::config::Config;

#[derive(Debug, PartialEq)]
pub enum PdfClassification {
    Simple,
    Book { chapter_count: usize },
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
    let doc =
        Document::load(path).with_context(|| format!("failed to load PDF: {}", path.display()))?;

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
    let doc =
        Document::load(path).with_context(|| format!("failed to load PDF: {}", path.display()))?;

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
                dest.page()
                    .ok()
                    .and_then(|page_obj| match page_obj.as_reference() {
                        Ok(obj_id) => page_id_to_num.get(&obj_id).copied(),
                        Err(_) => page_obj.as_i64().ok().map(|n| n as u32),
                    })
            } else {
                None
            }
        })
        .collect();

    if page_starts.is_empty() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut page_starts = page_starts;
    page_starts.sort();
    page_starts.dedup();

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
        let status = super::run_tool(
            Command::new(&config.tools.qpdf_path)
                .arg(path)
                .arg("--pages")
                .arg(".")
                .arg(format!("{start}-{end}"))
                .arg("--")
                .arg(&output_path),
            "qpdf",
        )?;

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
    // Try pdf_extract::extract_text first.
    // Wrapped in catch_unwind because the upstream cff-parser crate can panic
    // on malformed PDFs (e.g., cff-parser-0.1.0/src/encoding.rs:150).
    let pdf_extract_result = {
        let path_owned = path.to_path_buf();
        std::panic::catch_unwind(|| pdf_extract::extract_text(&path_owned))
    };
    match pdf_extract_result {
        Ok(Ok(text)) if !text.trim().is_empty() => return Ok(text),
        Ok(Err(e)) => {
            eprintln!("pdf-extract failed for {}: {e}", path.display());
        }
        Err(_) => {
            eprintln!(
                "pdf-extract panicked for {} (likely malformed font data), falling back to pdftotext",
                path.display()
            );
        }
        _ => {} // empty text, fall through
    }

    // Fallback to pdftotext (poppler) via Command
    let pdftotext_result = super::run_tool_output(
        Command::new(&config.tools.pdftotext_path)
            .arg(path)
            .arg("-"),
        "pdftotext",
    );

    if let Ok(output) = pdftotext_result
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }

    // Fallback: render PDF pages to images with pdftoppm, then OCR each with tesseract
    match ocr_pdf_text(path, config) {
        Ok(text) => Ok(text),
        Err(e) => Err(anyhow::anyhow!(
            "failed to extract text from PDF: {} — all methods (pdf-extract, pdftotext, tesseract) failed: {}",
            path.display(),
            e
        )),
    }
}

static OCR_COUNTER: AtomicU64 = AtomicU64::new(0);

// OCR fallback: render pages to images, then tesseract each
fn ocr_pdf_text(path: &Path, config: &Config) -> anyhow::Result<String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "ai_wiki_ocr_{}_{}",
        std::process::id(),
        OCR_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&temp_dir)?;

    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {}", path.display()))?;
    let page_prefix = temp_dir.join("page");
    let page_prefix_str = page_prefix
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("temp dir path is not valid UTF-8: {}", page_prefix.display()))?;

    // Render PDF pages to PPM images
    let status = super::run_tool(
        Command::new(&config.tools.pdftoppm_path)
            .arg(path_str)
            .arg(page_prefix_str),
        "pdftoppm",
    )?;

    if !status.success() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        anyhow::bail!("pdftoppm failed for {}", path.display());
    }

    // Find generated page images and OCR each
    let mut full_text = String::new();
    let pages_result: anyhow::Result<Vec<_>> = (|| {
        let mut pages: Vec<_> = std::fs::read_dir(&temp_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "ppm"))
            .collect();
        pages.sort();
        Ok(pages)
    })();

    let pages = match pages_result {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(e);
        }
    };

    for page_img in &pages {
        let page_img_str = match page_img.to_str() {
            Some(s) => s,
            None => {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(anyhow::anyhow!(
                    "page image path is not valid UTF-8: {}",
                    page_img.display()
                ));
            }
        };
        let output = match super::run_tool_output(
            Command::new(&config.tools.tesseract_path)
                .args([page_img_str, "stdout"]),
            "tesseract",
        ) {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(e);
            }
        };

        if output.status.success() {
            full_text.push_str(&String::from_utf8_lossy(&output.stdout));
            full_text.push('\n');
        }
    }

    // Cleanup temp dir (success path)
    let _ = std::fs::remove_dir_all(&temp_dir);

    if full_text.trim().is_empty() {
        anyhow::bail!("OCR produced no text for {}", path.display());
    }

    Ok(full_text)
}
