# Workflow

The ai-wiki application has two phases: **ingest** (pure Rust preprocessing) and **process** (LLM-driven wiki building). No LLM is involved in the ingest phase.

## Phase 1: Ingest

The ingest phase reads source files, classifies them by type, extracts text, and queues them for LLM processing. The output of each file is a processed text file stored in the `processed/` directory, keyed by queue item ID.

### Responsibilities

- Read source files (file, directory, glob pattern, or `@filelist`)
- Classify each file by type (extension-based, with sensitive filename rejection)
- Extract text from each file (or transcode audio/video to text)
- Write extracted text to `processed/<id>.txt`
- Enqueue each file in SQLite with its type, status, and parent linkage
- Skip duplicate files (detected by file path + parent ID)

### File Types

**1. Markdown / Text** (`.md`, `.markdown`, `.txt`, `.text`)
Copied directly to the processed directory as-is. Queued with status `queued`.

**2. ZIP files**
Extracted to a temporary directory. Each contained file is processed recursively (up to depth 3). The ZIP parent item is enqueued; each child is enqueued with the ZIP as its parent.

**3. PDF files**
PDF processing uses a three-stage extraction pipeline:
1. `pdf-extract` (Rust crate) — embedded text extraction. Wrapped in `catch_unwind` because the upstream cff-parser crate can panic on malformed PDFs with CFF font encoding issues.
2. `pdftotext` (poppler CLI) — fallback for PDFs with unusual encodings or where pdf-extract returns empty text.
3. `pdftoppm` + `tesseract` — OCR fallback. Renders PDF pages to PPM images, then runs tesseract on each.

PDF classification (via `lopdf` + `get_toc()`):
- **Book**: has a table of contents (outline) with at least one level-1 entry AND at least `book_min_pages` pages (default: 50). Split into chapters using `qpdf`. Top-level outline entries only are used for splitting — sub-sections are ignored to avoid over-fragmentation.
- **Simple**: everything else. Extracted as a single document.
- **Rejected**: filename matches sensitive patterns (e.g., "divorce", "court", "financial", "tax.return") or extension is non-operative (e.g., `.dmg`).

Chapter splitting uses `qpdf` CLI to extract page ranges. Each chapter becomes a child queue item of the book parent.

**4. Audio** (`.mp3`, `.wav`, `.flac`, `.ogg`, `.m4a`)
Transcribed directly with `whisper-cpp`. The transcript is stored as the processed text.

**5. Video** (`.mp4`, `.mkv`, `.avi`, `.mov`, `.webm`)
Audio is first extracted with `ffmpeg` (16kHz mono WAV), then transcribed with `whisper-cpp`. The transcript is stored as the processed text. The intermediate WAV file is cleaned up after transcription.

**6. Non-operative files** (`.dmg` and others in `non_operative_extensions` config)
Rejected immediately and logged in the queue with status `rejected`.

**7. Unknown extensions**
Rejected with reason "unknown file type".

### Progress Display

Each file prints a status line:
```
[1/794] document.pdf ... queued (0.3s)
[2/794] installer.dmg ... rejected (0.0s)
[3/794] already-done.pdf ... skipped (0.0s)
```

A summary line is printed at the end:
```
Ingest complete — queued: 500, rejected: 12, errors: 3, skipped: 279, failed: 0 (4m 23s)
```

### Deduplication

Files are skipped if their path+parent combination already exists in the queue database (any status — queued, complete, error, etc.).

## Phase 2: Process (LLM)

The process phase invokes the Claude CLI to read extracted text and build wiki pages. The `process` command:

1. Counts queued items in the database.
2. Builds a prompt with instructions for Claude to process the queue using the `ai-wiki queue` subcommands.
3. Launches `claude --print --dangerously-skip-permissions` with the prompt on stdin.

### Queue Protocol (used by Claude)

Claude processes the queue by calling `ai-wiki queue` subcommands:

- `ai-wiki queue claim` — atomically claim the next queued item. Prints tab-delimited: `<ID>\t<file_path>\t<file_type>\t<parent_id_or_none>`. Prints `EMPTY` when the queue is exhausted.
- `ai-wiki queue complete <ID> <wiki_page_path>` — mark item complete with path to created wiki page.
- `ai-wiki queue error <ID> <message>` — mark item as error.

### Wiki Building

For each item, Claude:
1. Reads processed text from `processed/<id>.txt`
2. Extracts entities, concepts, and claims
3. Creates wiki pages with YAML frontmatter and `[[wikilinks]]`:
   - `sources/<slug>.md` — source summary
   - `entities/<slug>.md` — people, organizations, tools
   - `concepts/<slug>.md` — ideas, theories, techniques
   - `claims/<slug>.md` — specific assertions, with `data-point: true` for statistics
4. Updates `index.md` and appends to `log.md`
5. Marks the item complete

### Error Recovery

- `ai-wiki retry` — requeues errored items that have processed text available, then runs `process`.
- `ai-wiki clear` — deletes all errored items from the queue so they can be re-ingested.

## Wiki Format

Obsidian-native markdown:
- `[[wikilinks]]` for cross-references between pages
- YAML frontmatter on every page: `type`, `tags`, `sources`, `created`, `updated`
- Directory structure: `entities/`, `concepts/`, `claims/`, `sources/`
- `index.md` — catalog of all pages by category
- `log.md` — append-only chronological record with `## [YYYY-MM-DD] action | Title` entries
- `CLAUDE.md` — schema telling the LLM how to maintain the wiki

## MCP Server (Alternative Integration)

`ai-wiki-mcp` exposes the same queue and wiki operations as MCP tools for direct Claude Code integration, without the CLI subprocess approach. See the spec for tool details.
