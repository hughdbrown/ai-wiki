# AI Wiki — Design Specification

## Overview

A Rust application that preprocesses source files (PDFs, markdown, text, ZIPs, audio/video) and exposes an MCP server for Claude Code to drive LLM-powered wiki generation. The wiki is an Obsidian-native vault of interlinked markdown pages that compounds knowledge over time.

## Architecture

Three-layer system (raw sources, wiki, schema) plus the application that operates on them:

1. **Raw sources** — immutable user-curated files (PDFs, articles, media). The application reads but never modifies these.
2. **The wiki** — LLM-generated Obsidian vault. Markdown pages with `[[wikilinks]]`, YAML frontmatter, organized by category. The LLM owns this layer entirely.
3. **The schema** — a `CLAUDE.md` file in the wiki vault root that tells the LLM how the wiki is structured, what conventions to follow, and what workflows to use when ingesting sources or updating pages. This is co-evolved by the user and LLM over time. It specifies: page format expectations, frontmatter schema, the step-by-step ingestion workflow, how to handle contradictions, and cross-referencing conventions.
4. **The application** — Rust workspace that preprocesses sources, manages a job queue, and exposes MCP tools for the LLM to build the wiki.

### Crate Structure

Cargo workspace with three crates:

```
ai-wiki/
├── Cargo.toml              # workspace root
├── crates/
│   ├── ai-wiki-core/       # library — all domain logic
│   ├── ai-wiki/            # CLI binary — ingest + TUI
│   └── ai-wiki-mcp/        # MCP server binary
├── raw/                    # source files (configurable path)
│   └── assets/             # downloaded images
└── wiki/                   # Obsidian vault (configurable path)
    ├── index.md
    ├── log.md
    ├── CLAUDE.md             # schema — LLM wiki conventions
    ├── entities/
    ├── concepts/
    ├── claims/
    └── sources/
```

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
- **PDF classifier**: inspects page count, bookmark/outline structure (via `lopdf`), metadata. Heuristics:
  - Has TOC/outline + >50 pages = book (split into chapters)
  - Filename or metadata matching sensitive patterns = reject (court documents, financial statements, tax returns, children's report cards)
  - Otherwise = simple PDF, ingest as-is
- **PDF splitter**: extracts chapter boundaries from bookmarks, splits via `qpdf` CLI. Each chapter enqueued as a child item. When all child chapters of a book are complete, the parent book item triggers creation of a book summary page linking to all chapter pages.
- **PDF text extraction**: `pdf-extract` for embedded text. Falls back to `pdftotext` (poppler) for PDFs with unusual encodings, then to `tesseract` OCR for scanned PDFs (no extractable text). Preprocessed text is written to a `processed/` directory alongside the queue database, keyed by queue item ID.
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

Two subcommands via `clap`:

### `ai-wiki ingest <path>`

- `<path>`: a file, glob pattern (expanded by the application via the `glob` crate, not shell expansion), or directory
- Walks the path, runs preprocessing on each file (classify, split, extract)
- Enqueues items into SQLite with appropriate status and parent linkage
- Does **not** call the LLM — only prepares the queue
- Prints summary: N items queued, N rejected, N errors

### `ai-wiki tui`

- Terminal UI via `ratatui` + `crossterm`
- Displays queue items with columns: filename, type, status, started at, parent, wiki page link (when complete)
- Expandable rows for compound items (ZIP contents, book chapters)
- Color-coded status: gray=queued, yellow=in-progress, green=complete, red=rejected/error
- Auto-refreshes by polling SQLite
- Read-only monitoring view

## MCP Server: `ai-wiki-mcp`

Long-running process connected to Claude Code via `claude mcp add`. Exposes tools for LLM-driven wiki building.

### Queue Tools

- `get_next_item` — returns next `queued` item (path, type, parent info), marks it `in_progress`
- `complete_item(id, wiki_page_path)` — marks item `complete`, records the primary wiki page created for this source
- `reject_item(id, reason)` — marks item `rejected` with explanation
- `error_item(id, message)` — marks item `error`
- `list_items(status?)` — list queue items, optionally filtered by status

### Source Tools

- `read_source(id)` — returns preprocessed text content from the `processed/` directory (extracted PDF text, markdown content, transcription output)

### Wiki Tools

- `read_page(path)` — read an existing wiki page
- `write_page(path, content)` — create or overwrite a wiki page
- `list_pages(directory?)` — list pages, optionally within a subdirectory
- `read_index` — read current `index.md`
- `update_index(entry)` — add or update an entry in `index.md`
- `append_log(entry)` — append a timestamped entry to `log.md`

### LLM Workflow

From Claude Code's perspective, processing one source:

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
