# Application

> **Authority note:** This document describes the current multi-wiki architecture.
> For the canonical config/resolution spec, see `../superpowers/specs/2026-04-10-multi-wiki-design.md`.
> For the doc hierarchy, see `../README.md` (docs).

The application suite is a Cargo workspace with four crates. A single `ai-wiki` CLI binary handles all user-facing operations via subcommands.

## CLI: `ai-wiki`

```
ai-wiki [--wiki <name>] <command>
```

Wiki selection: if `--wiki <name>` is given, that named wiki is used. Otherwise the CLI checks whether the current working directory is at or under a registered wiki root. If neither matches, available wikis are listed and the command exits with an error.

Configuration is stored centrally at `~/.ai-wiki/config.toml`. See the multi-wiki spec for details.

### `ai-wiki init [--name <name>] [<directory>]`

Creates a new wiki directory structure and registers it in `~/.ai-wiki/config.toml`. Name defaults to the directory name; directory defaults to CWD. Creates the config file if it doesn't exist.

### `ai-wiki list`

Lists all registered wikis with name, root path, and queue counts.

### `ai-wiki ingest [--wiki <name>] <path>`

Reads source files, classifies them by type, extracts text, and adds items to the processing queue. No LLM is involved — this is pure Rust preprocessing.

The `<path>` argument accepts:
1. **Single file**: `ai-wiki ingest ~/Downloads/paper.pdf`
2. **Directory**: `ai-wiki ingest ~/Downloads/rust-books/` — walks all files recursively (max depth 50)
3. **Glob pattern**: `ai-wiki ingest "~/Downloads/*.pdf"` — expanded by ai-wiki, not the shell (use quotes)
4. **File list**: `ai-wiki ingest @my-reading-list.txt` — reads one path per line; `#` comments and blank lines are skipped; leading/trailing quotes on each path are stripped

Supported file types:
- PDF: text extracted via pdf-extract, pdftotext, or OCR. Books with a table of contents are split into chapters automatically.
- Markdown/Text: copied directly to the processed directory.
- ZIP: extracted and each contained file processed recursively.
- Audio/Video: audio extracted with ffmpeg, transcribed with whisper-cpp.
- Unknown file types: rejected immediately.

Duplicate files (same path + parent) are detected and skipped automatically.

Progress is shown per file, with a summary at the end:
```
[1/794] document.pdf ... queued (0.3s)
[2/794] installer.dmg ... rejected (0.0s)
Ingest complete — queued: 500, rejected: 12, errors: 3, skipped: 279, failed: 0 (4m 23s)
```

### `ai-wiki process [--wiki <name>]`

Invokes the Claude CLI to process every queued item in the database. Claude reads each item's extracted text, identifies entities, concepts, and claims, creates wiki pages with YAML frontmatter and `[[wikilinks]]`, updates the index and log, and marks items complete.

Requires the `claude` CLI to be installed and on PATH. Runs with `--dangerously-skip-permissions` — only process documents you trust.

Book parents (items with chapters) are summarized from their children's processed text.

### `ai-wiki tui [--wiki <name>]`

Opens a terminal UI showing all queue items with color-coded status:
- Gray: queued
- Yellow: in progress
- Green: complete
- Red: error/rejected

Keyboard controls:
- `↑`/`↓` — navigate items
- `Enter` — view details (error message, rejection reason, or wiki page content)
- `R` — retry an errored/rejected item (requeue it)
- `r` — force refresh
- `q`/`Esc` — quit

### `ai-wiki retry [--wiki <name>]`

Requeues errored items that have extracted text in the processed directory, then runs `process` to have Claude build their wiki pages.

This is for items where text extraction succeeded but wiki page creation failed (e.g., Claude timeout, network error). Items without processed text are left as errors — use `clear` to remove them, then re-ingest.

### `ai-wiki clear [--wiki <name>]`

Deletes all items with `error` status from the queue database. Use this to clean up items that failed text extraction and cannot be retried without re-ingesting the original files. Also deletes errored child items of errored parents.

After clearing, re-ingest the original files:
```bash
ai-wiki clear
ai-wiki ingest ~/Downloads/*.pdf
```

