use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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

    let mut page_starts: Vec<u32> = top_level.iter().map(|e| e.page as u32).collect();
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
            Command::new(&config.tools.tesseract_path).args([page_img_str, "stdout"]),
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
