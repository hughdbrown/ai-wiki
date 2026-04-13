# AI Wiki — Design Specification

> **Status: Superseded** (2026-04-09, single-wiki model)
>
> This spec describes the original single-wiki architecture. It has been superseded by
> `2026-04-10-multi-wiki-design.md`, which replaces per-project config with a central
> `~/.ai-wiki/config.toml` and adds multi-wiki support. This document is retained as
> historical context — see `../../README.md` (docs) for the authority hierarchy.

## Overview

A Rust application that preprocesses source files (PDFs, markdown, text, ZIPs, audio/video) and exposes an MCP server for Claude Code to drive LLM-powered wiki generation. The wiki is an Obsidian-native vault of interlinked markdown pages that compounds knowledge over time.

## Architecture

Three-layer system (raw sources, wiki, schema) plus the application that operates on them:

1. **Raw sources** — immutable user-curated files (PDFs, articles, media). The application reads but never modifies these.
2. **The wiki** — LLM-generated Obsidian vault. Markdown pages with `[[wikilinks]]`, YAML frontmatter, organized by category. The LLM owns this layer entirely.
3. **The schema** — a `CLAUDE.md` file in the wiki vault root that tells the LLM how the wiki is structured, what conventions to follow, and what workflows to use when ingesting sources or updating pages. This is co-evolved by the user and LLM over time. It specifies: page format expectations, frontmatter schema, the step-by-step ingestion workflow, how to handle contradictions, and cross-referencing conventions.
4. **The application** — Rust workspace that preprocesses sources, manages a job queue, and exposes MCP tools for the LLM to build the wiki.

### Crate Structure

Cargo workspace with four crates:

```
ai-wiki/
├── Cargo.toml              # workspace root
├── crates/
│   ├── ai-wiki-core/       # library — all domain logic
│   ├── ai-wiki/            # CLI binary — ingest, tui, process, retry, clear, queue
│   ├── ai-wiki-mcp/        # MCP server binary
│   └── pdf-dump/           # diagnostic utility for PDF chapter inspection
├── raw/                    # split PDFs and extracted ZIPs (configurable path)
└── wiki/                   # Obsidian vault (configurable path)
    ├── index.md
    ├── log.md
    ├── CLAUDE.md             # schema — LLM wiki conventions
    ├── entities/
    ├── concepts/
    ├── claims/
    └── sources/
```

**Deviation from spec:** The spec planned three crates. A fourth crate (`pdf-dump`) was added as a diagnostic utility for inspecting PDF chapter splitting behavior.

## Library Crate: `ai-wiki-core`

Four modules:

### `queue`

Job queue backed by SQLite.

- Each source file becomes a queue item with fields: id, file path, file type, status, timestamps, parent item ID (for ZIP contents / PDF chapters), wiki page path (populated on completion), error message
- Statuses: `queued`, `in_progress`, `complete`, `rejected`, `error`
- On application restart, `in_progress` items reset to `queued`
- Both CLI and MCP server access the same SQLite database (WAL mode enabled for concurrent read access; only one writer at a time)

### `preprocessing`

File inspection and preparation. No LLM calls.

- **File type detection**: by extension + magic bytes. Non-operative file types (`.dmg` and others) are rejected immediately and logged.
- **PDF classifier**: inspects page count and outline structure via `lopdf`'s `get_toc()` method. Heuristics:
  - Has TOC with at least one level-1 entry + `book_min_pages` (default 50) pages = book (split into chapters)
  - Filename matching sensitive patterns = reject (court documents, financial statements, tax returns)
  - Otherwise = simple PDF, ingest as-is
  - **Deviation from spec:** `get_toc()` is used instead of raw outline inspection. This properly resolves titles, page numbers, and nesting levels. Only top-level (level 1) entries are used for splitting — sub-sections are ignored to avoid over-fragmenting the book.
- **PDF splitter**: extracts chapter boundaries from top-level bookmark entries, splits via `qpdf` CLI. Each chapter enqueued as a child item of the book parent.
- **PDF text extraction**: Three-stage fallback chain:
  1. `pdf-extract` Rust crate for embedded text. **Deviation from spec:** Wrapped in `std::panic::catch_unwind` because the upstream cff-parser crate can panic on malformed PDFs with CFF font encoding issues.
  2. `pdftotext` (poppler) CLI as fallback for unusual encodings or empty pdf-extract results.
  3. `pdftoppm` + `tesseract` OCR for scanned PDFs (no extractable text). Renders pages to PPM images, OCRs each page, concatenates results.
  - Preprocessed text written to `processed/<id>.txt`, keyed by queue item ID.