The dedup check skips files that were already successfully processed and only picks up previously failed ones.

### `ai-wiki queue [--wiki <name>] <subcommand>`

Low-level queue operations used by the Claude process prompt. Not typically called by users directly.

#### `ai-wiki queue claim`

Atomically claim the next queued item and print its details as tab-separated fields:
```
<ID>\t<file_path>\t<file_type>\t<parent_id_or_none>
```
Prints `EMPTY` if the queue is exhausted.

#### `ai-wiki queue complete <ID> <wiki_page_path>`

Mark an in-progress item as complete, recording the relative path to the created wiki page.

#### `ai-wiki queue error <ID> <message>`

Mark an item as errored with a descriptive message.

## TUI Detail View

Pressing `Enter` on a terminal-state item shows:
- **Errors**: the error message and stack trace
- **Rejected**: the rejection reason
- **Complete**: the full wiki page content (read from the wiki directory)

## Utility: `pdf-dump`

Diagnostic tool for inspecting how a PDF will be split into chapters.

```bash
cargo run -p pdf-dump -- ~/Downloads/some-book.pdf
```

Output:
1. File path and page count
2. TOC parsing warnings (if any)
3. Chapter split blocks — how the book would be divided (top-level outline entries only, with start/end page numbers and page span)
4. Classification verdict: BOOK (would split) or SIMPLE (too few pages)

Useful for understanding why a particular PDF was or was not split, or for diagnosing issues with PDF bookmark structure.

## MCP Server: `ai-wiki-mcp`

Long-running process connected to Claude Code via `claude mcp add`. Reads `~/.ai-wiki/config.toml` on startup and serves all registered wikis. Every tool requires a `wiki` parameter (the registered wiki name) to identify which wiki to operate on.

### Queue Tools
- `get_next_item(wiki)` — claim the next queued item (marks it `in_progress`), returns JSON
- `complete_item(wiki, id, wiki_page_path)` — mark item complete; returns whether all siblings are done
- `reject_item(wiki, id, reason)` — mark item rejected
- `error_item(wiki, id, message)` — mark item errored
- `list_items(wiki, status?)` — list queue items, optionally filtered by status

### Source Tools
- `read_source(wiki, id)` — read preprocessed text from `processed/<id>.txt`

### Wiki Tools
- `read_page(wiki, path)` — read a wiki page by relative path
- `write_page(wiki, path, content)` — create or overwrite a wiki page
- `list_pages(wiki, directory?)` — list pages, optionally within a subdirectory
- `read_index(wiki)` — read current `index.md`
- `update_index(wiki, entry)` — append an entry to `index.md`
- `append_log(wiki, entry)` — append a timestamped entry to `log.md`

## Library: `ai-wiki-core`

Shared library crate providing:
- `config` — central TOML config loading/saving (`~/.ai-wiki/config.toml`), validation, per-wiki path helpers
- `queue` — SQLite-backed job queue with WAL mode, atomic claim, status transitions
- `preprocessing` — file type detection, PDF classification/splitting/extraction, ZIP extraction, audio/video transcription
- `wiki` — wiki file read/write operations, index and log management

## Workspace Structure

```
ai-wiki/                          # Source repository
├── crates/
│   ├── ai-wiki-core/             # Library: config, queue, preprocessing, wiki
│   ├── ai-wiki/                  # CLI: init, ingest, process, tui, retry, clear, list, queue
│   ├── ai-wiki-mcp/              # MCP server: multi-wiki, all tools require wiki param
│   └── pdf-dump/                 # Diagnostic utility for PDF inspection
├── docs/
│   ├── design/                   # Architecture and workflow docs
│   ├── superpowers/              # Specs, plans, review findings
│   └── archive/                  # Raw session transcripts (non-authoritative)
└── justfile                      # Task runner recipes

~/.ai-wiki/
└── config.toml                   # Central config: tool paths + wiki registry

~/wikis/<name>/                   # Each registered wiki root
├── wiki/                         # Obsidian vault
├── processed/                    # Extracted text files
├── raw/                          # Split PDFs and extracted ZIPs
└── ai-wiki.db                    # SQLite queue database
```
