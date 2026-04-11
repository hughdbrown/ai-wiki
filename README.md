# ai-wiki

A Rust application that builds and maintains personal knowledge bases by having an LLM incrementally process your documents into interlinked Obsidian wikis.

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

**Multi-wiki:** ai-wiki supports multiple independent wikis from a single installation. Each wiki has its own database, processed text, and Obsidian vault. A central config at `~/.ai-wiki/config.toml` registers all wikis.

## Setup

### Prerequisites

**Rust toolchain:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**External tools** (for PDF processing):
```bash
brew install qpdf poppler tesseract
```

Optional (for audio/video transcription):
```bash
brew install ffmpeg whisper-cpp
```

**Claude Code** (for the `process` command and querying):
```bash
npm install -g @anthropic-ai/claude-code
```

### Install

```bash
git clone <this-repo>
cd ai-wiki
just deploy    # installs ai-wiki and ai-wiki-mcp to ~/.cargo/bin
```

Or build manually:
```bash
cargo build --release
# Binaries at target/release/ai-wiki, target/release/ai-wiki-mcp
```

### Create a Wiki

```bash
mkdir ~/wikis/rust && cd ~/wikis/rust
ai-wiki init
```

This creates the directory structure and registers the wiki (named `rust`, from the directory name) in `~/.ai-wiki/config.toml`:

```toml
[tools]
qpdf_path = "qpdf"
pdftotext_path = "pdftotext"
# ...

[wikis.rust]
root = "/Users/you/wikis/rust"
```

You can create multiple wikis:
```bash
mkdir ~/wikis/python && cd ~/wikis/python
ai-wiki init

mkdir ~/wikis/ml && cd ~/wikis/ml
ai-wiki init --name machine-learning
```

List all registered wikis:
```bash
ai-wiki list
```

### Open in Obsidian

Point Obsidian at the `wiki/` subdirectory inside any wiki root (e.g., `~/wikis/rust/wiki/`). You'll see the wiki grow as items are processed.

## Usage

### Wiki Selection

When you're inside a wiki's directory, commands auto-detect which wiki to use:
```bash
cd ~/wikis/rust
ai-wiki ingest ~/Downloads/rust-book.pdf    # uses the "rust" wiki
```

From anywhere else, specify the wiki by name:
```bash
ai-wiki --wiki rust ingest ~/Downloads/rust-book.pdf
```

The `--wiki` flag always takes priority over directory detection.

### Phase 1: Load (Ingest)

Ingest reads source files, classifies them, extracts text, and queues them for LLM processing. No LLM is involved -- pure Rust preprocessing.

```bash
ai-wiki ingest ~/Downloads/paper.pdf
ai-wiki ingest ~/Downloads/rust-books/
ai-wiki ingest "~/Downloads/*.pdf"
ai-wiki ingest @my-reading-list.txt
```

What happens during ingest:

- **PDFs** with a table of contents are split into chapters via `qpdf`. Text extracted via pdf-extract, pdftotext, or OCR (pdftoppm + tesseract). Front-matter chapters (cover, copyright, TOC) with no extractable text are rejected gracefully.
- **Markdown/text** files are copied directly.
- **ZIP** archives are extracted and each file processed recursively.
- **Audio/video** (MP4, MKV, etc.) are transcribed via ffmpeg + whisper-cpp.
- **Unknown file types** are rejected.

PDF processing is resilient against malformed files: panics from third-party PDF crates are caught silently, stack overflows are isolated to a dedicated 16MB thread, and `qpdf` warnings are accepted without failing the split.

Duplicate files are detected and skipped automatically. Progress is shown per file:
```
[1/794] document.pdf ... queued (0.3s)
[2/794] installer.dmg ... rejected (0.0s)
[3/794] already-done.pdf ... skipped (0.0s)
Ingest complete — queued: 500, rejected: 12, errors: 3, skipped: 279 (4m 23s)
```

### Phase 2: Process (LLM)

```bash
ai-wiki process
```

Each queued item gets its own fresh Claude session with a two-pass prompt:
1. **Pass 1:** Read the source text, create a source summary page and entity pages.
2. **Pass 2:** Re-read the text, extract concepts and claims into dedicated wiki pages with cross-references.

Book parents (PDFs with chapters) have all their children's text gathered and processed in a single session.

**Authentication:** By default, `process` uses your Claude Code Pro subscription. To use an API key instead:

