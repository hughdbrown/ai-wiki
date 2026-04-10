use std::path::Path;

use anyhow::Context;
use lopdf::Document;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: pdf-dump <file.pdf>");
        std::process::exit(1);
    }

    let path = Path::new(&args[1]);
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }

    let doc = Document::load(path)
        .with_context(|| format!("failed to load PDF: {}", path.display()))?;

    let page_count = doc.get_pages().len();
    println!("File: {}", path.display());
    println!("Pages: {page_count}");
    println!();

    // Get the table of contents
    let toc = match doc.get_toc() {
        Ok(toc) => toc,
        Err(e) => {
            println!("No table of contents found: {e}");
            println!();
            println!("This PDF has no outlines/bookmarks.");
            println!("It would be classified as a Simple PDF (not split into chapters).");
            return Ok(());
        }
    };

    if !toc.errors.is_empty() {
        println!("TOC parsing warnings:");
        for err in &toc.errors {
            println!("  ! {err}");
        }
        println!();
    }

    if toc.toc.is_empty() {
        println!("Table of contents is empty.");
        println!("This PDF would be classified as Simple (not split).");
        return Ok(());
    }

    // Display table of contents
    println!("═══════════════════════════════════════════════════════════════════");
    println!("TABLE OF CONTENTS ({} entries)", toc.toc.len());
    println!("═══════════════════════════════════════════════════════════════════");
    println!();

    for (i, entry) in toc.toc.iter().enumerate() {
        let indent = "  ".repeat(entry.level.saturating_sub(1));
        println!(
            "  {:>3}. {indent}{:<60} page {}",
            i + 1,
            entry.title,
            entry.page
        );
    }
    println!();

    // Compute chapter split blocks (same logic as split_pdf_chapters)
    // Only use top-level entries (level 1) for splitting
    let top_level: Vec<_> = toc.toc.iter().filter(|e| e.level == 1).collect();

    if top_level.is_empty() {
        println!("No top-level (level 1) entries for splitting.");
        println!("All entries are nested — would not split this PDF.");
        return Ok(());
    }

    // Collect unique sorted page starts from top-level entries
    let mut page_starts: Vec<usize> = top_level.iter().map(|e| e.page).collect();
    page_starts.sort();
    page_starts.dedup();

    println!("═══════════════════════════════════════════════════════════════════");
    println!("SPLIT BLOCKS ({} chapters from {} top-level entries)", page_starts.len(), top_level.len());
    println!("═══════════════════════════════════════════════════════════════════");
    println!();
    println!(
        "  {:>5}  {:<50} {:>8}  {:>8}  {:>5}",
        "Block", "Title", "Start", "End", "Pages"
    );
    println!("  {}", "─".repeat(80));

    for (i, &start) in page_starts.iter().enumerate() {
        let end = if i + 1 < page_starts.len() {
            page_starts[i + 1] - 1
        } else {
            page_count
        };

        // Find the TOC entry that starts at this page
        let title = top_level
            .iter()
            .find(|e| e.page == start)
            .map(|e| e.title.as_str())
            .unwrap_or("(unnamed)");

        let page_span = if end >= start { end - start + 1 } else { 0 };

        println!(
            "  {:>5}  {:<50} {:>8}  {:>8}  {:>5}",
            i + 1,
            if title.len() > 50 { &title[..50] } else { title },
            start,
            end,
            page_span
        );
    }

    println!();
    println!("Total pages covered: {page_count}");
    println!(
        "Book detection: {} top-level entries, {} pages → {}",
        top_level.len(),
        page_count,
        if page_count >= 50 { "BOOK (would split)" } else { "SIMPLE (too few pages)" }
    );

    // Also show what the current code does differently:
    // The current code uses ALL Destination outlines, not just level 1
    println!();
    println!("═══════════════════════════════════════════════════════════════════");
    println!("COMPARISON: Current code's split (all Destination outlines, not just level 1)");
    println!("═══════════════════════════════════════════════════════════════════");

    let all_pages: Vec<usize> = toc.toc.iter().map(|e| e.page).collect();
    let mut all_deduped = all_pages.clone();
    all_deduped.sort();
    all_deduped.dedup();

    println!();
    println!(
        "  All outline entries: {} → {} unique page starts after dedup",
        toc.toc.len(),
        all_deduped.len()
    );
    println!(
        "  Top-level only:     {} → {} unique page starts after dedup",
        top_level.len(),
        page_starts.len()
    );

    if all_deduped.len() != page_starts.len() {
        println!();
        println!("  ⚠ MISMATCH: The current code would create {} split blocks", all_deduped.len());
        println!("    because it uses ALL outline destinations, not just top-level.");
        println!("    This means sub-chapters create extra splits.");
    }

    Ok(())
}