- **ZIP extractor**: unpacks archive, enqueues each contained file as a child item
- **Audio/video**: shells out to `ffmpeg` to extract audio from MP4/MKV, then `whisper-cpp` for transcription to markdown

### `wiki`

Wiki file operations. These become the backing implementations for MCP tools.

- Read/write/update markdown pages in the Obsidian vault
- Manage `[[wikilinks]]` and YAML frontmatter
- Append timestamped entries to `log.md`
- Read and update `index.md`
- Create pages in the correct subdirectory (`entities/`, `concepts/`, `claims/`, `sources/`)

### `config`

Application configuration loaded from a TOML file.

- Paths: raw source directory, wiki vault directory, SQLite database
- PDF classification thresholds (page count for book detection)
- Sensitive file rejection patterns
- Non-operative file extensions (`.dmg`, etc.)
- External tool paths (qpdf, pdftotext, tesseract, ffmpeg, whisper-cpp)
- Whisper model path (GGML model file, see Setup Requirements)
- Sensible defaults for all values

## CLI Binary: `ai-wiki`

Six subcommands via `clap`:

### `ai-wiki ingest <path>`

- `<path>`: a file, glob pattern (expanded by the application via the `glob` crate, not shell expansion), directory, or `@filelist`
- **Deviation from spec:** Added `@filelist` support: prefix with `@` to read a list of file paths (one per line, `#` comments skipped, quoted paths supported).
- Walks the path, runs preprocessing on each file (classify, split, extract)
- Enqueues items into SQLite with appropriate status and parent linkage
- Does **not** call the LLM — only prepares the queue
- Prints per-file progress and a final summary: N queued, N rejected, N errors, N skipped, N failed

### `ai-wiki tui`

- Terminal UI via `ratatui` + `crossterm`
- Displays queue items with color-coded status: gray=queued, yellow=in-progress, green=complete, red=rejected/error
- Auto-refreshes by polling SQLite
- Press `Enter` to view item details (error message, rejection reason, or wiki page content)
- Press `R` to requeue an errored/rejected item
- **Deviation from spec:** No expandable rows for child items — all items shown in a flat list.

### `ai-wiki process`

**New command (not in original spec).** Invokes the Claude CLI to process all queued items.
- Builds a prompt with instructions for Claude to use the `queue` subcommands
- Launches `claude --print --dangerously-skip-permissions` with the prompt on stdin
- Path injection guard: validates that config paths contain only safe characters before embedding in prompt

### `ai-wiki retry`

**New command (not in original spec).** Requeues errored items that have processed text available, then runs `process`.

### `ai-wiki clear`

**New command (not in original spec).** Deletes all errored items from the queue database (and their errored children). Use when text extraction failed and items need to be re-ingested.

### `ai-wiki queue <subcommand>`

**New command (not in original spec).** Low-level queue operations used by the Claude prompt:
- `claim` — atomically claim the next queued item, print tab-delimited details
- `complete <ID> <wiki_page_path>` — mark item complete
- `error <ID> <message>` — mark item errored

## MCP Server: `ai-wiki-mcp`

Long-running process connected to Claude Code via `claude mcp add`. Exposes 11 MCP tools for LLM-driven wiki building. Uses the `rmcp` crate (not the originally planned custom implementation).

### Queue Tools (5)

- `get_next_item` — returns next `queued` item as JSON, marks it `in_progress`
- `complete_item(id, wiki_page_path)` — marks item `complete`; returns whether all siblings are done
- `reject_item(id, reason)` — marks item `rejected` with explanation
- `error_item(id, message)` — marks item `error`
- `list_items(status?)` — list queue items, optionally filtered by status

### Source Tools (1)

- `read_source(id)` — returns preprocessed text from `processed/<id>.txt`; verifies item exists in queue first

### Wiki Tools (5)