```bash
ai-wiki process --auth api     # uses ANTHROPIC_API_KEY
ai-wiki process --auth pro     # uses Pro subscription (default)
```

**Model selection:**

```bash
ai-wiki process --model opus
ai-wiki process --model sonnet
```

Both `auth` and `model` can be set as defaults in `~/.ai-wiki/config.toml`:

```toml
[process]
auth = "pro"
model = "sonnet"
```

CLI flags override the config.

> **Security note:** The `process` command grants Claude broad tool access. Only process documents you trust.

### Monitor

```bash
ai-wiki tui
```

Terminal UI with color-coded queue status. Keyboard shortcuts:

| Key | Action |
|-----|--------|
| `↑`/`↓` | Navigate items |
| `n`/`N` | Jump to next/previous book (skip chapters) |
| `g`/`G` | Jump to top/bottom |
| `Enter` | View item details |
| `R` | Retry errored/rejected item |
| `r` | Refresh |
| `q` | Quit |

### Error Recovery

```bash
ai-wiki retry    # requeue items with text, then process
ai-wiki clear    # delete errored items (re-ingest afterward)
```

## Querying a Wiki with Claude

Once your wiki has been built, you can query it directly from a Claude Code session. The wiki is just markdown files — Claude can read them with its built-in tools.

### Direct Querying (no MCP needed)

Start Claude Code in your wiki's directory:
```bash
cd ~/wikis/rust
claude
```

Then ask questions:
```
> What does this wiki say about the borrow checker?
> Compare the async approaches described across all sources
> Which books cover error handling? Summarize their perspectives.
> What entities are connected to the concept of memory safety?
```

Claude will read `wiki/index.md` to find relevant pages, then read those pages to answer your question. The wiki's `[[wikilinks]]` and frontmatter help Claude navigate the knowledge graph.

### Querying via MCP Server

For richer integration, register the MCP server:
```bash
# Add to ~/.claude/settings.json or project .claude/settings.json
```

```json
{
  "mcpServers": {
    "ai-wiki": {
      "command": "ai-wiki-mcp",
      "args": []
    }
  }
}
```

The MCP server reads `~/.ai-wiki/config.toml` and serves all registered wikis. Every tool call requires a `wiki` parameter specifying which wiki to operate on. Available tools:

- `get_next_item(wiki)` — claim next queued item
- `complete_item(wiki, id, wiki_page_path)` — mark item complete
- `read_source(wiki, id)` — read extracted text
- `read_page(wiki, path)` — read a wiki page
- `write_page(wiki, path, content)` — create/update a wiki page
- `list_pages(wiki, directory)` — list pages
- `read_index(wiki)` — read index.md
- `update_index(wiki, entry)` — append to index
- `append_log(wiki, entry)` — append to log

### Cross-Wiki Queries

With the MCP server, Claude can query across multiple wikis in a single session:
```
> Compare what the rust wiki says about async with what the python wiki says about asyncio
> Find all entities that appear in both the rust and ml wikis
```

## Utilities

### pdf-dump

Diagnostic tool for inspecting how a PDF will be split into chapters:

```bash
pdf-dump ~/Downloads/some-book.pdf
```

Shows the level-1 table of contents entries and the page ranges each chapter would produce.

## Project Structure

```
ai-wiki/
├── crates/
│   ├── ai-wiki-core/     # Library: config, queue, preprocessing, wiki
│   ├── ai-wiki/          # CLI: init, ingest, process, tui, retry, clear, list, queue
│   ├── ai-wiki-mcp/      # MCP server: multi-wiki, 12 tools
│   └── pdf-dump/         # PDF chapter inspection utility
├── docs/
│   ├── design/           # Design documents
│   └── superpowers/      # Specs, plans, reviews
├── justfile              # Task runner
└── README.md

~/.ai-wiki/
└── config.toml           # Central config with tool paths and wiki registry
```

Each wiki root:
```
~/wikis/rust/
├── wiki/                 # Obsidian vault
│   ├── entities/
│   ├── concepts/
│   ├── claims/
│   ├── sources/
│   ├── index.md
│   ├── log.md
│   └── CLAUDE.md
├── processed/            # Extracted text
├── raw/                  # Split PDFs
└── ai-wiki.db            # Queue database
```

## Development

```bash
just check      # Fast compile check
just test       # Run all tests
just lint       # Clippy lints
just ci         # Full CI pipeline
just deploy     # Install to ~/.cargo/bin
```

## License

MIT
