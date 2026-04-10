use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use lopdf::Document;

use crate::config::Config;

#[derive(Debug, PartialEq)]
pub enum PdfClassification {
    Simple,
    Book { chapter_count: usize },
}

pub fn classify_pdf(path: &Path, config: &Config) -> anyhow::Result<PdfClassification> {
    let doc =
        Document::load(path).with_context(|| format!("failed to load PDF: {}", path.display()))?;

    let page_count = doc.get_pages().len() as u32;

    // Use get_toc() for reliable outline detection
    let toc = match doc.get_toc() {
        Ok(toc) if !toc.toc.is_empty() => toc,
        _ => return Ok(PdfClassification::Simple),
    };

    let top_level_count = toc.toc.iter().filter(|e| e.level == 1).count();

    if top_level_count > 0 && page_count >= config.pdf.book_min_pages {
        Ok(PdfClassification::Book {
            chapter_count: top_level_count,
        })
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

    let total_pages = doc.get_pages().len() as u32;

    // Use get_toc() which properly resolves titles, page numbers, and nesting levels
    let toc = doc
        .get_toc()
        .map_err(|e| anyhow::anyhow!("failed to get table of contents: {e}"))?;

    // Only split on top-level (level 1) entries to avoid fragmenting into sub-sections
    let top_level: Vec<_> = toc.toc.iter().filter(|e| e.level == 1).collect();

    if top_level.is_empty() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut page_starts: Vec<u32> = top_level
        .iter()
        .map(|e| e.page as u32)
        .filter(|&p| p >= 1 && p <= total_pages)
        .collect();
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

        if end < start {
            continue; // skip degenerate range
        }

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
        Ok(Err(_)) | Err(_) => {
            // pdf-extract failed or panicked (common with CFF font encoding issues).
            // Not worth logging — pdftotext handles these files fine.
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

// OCR fallback: render pages to images, then tesseract each
fn ocr_pdf_text(path: &Path, config: &Config) -> anyhow::Result<String> {
    let temp_dir =
        tempfile::tempdir().context("failed to create temp directory for OCR")?;

    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {}", path.display()))?;
    let page_prefix = temp_dir.path().join("page");
    let page_prefix_str = page_prefix.to_str().ok_or_else(|| {
        anyhow::anyhow!(
            "temp dir path is not valid UTF-8: {}",
            page_prefix.display()
        )
    })?;

    // Render PDF pages to PPM images
    let status = super::run_tool(
        Command::new(&config.tools.pdftoppm_path)
            .arg(path_str)
            .arg(page_prefix_str),
        "pdftoppm",
    )?;

    if !status.success() {
        anyhow::bail!("pdftoppm failed for {}", path.display());
    }

    // Find generated page images and OCR each
    let mut pages: Vec<_> = std::fs::read_dir(temp_dir.path())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ppm"))
        .collect();
    pages.sort();

    let mut full_text = String::new();
    for page_img in &pages {
        let page_img_str = page_img
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("page image path is not valid UTF-8: {}", page_img.display()))?;
        let output = super::run_tool_output(
            Command::new(&config.tools.tesseract_path).args([page_img_str, "stdout"]),
            "tesseract",
        )?;

        if output.status.success() {
            full_text.push_str(&String::from_utf8_lossy(&output.stdout));
            full_text.push('\n');
        }
    }

    // temp_dir auto-cleans up on drop

    if full_text.trim().is_empty() {
        anyhow::bail!("OCR produced no text for {}", path.display());
    }

    Ok(full_text)
}