- `read_page(path)` — read an existing wiki page by relative path
- `write_page(path, content)` — create or overwrite a wiki page
- `list_pages(directory?)` — list pages, optionally within a subdirectory
- `read_index` — read current `index.md`
- `update_index(entry)` — append an entry to `index.md`
- `append_log(entry)` — append a timestamped entry to `log.md` with `## [YYYY-MM-DD]` prefix

**Note:** `read_index` and `append_log` are implemented but count toward the 11 — `update_index` and `append_log` are separate tools (spec counted `update_index` and `append_log` separately, which is correct).

**Deviation from spec:** The spec listed `update_index(entry)` as adding/updating entries. The implementation only appends — there is no update-in-place of existing entries.

### LLM Workflow (unchanged from spec)

1. `get_next_item` — receive a source to process
2. `read_source` — read its content
3. `read_index` + `read_page` calls — understand existing wiki state
4. Generate wiki content (summaries, entity updates, concept pages, cross-references, contradiction flags)
5. `write_page` for each page created or updated
6. `update_index` and `append_log`
7. `complete_item`
8. Repeat

## Wiki Format

Obsidian-native markdown:

- **Wikilinks**: `[[page-name]]` for cross-references between pages
- **YAML frontmatter**: on every page, with fields like `type` (entity/concept/source), `tags`, `sources`, `created`, `updated`
- **Directory structure**: `entities/`, `concepts/`, `claims/`, `sources/` subdirectories. Entities are people, places, organizations. Concepts are ideas, themes, theories. Claims are specific assertions from sources that may be supported or contradicted by other sources. Data points (statistics, measurements, dates, quantities) are stored as claims with a `data-point: true` frontmatter tag. Sources are summaries of ingested files.
- **Contradictions**: when a new source contradicts an existing claim or page, the LLM adds a `> [!warning] Contradiction` callout block on the affected page noting the conflicting sources. Contradicted claims are tagged `contradicted: true` in frontmatter for Dataview queries.
- **`index.md`**: catalog of all pages organized by category, each with a link and one-line summary
- **`log.md`**: append-only chronological record. Each entry prefixed with `## [YYYY-MM-DD] action | Title` for parseability

## Error Handling

- SQLite transactions ensure no corrupted state on crash
- On restart, `in_progress` items reset to `queued` for retry
- Failed preprocessing (corrupted PDF, bad ZIP) marks the item `error` and continues to the next
- Child items of a failed parent are not enqueued
- No retry loops — failures are marked and skipped. Review in TUI, re-queue manually if desired.
- MCP tool calls are independent — Claude Code disconnect mid-ingestion leaves item as `in_progress`, retried on reconnect

## Dependencies

### Rust Crates

- `clap` — CLI parsing
- `ratatui` + `crossterm` — TUI
- `rusqlite` — SQLite
- `lopdf` — PDF inspection
- `pdf-extract` — PDF text extraction
- `rmcp` — MCP server SDK
- `serde` + `toml` — config
- `zip` — ZIP extraction
- `glob` — file pattern expansion for ingest command
- `chrono` — timestamps

### External Tools

- `qpdf` — PDF splitting (`brew install qpdf`)
- `poppler` — PDF text extraction fallback via `pdftotext` (`brew install poppler`)
- `tesseract` — OCR for scanned PDFs (`brew install tesseract`)
- `ffmpeg` — audio extraction from video (`brew install ffmpeg`)
- `whisper-cpp` — audio transcription (`brew install whisper-cpp`)

### Setup Requirements

`whisper-cpp` requires GGML model files downloaded separately. Models are available at:
- https://huggingface.co/ggerganov/whisper.cpp/tree/main
- https://ggml.ggerganov.com/

The model path is configured in the TOML config file. Recommended: `ggml-large-v3.bin` for best accuracy on Apple Silicon with 128GB unified memory.

## Deferred

These are explicitly out of scope for v1 but the architecture supports adding them:

- **Search**: BM25/vector search over wiki pages (qmd, tantivy, or similar). Index.md is sufficient initially; search drops in as an additional MCP tool later.
- **Query operation**: Dedicated query feature in the Rust app. Claude Code can query the wiki directly by reading files. A structured search tool is the deferred piece.
- **Lint operation**: Wiki health checks (contradictions, orphans, stale claims). Can be added as an MCP tool or a CLI subcommand.
