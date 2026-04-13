# Code Review Findings — Aggregated

> **Status: Historical / Non-normative** (snapshot from 2026-04-09)
>
> These findings are a point-in-time record from two review rounds. They are not open requirements.
> Some issues have been fixed in subsequent commits; others may no longer apply after the multi-wiki
> redesign. Check the current code before treating any finding as actionable.

Two rounds of review by 5+3 agents. Findings deduplicated and prioritized.

## Critical (4 issues)

### C1. Path traversal in MCP wiki tools
**Files:** `wiki.rs:50-65`, `main.rs:256-289` (MCP server)
**Found by:** Round 1 (correctness, security, concurrency) + Round 2 (MCP, wiki)
`read_page`, `write_page`, `list_pages` accept arbitrary relative paths like `../../etc/passwd`. No validation that resolved path stays within wiki root. MCP tools accept untrusted LLM input.
**Fix:** After `self.root.join(relative_path)`, normalize and verify the path starts with `self.root`. Reject any path containing `..` components.

### C2. Blocking I/O on async runtime in MCP server
**Files:** `main.rs:124-320` (MCP server)
**Found by:** Round 1 (concurrency)
All 12 tool handlers do synchronous `std::sync::Mutex` locking + SQLite queries + filesystem I/O directly on tokio worker threads. Under concurrent load, this starves the async runtime.
**Fix:** Wrap all tool handler bodies in `tokio::task::spawn_blocking`.

### C3. TUI missing panic hook — terminal left in raw mode on crash
**File:** `tui.rs:15-32`
**Found by:** Round 2 (wiki/config/TUI)
If any code panics during TUI execution, `disable_raw_mode` and `LeaveAlternateScreen` are never called. Terminal becomes unusable.
**Fix:** Install a panic hook before entering raw mode that restores the terminal.

### C4. Tesseract OCR fallback is non-functional
**File:** `pdf.rs:174-179`
**Found by:** Round 1 (correctness) + Round 2 (preprocessing)
Tesseract cannot accept PDF files as input. The OCR fallback path always fails silently. Need to render PDF pages to images first (e.g., `pdftoppm`), then run tesseract on each image.
**Fix:** Either implement proper PDF-to-image-to-OCR pipeline, or remove the non-functional fallback and document the limitation.

## Major (9 issues)

### M1. ZIP extraction flattens directories — silent file overwrite
**File:** `zip_extract.rs:30-33`
**Found by:** Round 1 (correctness) + Round 2 (preprocessing)
Zip-slip protection strips directory components. `a/config.txt` and `b/config.txt` both become `config.txt`; second overwrites first.
**Fix:** Preserve relative directory structure under output_dir (after sanitizing `..` components), or disambiguate colliding names.

### M2. PDF chapter splitting with unsorted outlines produces wrong page ranges
**File:** `pdf.rs:82-117`
**Found by:** Round 1 (correctness)
`page_starts` collected from outlines may not be in page order. Range computation produces nonsensical results (e.g., pages 10-2).
**Fix:** Sort and dedup `page_starts` after collecting.

### M3. `classify_pdf` never returns `Sensitive` variant
**File:** `pdf.rs:40-53`, `ingest.rs:164-171`
**Found by:** Round 1 (correctness, functionality) + Round 2 (preprocessing)
The enum variant exists, ingest handles it, but `classify_pdf` never produces it. PDF metadata is never inspected for sensitive patterns.
**Fix:** Either implement metadata inspection in `classify_pdf` or remove the dead variant and handler.

### M4. MCP server doesn't reset `in_progress` items on startup
**File:** `main.rs` (MCP server, main function)
**Found by:** Round 1 (functionality)
After crash, `in_progress` items are stuck forever. The CLI ingest command resets them, but the MCP server doesn't.
**Fix:** Add `queue.reset_in_progress()?;` after opening the queue in MCP server's `main`.

### M5. No guard on queue status transitions
**File:** `queue.rs:180-230`
**Found by:** Round 2 (MCP/queue)
`mark_complete` doesn't verify item is `in_progress`; `mark_in_progress` doesn't verify item is `queued`. Any item can jump to any status.
**Fix:** Add `AND status = ?` to WHERE clauses. Return `InvalidTransition` error when item exists but is in wrong state.

### M6. Non-atomic get-then-update in `get_next_item`
**File:** `queue.rs` (get_next_queued + mark_in_progress), `main.rs:126-138`
**Found by:** Round 2 (MCP/queue)
Two separate SQL statements for fetch-then-update. Another process accessing the same DB could grab the same item.
**Fix:** Use a single `UPDATE ... WHERE status='queued' ... RETURNING *` statement, or wrap in an explicit transaction.

### M7. Non-atomic wiki writes — concurrent update_index/append_log loses data
**File:** `wiki.rs:87-105`
**Found by:** Round 1 (correctness) + Round 2 (wiki)
Read-modify-write without locking. Two concurrent MCP calls can lose one update.
**Fix:** Use `fs::OpenOptions` with append mode for atomic appends, or add file locking.

### M8. Ingest counting inaccuracy — items counted as both queued and errored
**File:** `ingest.rs:96-97, 114-119, 128-129, 154-159`
**Found by:** Round 2 (preprocessing)
ZIP and PDF-Book paths increment `queued` on enqueue, then also increment `errors` if extraction fails. Final totals double-count. The Sensitive PDF path correctly uses `saturating_sub` but other error paths don't.
**Fix:** Decrement `queued` on error (like the Sensitive path does) or only count as queued after success.

### M9. Config has no validation
**File:** `config.rs:75-83`
**Found by:** Round 2 (wiki/config/TUI)
Empty tool paths, `book_min_pages = 0`, nonexistent directories silently accepted.
**Fix:** Add `Config::validate()` called after `load()`.

## Minor (14 issues)

| # | Issue | File | Fix |
|---|-------|------|-----|
| m1 | Symlink following in walk_dir/collect_md_files — infinite loops | ingest.rs:348, wiki.rs:199 | Check `is_symlink()`, skip |
| m2 | No dedup on re-ingest | queue.rs:136-150 | Add UNIQUE constraint or check before enqueue |
| m3 | `to_string_lossy` silently corrupts non-UTF-8 paths | queue.rs:169 | Use `to_str()` + error |
| m4 | Temp file leaks from OCR | pdf.rs:168-189 | Use tempfile crate or explicit cleanup |
| m5 | Intermediate extract dirs never cleaned up | ingest.rs:98,132,267 | Cleanup after processing |
| m6 | Unnecessary String clones in TUI render loop | tui.rs:152,154 | Use `as_deref()` |
| m7 | `all_children_complete` returns true for parentless items | queue.rs:335-343 | Document or return distinct response |
| m8 | `read_source` doesn't verify item exists | main.rs:243-250 | Query queue first |
| m9 | Relative paths in @file list resolved from CWD, not list file dir | ingest.rs:371 | Document or resolve relative to list file |
| m10 | Error messages in MCP expose absolute filesystem paths | main.rs:249 | Sanitize paths in MCP responses |
| m11 | ZIP bomb / recursive ZIP — no size/depth limits | zip_extract.rs, ingest.rs | Add max decompressed size and recursion depth |
| m12 | Sensitive patterns bypassable via Unicode homoglyphs | detect.rs:26-35 | Normalize Unicode before comparison |
| m13 | Generated CLAUDE.md is static, could drift from actual tool names | wiki.rs:111-184 | Add comment noting coupling |
| m14 | `whisper-cpp` output read order fragile (stdout vs file) | media.rs:56-57 | Read file first, fall back to stdout |
