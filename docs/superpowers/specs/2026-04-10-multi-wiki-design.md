# Multi-Wiki Support — Design Specification

## Overview

Replace the per-project `ai-wiki.toml` config with a central `~/.ai-wiki/config.toml` that registers multiple named wikis. Each wiki is self-contained with its own database, processed text, and Obsidian vault.

## Config

**File:** `~/.ai-wiki/config.toml`

```toml
[tools]
qpdf_path = "qpdf"
pdftotext_path = "pdftotext"
pdftoppm_path = "pdftoppm"
tesseract_path = "tesseract"
ffmpeg_path = "ffmpeg"
whisper_cpp_path = "whisper-cpp"
whisper_model_path = "models/ggml-large-v3.bin"

[wikis.rust]
root = "/Users/hugh/wikis/rust"

[wikis.python]
root = "/Users/hugh/wikis/python"
```

### Per-wiki directory structure

Each wiki's `root` contains:
```
<root>/
├── wiki/          (Obsidian vault)
├── processed/     (extracted text)
├── raw/           (split PDFs, extracted ZIPs)
└── ai-wiki.db     (SQLite queue)
```

These paths are derived from `root`, not individually configurable.

### Removed config fields

- `book_min_pages` — any PDF with a TOC gets split, regardless of length
- `non_operative_extensions` — file type detection handles rejection
- `sensitive_filename_patterns` — removed entirely
- `RejectionConfig` struct — removed
- `PdfConfig` struct — removed
- Per-project `ai-wiki.toml` — replaced by central config

## Wiki Resolution

When a command needs a wiki:

1. If `--wiki <name>` is specified, use that wiki (user's explicit choice wins)
2. If no `--wiki`, check if CWD is at or under any registered wiki's `root`
3. If neither, print available wikis and exit with error

## CLI Changes

```
ai-wiki init [--name <name>] [<directory>]
ai-wiki list
ai-wiki ingest [--wiki <name>] <path>
ai-wiki process [--wiki <name>]
ai-wiki tui [--wiki <name>]
ai-wiki retry [--wiki <name>]
ai-wiki clear [--wiki <name>]
ai-wiki queue [--wiki <name>] {claim|complete|error}
```

The `--config` flag is removed.

### `ai-wiki init`
- Takes optional `--name` (defaults to directory name) and optional directory (defaults to CWD)
- Creates the directory structure under the target directory
- Registers the wiki in `~/.ai-wiki/config.toml` (creates the file if it doesn't exist)

### `ai-wiki list`
- Lists all registered wikis: name, root path, queue counts (queued/complete/error)

## MCP Server Changes

- Reads `~/.ai-wiki/config.toml` on startup
- Every tool requires a `wiki` parameter (the wiki name)
- The server resolves the name to the wiki's root, database, etc.
- Single server instance serves all wikis

## Migration from Per-Project Config

Existing setups that used a per-project `ai-wiki.toml` must be migrated manually:

1. **Create central config:** Run `ai-wiki init [--name <name>] [<directory>]` from the existing wiki root directory (or pass the directory explicitly). Name defaults to the directory name; directory defaults to CWD. This creates `~/.ai-wiki/config.toml` (if needed) and registers the wiki.
2. **Verify paths:** The `init` command derives sub-paths (`wiki/`, `processed/`, `raw/`, `ai-wiki.db`) from the registered root. Existing directories at those paths are reused — no data is moved or copied.
3. **Remove old config:** Delete the per-project `ai-wiki.toml` file. It is no longer read.
4. **Removed fields:** The following config fields no longer exist and have no equivalent:
   - `book_min_pages` — any PDF with a level-1 TOC entry is now split
   - `non_operative_extensions` — file type detection handles rejection
   - `sensitive_filename_patterns` — removed entirely
   - `PdfConfig` / `RejectionConfig` structs — removed

If an old-style `ai-wiki.toml` is found in the working directory, the CLI ignores it. No automatic discovery or import is performed.

## Overlapping and Nested Wiki Roots

When CWD-based wiki resolution finds multiple matching roots (e.g., `/wikis/rust` and `/wikis/rust/embedded`), the **most specific (longest) root wins**. This is a simple prefix-length comparison on canonicalized paths.

Symlinks: wiki roots are canonicalized (symlinks resolved) before comparison and before registration. Two roots that resolve to the same canonical path are rejected at `init` time as a duplicate.

Edge cases:
- A wiki root nested inside another wiki root is allowed but discouraged. The `--wiki` flag is the reliable way to disambiguate.
- If CWD matches exactly one root, that root is used regardless of nesting.

## Security: Process Trust Model

The `process` command grants Claude broad tool access via `--dangerously-skip-permissions`. This means Claude can read and write files within the wiki root without confirmation. Only process documents you trust. The process command validates that config paths contain only safe characters before embedding them in the Claude prompt.

## Implementation Notes

- The `Config` struct splits into `AppConfig` (global) containing `ToolsConfig` and a `HashMap<String, WikiConfig>`
- `WikiConfig` holds just `root: PathBuf` — all sub-paths derived via methods
- The old `PathsConfig` is replaced by `WikiConfig` methods: `wiki_dir()`, `processed_dir()`, `raw_dir()`, `database_path()`
- `classify_pdf` no longer checks `book_min_pages` — any PDF with level-1 TOC entries gets split
- `detect_file_type` no longer checks rejection patterns — unknown extensions return `Ingestable(Unknown)` which the ingest pipeline rejects
