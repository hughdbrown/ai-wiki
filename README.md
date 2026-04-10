# ai-wiki

A Rust application that builds and maintains a personal knowledge base by having an LLM incrementally process your documents into an interlinked Obsidian wiki.

## What is this?

Most people's experience with LLMs and documents looks like RAG: upload files, retrieve chunks at query time, generate an answer. The LLM rediscovers knowledge from scratch on every question. Nothing accumulates.

ai-wiki takes a different approach. Instead of retrieving from raw documents at query time, the LLM **incrementally builds and maintains a persistent wiki** -- a structured collection of markdown files that sits between you and your sources. When you add a new document, the LLM reads it, extracts the key information, and integrates it into the existing wiki: updating entity pages, creating concept pages, noting claims and data points, cross-referencing everything with `[[wikilinks]]`. The knowledge is compiled once and kept current, not re-derived on every query.

The wiki is the artifact. It keeps getting richer with every source you add.

## Inspiration

This project implements the pattern described in [Tobi Lutke's "LLM Wiki" gist](https://gist.github.com/tobi/2a735ef683eab0c89e9e78e1e31ee920), which was itself inspired by ideas from Andrej Karpathy about using LLMs to build personal knowledge bases. The core insight: LLMs are good at the grunt work humans abandon -- summarizing, cross-referencing, filing, maintaining consistency across hundreds of pages. The human curates sources and asks questions; the LLM does the bookkeeping.

## Architecture

```
Sources (PDFs, text, markdown)
        |
        v
  ┌─────────────┐     ┌─────────────┐
  │  ai-wiki     │     │  ai-wiki    │
  │  ingest      │────>│  process    │───> Obsidian Wiki
  │  (Rust CLI)  │     │  (Claude)   │     (markdown files)
  └─────────────┘     └─────────────┘
        |                    |
        v                    v
    SQLite Queue        Wiki Pages
    Processed Text      Index & Log
```

Three layers:

- **Raw sources** -- your PDFs, articles, text files. Immutable. The app reads but never modifies them.
- **The wiki** -- LLM-generated Obsidian vault. Summaries, entity pages, concept pages, claims, an index, a log. The LLM owns this layer.
- **The application** -- Rust CLI that preprocesses sources and manages the queue. Claude does the knowledge work.

## Setup

### Prerequisites

**Rust toolchain:**
```bash
# Option 1: Official rustup installer
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Option 2: Install via Homebrew (macOS)
brew install rustup
```

**External tools** (for PDF processing):
```bash
brew install qpdf poppler tesseract
```

Optional (for audio/video transcription):
```bash
brew install ffmpeg whisper-cpp
```

**Claude Code** (for the `process` command):
```bash
npm install -g @anthropic-ai/claude-code
```

**Fast linker** (recommended, speeds up Rust builds):
```bash
brew install lld
```

### Build

```bash
git clone <this-repo>
cd ai-wiki
cargo build --release
```

The release build produces two binaries:
- `target/release/ai-wiki` -- the CLI (ingest, TUI, process)
- `target/release/ai-wiki-mcp` -- the MCP server (for direct Claude Code integration)

### Configure

On first run, `ai-wiki` creates a default `ai-wiki.toml`. Edit it to set absolute paths:

```toml
[paths]
raw_dir = "/path/to/your/raw/sources"
wiki_dir = "/path/to/your/obsidian/vault"
database_path = "/path/to/ai-wiki.db"
processed_dir = "/path/to/processed/text"

[pdf]
book_min_pages = 50    # PDFs with outlines + this many pages are split into chapters

[rejection]
non_operative_extensions = [".dmg"]
sensitive_filename_patterns = ["divorce", "court", "financial", "tax.return"]

[tools]
qpdf_path = "qpdf"
pdftotext_path = "pdftotext"
pdftoppm_path = "pdftoppm"
tesseract_path = "tesseract"
ffmpeg_path = "ffmpeg"
whisper_cpp_path = "whisper-cpp"
whisper_model_path = "models/ggml-large-v3.bin"
```

### Open in Obsidian

Point Obsidian at your `wiki_dir`. You'll see the wiki grow as items are processed.

## Usage

ai-wiki operates in two phases: **load** and **process**.

### Phase 1: Load (Ingest)

Ingest reads your source files, classifies them, extracts text, and queues them for LLM processing. No LLM is involved in this phase -- it's pure Rust.

```bash
# Ingest a single file
ai-wiki ingest ~/Downloads/paper.pdf

# Ingest a directory
ai-wiki ingest ~/Downloads/rust-books/

# Ingest with a glob pattern (quotes prevent shell expansion; ai-wiki expands globs internally)
ai-wiki ingest "~/Downloads/*.pdf"

# Ingest from a file list (one path per line, # comments allowed)
ai-wiki ingest @my-reading-list.txt
```

What happens during ingest:

- **Markdown/text files** are copied to the processed directory as-is.
- **PDFs** are classified:
  - Simple PDFs: text is extracted (pdf-extract, pdftotext, or OCR as fallback).
  - Books (outlines + 50+ pages): split into chapters via `qpdf`, each chapter extracted separately.
  - Sensitive files (matching rejection patterns): rejected and logged.
- **ZIP files** are extracted and each contained file is processed recursively.
- **Audio/video** (MP4, MKV, etc.): audio extracted with ffmpeg, transcribed with whisper-cpp.
- **Non-operative files** (.dmg, etc.): rejected immediately.

Each file gets a queue entry in SQLite. Duplicate files are detected and skipped automatically.

Progress is shown for each file:
```
[1/794] document.pdf ... queued (0.3s)
[2/794] installer.dmg ... rejected (0.0s)
[3/794] already-done.pdf ... skipped (0.0s)
Ingest complete — queued: 500, rejected: 12, errors: 3, skipped: 279 (4m 23s)
```

### Phase 2: Process (LLM)

Processing invokes Claude to read the extracted text and build wiki pages.

> **Security note:** The `process` command runs Claude with broad tool access (`--dangerously-skip-permissions`). Only process documents you trust, as source content could potentially trigger unintended actions via prompt injection.

```bash
ai-wiki process
```

This processes all queued items. For each item, Claude:
1. Reads the extracted text
2. Identifies entities, concepts, and claims
3. Creates wiki pages with YAML frontmatter and `[[wikilinks]]`
4. Updates the index and log
5. Marks the item complete

The wiki follows Obsidian conventions:
- `entities/` -- people, organizations, tools
- `concepts/` -- ideas, theories, techniques
- `claims/` -- specific assertions with `data-point: true` tag
- `sources/` -- summaries of ingested documents
- `index.md` -- catalog of all pages
- `log.md` -- chronological record of ingestions
- `CLAUDE.md` -- schema telling the LLM how to maintain the wiki

### Monitor

```bash
ai-wiki tui
```

A terminal UI showing queue status with color-coded entries:
- Gray: queued
- Yellow: in progress
- Green: complete
- Red: error/rejected

Press `Enter` on any terminal-state item to see details:
- **Errors**: the error message
- **Rejected**: the rejection reason
- **Complete**: the full wiki page content

Press `R` on an errored/rejected item to requeue it for retry.

## Utilities

### pdf-dump

Diagnostic tool for inspecting how a PDF will be split into chapters.

```bash
cargo run -p pdf-dump -- ~/Downloads/some-book.pdf
```

Output shows:
1. **Table of contents** -- all outline entries with nesting level and page number
2. **Split blocks** -- how the book would be divided into chapters (top-level entries only, with start/end page numbers)
3. **Comparison** -- flags if the current splitting code would over-fragment the book by using sub-sections instead of just top-level entries

Example:
```
File: /Users/you/Downloads/some-book.pdf
Pages: 542

TABLE OF CONTENTS (491 entries)
  1. Contributors                         page 5
  2. Preface                              page 22
  3. Foundations of Agent Engineering      page 32
  ...

SPLIT BLOCKS (98 chapters from 121 top-level entries)
  Block  Title                                   Start    End   Pages
  ─────────────────────────────────────────────────────────────────
      1  Contributors                                5      21     17
      2  Preface                                    22      31     10
      3  Foundations of Agent Engineering            32      48     17
  ...

Book detection: 121 top-level entries, 542 pages → BOOK (would split)
```

This is useful for understanding why a particular PDF was split the way it was, or for diagnosing issues with PDF bookmark structure.

## Project Structure

```
ai-wiki/
├── crates/
│   ├── ai-wiki-core/     # Library: config, queue, preprocessing, wiki operations
│   ├── ai-wiki/          # CLI binary: ingest, tui, process
│   ├── ai-wiki-mcp/      # MCP server: 12 tools for Claude Code integration
│   └── pdf-dump/         # Diagnostic utility for PDF inspection
├── docs/
│   ├── design/           # Original design documents
│   └── superpowers/      # Implementation plans and review findings
├── wiki/                 # Generated Obsidian vault (gitignored content)
├── processed/            # Extracted text files (gitignored)
├── raw/                  # Split PDFs and extracted ZIPs (gitignored)
├── ai-wiki.toml          # Configuration
├── justfile              # Task runner recipes
└── README.md
```

## Development

```bash
just check      # Fast compile check
just test       # Run all 74 tests
just lint        # Clippy lints
just ci          # Full CI pipeline (check + test + lint + fmt)
just build       # Debug build
just release     # Optimized release build
```

## License

MIT
