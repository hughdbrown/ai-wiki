use std::path::Path;
use std::process;

use anyhow::Context;
use lopdf::Document;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: pdf-dump <file.pdf>");
        process::exit(1);
    }

    let path = Path::new(&args[1]);
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }

    let doc =
        Document::load(path).with_context(|| format!("failed to load PDF: {}", path.display()))?;

    let page_count = doc.get_pages().len();
    println!("File: {}", path.display());
    println!("Pages: {page_count}");
    println!();

    let toc = match doc.get_toc() {
        Ok(toc) => toc,
        Err(e) => {
            println!("No table of contents found: {e}");
            println!("This PDF has no outlines/bookmarks — classified as Simple (not split).");
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
        println!("Table of contents is empty — classified as Simple (not split).");
        return Ok(());
    }

    // Filter to top-level (level 1) entries only — these are what the splitter uses
    let top_level: Vec<_> = toc.toc.iter().filter(|e| e.level == 1).collect();

    if top_level.is_empty() {
        println!(
            "TOC has {} entries but none at level 1 — classified as Simple (not split).",
            toc.toc.len()
        );
        return Ok(());
    }

    // Collect unique sorted page starts
    let mut page_starts: Vec<usize> = top_level.iter().map(|e| e.page).collect();
    page_starts.sort();
    page_starts.dedup();

    println!(
        "TOC: {} total entries, {} top-level (level 1)",
        toc.toc.len(),
        top_level.len()
    );
    println!();

    // Show chapter split blocks
    println!(
        "{:>5}  {:<50} {:>8}  {:>8}  {:>5}",
        "Block", "Title", "Start", "End", "Pages"
    );
    println!("{}", "─".repeat(82));

    for (i, &start) in page_starts.iter().enumerate() {
        let end = if i + 1 < page_starts.len() {
            page_starts[i + 1] - 1
        } else {
            page_count
        };

        let title = top_level
            .iter()
            .find(|e| e.page == start)
            .map(|e| e.title.as_str())
            .unwrap_or("(unnamed)");

        let display_title = match title.char_indices().nth(50) {
            Some((idx, _)) => &title[..idx],
            None => title,
        };

        let page_span = if end >= start { end - start + 1 } else { 0 };

        println!(
            "{:>5}  {:<50} {:>8}  {:>8}  {:>5}",
            i + 1,
            display_title,
            start,
            end,
            page_span
        );
    }

    println!();
    println!(
        "{} chapters, {} pages — {}",
        page_starts.len(),
        page_count,
        if page_count >= 50 {
            "BOOK (would split)"
        } else {
            "SIMPLE (too few pages)"
        }
    );

    Ok(())
}
