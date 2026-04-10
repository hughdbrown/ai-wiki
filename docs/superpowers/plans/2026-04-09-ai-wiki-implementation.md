# AI Wiki Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust application that preprocesses source files and exposes an MCP server for Claude Code to drive LLM-powered wiki generation into an Obsidian vault.

**Architecture:** Cargo workspace with three crates: `ai-wiki-core` (library with queue, preprocessing, wiki, config modules), `ai-wiki` (CLI binary with ingest + TUI subcommands), and `ai-wiki-mcp` (MCP server binary). The library owns all domain logic; binaries are thin wrappers.

**Tech Stack:** Rust 2024 edition, clap (CLI), ratatui + crossterm (TUI), rusqlite (SQLite), lopdf (PDF inspection), pdf-extract (PDF text), rmcp (MCP server), serde + toml (config), zip (archives), glob (file patterns), chrono (timestamps). External tools: qpdf, pdftotext, tesseract, ffmpeg, whisper-cpp.

**Spec:** `docs/superpowers/specs/2026-04-09-ai-wiki-design.md`

**Rust workflow:** Follow `hdb-rust-developer` skill — batch writes before compiling, `cargo check` first, fix all errors in one pass, then `cargo test`, then `cargo clippy`.

---

## Chunk 1: Project Scaffolding and Config

### Task 1: Initialize Cargo Workspace

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/ai-wiki-core/Cargo.toml`
- Create: `crates/ai-wiki-core/src/lib.rs`
- Create: `crates/ai-wiki/Cargo.toml`
- Create: `crates/ai-wiki/src/main.rs`
- Create: `crates/ai-wiki-mcp/Cargo.toml`
- Create: `crates/ai-wiki-mcp/src/main.rs`
- Create: `.cargo/config.toml`
- Create: `.gitignore`

- [ ] **Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
members = ["crates/ai-wiki-core", "crates/ai-wiki", "crates/ai-wiki-mcp"]
resolver = "2"

[workspace.package]
edition = "2024"
version = "0.1.0"
license = "MIT"

[workspace.dependencies]
anyhow = "1.0"
chrono = { version = "0.4", features = ["serde"] }
clap = { version = "4.3", features = ["derive"] }
crossterm = "0.28"
glob = "0.3"
lopdf = "0.40"
pdf-extract = "0.10"
ratatui = "0.29"
rmcp = { version = "1", features = ["server", "transport-io", "macros"] }
rusqlite = { version = "0.32", features = ["bundled"] }
schemars = "1.0"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tokio = { version = "1.40", features = ["full"] }
toml = "0.8"
zip = "2"
```

- [ ] **Step 2: Create ai-wiki-core crate**

`crates/ai-wiki-core/Cargo.toml`:
```toml
[package]
name = "ai-wiki-core"
edition.workspace = true
version.workspace = true

[dependencies]
anyhow.workspace = true
chrono.workspace = true
glob.workspace = true
lopdf.workspace = true
pdf-extract.workspace = true
rusqlite.workspace = true
serde.workspace = true
thiserror.workspace = true
toml.workspace = true
zip.workspace = true
```

`crates/ai-wiki-core/src/lib.rs`:
```rust
pub mod config;
pub mod queue;
pub mod preprocessing;
pub mod wiki;
```

- [ ] **Step 3: Create ai-wiki CLI crate**

`crates/ai-wiki/Cargo.toml`:
```toml
[package]
name = "ai-wiki"
edition.workspace = true
version.workspace = true

[dependencies]
ai-wiki-core = { path = "../ai-wiki-core" }
anyhow.workspace = true
clap.workspace = true
crossterm.workspace = true
ratatui.workspace = true
```

`crates/ai-wiki/src/main.rs`:
```rust
fn main() {
    println!("ai-wiki");
}
```

- [ ] **Step 4: Create ai-wiki-mcp crate**

`crates/ai-wiki-mcp/Cargo.toml`:
```toml
[package]
name = "ai-wiki-mcp"
edition.workspace = true
version.workspace = true

[dependencies]
ai-wiki-core = { path = "../ai-wiki-core" }
anyhow.workspace = true
rmcp.workspace = true
schemars.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
```

`crates/ai-wiki-mcp/src/main.rs`:
```rust
fn main() {
    println!("ai-wiki-mcp");
}
```

- [ ] **Step 5: Create .cargo/config.toml for fast linking**

```toml
[target.aarch64-apple-darwin]
rustflags = ["-C", "link-arg=-fuse-ld=/opt/homebrew/bin/ld64.lld"]
```

- [ ] **Step 6: Create .gitignore**

```
/target
*.db
*.db-wal
*.db-shm
/processed/
```

- [ ] **Step 7: Run `cargo check` to verify workspace compiles**

```bash
cargo check 2>&1
```

Expected: succeeds (possibly with warnings about empty modules — that's fine).

- [ ] **Step 8: Initialize git repo and commit**

```bash
git init
git add Cargo.toml crates/ .cargo/ .gitignore docs/
git commit -m "feat: initialize cargo workspace with three crates"
```

### Task 2: Config Module

**Files:**
- Create: `crates/ai-wiki-core/src/config.rs`
- Test: inline `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing test**

In `crates/ai-wiki-core/src/config.rs`:

```rust
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub paths: PathsConfig,
    pub pdf: PdfConfig,
    pub rejection: RejectionConfig,
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    pub raw_dir: PathBuf,
    pub wiki_dir: PathBuf,
    pub database_path: PathBuf,
    pub processed_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfConfig {
    pub book_min_pages: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionConfig {
    pub non_operative_extensions: Vec<String>,
    pub sensitive_filename_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub qpdf_path: String,
    pub pdftotext_path: String,
    pub tesseract_path: String,
    pub ffmpeg_path: String,
    pub whisper_cpp_path: String,
    pub whisper_model_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            paths: PathsConfig {
                raw_dir: PathBuf::from("raw"),
                wiki_dir: PathBuf::from("wiki"),
                database_path: PathBuf::from("ai-wiki.db"),
                processed_dir: PathBuf::from("processed"),
            },
            pdf: PdfConfig {
                book_min_pages: 50,
            },
            rejection: RejectionConfig {
                non_operative_extensions: vec![
                    ".dmg".to_string(),
                ],
                sensitive_filename_patterns: vec![
                    "divorce".to_string(),
                    "court".to_string(),
                    "bank.statement".to_string(),
                    "tax.return".to_string(),
                    "report.card".to_string(),
                    "financial".to_string(),
                    "lease".to_string(),
                ],
            },
            tools: ToolsConfig {
                qpdf_path: "qpdf".to_string(),
                pdftotext_path: "pdftotext".to_string(),
                tesseract_path: "tesseract".to_string(),
                ffmpeg_path: "ffmpeg".to_string(),
                whisper_cpp_path: "whisper-cpp".to_string(),
                whisper_model_path: PathBuf::from("models/ggml-large-v3.bin"),
            },
        }
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config file {}: {}", path.display(), e))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse config file {}: {}", path.display(), e))?;
        Ok(config)
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("failed to serialize config: {}", e))?;
        std::fs::write(path, content)
            .map_err(|e| anyhow::anyhow!("failed to write config file {}: {}", path.display(), e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_default_config_round_trips() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.pdf.book_min_pages, 50);
        assert_eq!(deserialized.paths.raw_dir, PathBuf::from("raw"));
        assert_eq!(deserialized.rejection.non_operative_extensions, vec![".dmg"]);
    }

    #[test]
    fn test_load_config_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config::default();
        config.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.pdf.book_min_pages, config.pdf.book_min_pages);
        assert_eq!(loaded.paths.wiki_dir, config.paths.wiki_dir);
    }

    #[test]
    fn test_load_missing_file_returns_error() {
        let result = Config::load(std::path::Path::new("/nonexistent/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"this is not valid toml [[[").unwrap();

        let result = Config::load(&path);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Add tempfile dev-dependency**

Add to `crates/ai-wiki-core/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Run `cargo check --tests -p ai-wiki-core`**

Expected: compiles clean.

- [ ] **Step 4: Run tests**

```bash
cargo test -p ai-wiki-core -- --nocapture 2>&1
```

Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ai-wiki-core/src/config.rs crates/ai-wiki-core/Cargo.toml
git commit -m "feat: add config module with TOML serialization and defaults"
```

---

## Chunk 2: Queue Module

### Task 3: Queue Database Schema and Core Operations

**Files:**
- Create: `crates/ai-wiki-core/src/queue.rs`
- Test: inline `#[cfg(test)] mod tests`

- [ ] **Step 1: Write queue module with types and database operations**

In `crates/ai-wiki-core/src/queue.rs`:

```rust
use std::path::{Path, PathBuf};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemStatus {
    Queued,
    InProgress,
    Complete,
    Rejected,
    Error,
}

impl ItemStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::InProgress => "in_progress",
            Self::Complete => "complete",
            Self::Rejected => "rejected",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "in_progress" => Some(Self::InProgress),
            "complete" => Some(Self::Complete),
            "rejected" => Some(Self::Rejected),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileType {
    Markdown,
    Text,
    Pdf,
    Zip,
    Audio,
    Video,
    Unknown,
}

impl FileType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Pdf => "pdf",
            Self::Zip => "zip",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "markdown" => Self::Markdown,
            "text" => Self::Text,
            "pdf" => Self::Pdf,
            "zip" => Self::Zip,
            "audio" => Self::Audio,
            "video" => Self::Video,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueItem {
    pub id: i64,
    pub file_path: PathBuf,
    pub file_type: FileType,
    pub status: ItemStatus,
    pub parent_id: Option<i64>,
    pub wiki_page_path: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("item not found: {0}")]
    NotFound(i64),
    #[error("invalid status in database: {0}")]
    InvalidStatus(String),
}

pub struct Queue {
    conn: Connection,
}

impl Queue {
    pub fn open(db_path: &Path) -> Result<Self, QueueError> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA busy_timeout=5000;")?;
        let queue = Self { conn };
        queue.create_tables()?;
        Ok(queue)
    }

    pub fn open_in_memory() -> Result<Self, QueueError> {
        let conn = Connection::open_in_memory()?;
        let queue = Self { conn };
        queue.create_tables()?;
        Ok(queue)
    }

    fn create_tables(&self) -> Result<(), QueueError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS queue_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL,
                file_type TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                parent_id INTEGER REFERENCES queue_items(id),
                wiki_page_path TEXT,
                error_message TEXT,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_queue_status ON queue_items(status);
            CREATE INDEX IF NOT EXISTS idx_queue_parent ON queue_items(parent_id);"
        )?;
        Ok(())
    }

    pub fn enqueue(
        &self,
        file_path: &Path,
        file_type: FileType,
        parent_id: Option<i64>,
    ) -> Result<i64, QueueError> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO queue_items (file_path, file_type, status, parent_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file_path.to_string_lossy().as_ref(),
                file_type.as_str(),
                ItemStatus::Queued.as_str(),
                parent_id,
                now,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_item(&self, id: i64) -> Result<QueueItem, QueueError> {
        let item = self.conn.query_row(
            "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                    error_message, created_at, started_at, completed_at
             FROM queue_items WHERE id = ?1",
            params![id],
            |row| Self::row_to_item(row),
        )?;
        Ok(item)
    }

    pub fn get_next_queued(&self) -> Result<Option<QueueItem>, QueueError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                    error_message, created_at, started_at, completed_at
             FROM queue_items WHERE status = 'queued'
             ORDER BY id ASC LIMIT 1"
        )?;
        let item = stmt.query_row([], |row| Self::row_to_item(row)).optional()?;
        Ok(item)
    }

    pub fn mark_in_progress(&self, id: i64) -> Result<(), QueueError> {
        let now = Utc::now().to_rfc3339();
        let changed = self.conn.execute(
            "UPDATE queue_items SET status = 'in_progress', started_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(QueueError::NotFound(id));
        }
        Ok(())
    }

    pub fn mark_complete(&self, id: i64, wiki_page_path: &str) -> Result<(), QueueError> {
        let now = Utc::now().to_rfc3339();
        let changed = self.conn.execute(
            "UPDATE queue_items SET status = 'complete', wiki_page_path = ?1, completed_at = ?2
             WHERE id = ?3",
            params![wiki_page_path, now, id],
        )?;
        if changed == 0 {
            return Err(QueueError::NotFound(id));
        }
        Ok(())
    }

    pub fn mark_rejected(&self, id: i64, reason: &str) -> Result<(), QueueError> {
        let now = Utc::now().to_rfc3339();
        let changed = self.conn.execute(
            "UPDATE queue_items SET status = 'rejected', error_message = ?1, completed_at = ?2
             WHERE id = ?3",
            params![reason, now, id],
        )?;
        if changed == 0 {
            return Err(QueueError::NotFound(id));
        }
        Ok(())
    }

    pub fn mark_error(&self, id: i64, message: &str) -> Result<(), QueueError> {
        let now = Utc::now().to_rfc3339();
        let changed = self.conn.execute(
            "UPDATE queue_items SET status = 'error', error_message = ?1, completed_at = ?2
             WHERE id = ?3",
            params![message, now, id],
        )?;
        if changed == 0 {
            return Err(QueueError::NotFound(id));
        }
        Ok(())
    }

    pub fn list_items(&self, status_filter: Option<&ItemStatus>) -> Result<Vec<QueueItem>, QueueError> {
        let (sql, filter_params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match status_filter {
            Some(status) => (
                "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                        error_message, created_at, started_at, completed_at
                 FROM queue_items WHERE status = ?1 ORDER BY id ASC".to_string(),
                vec![Box::new(status.as_str().to_string())],
            ),
            None => (
                "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                        error_message, created_at, started_at, completed_at
                 FROM queue_items ORDER BY id ASC".to_string(),
                vec![],
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = filter_params.iter().map(|p| p.as_ref()).collect();
        let items = stmt.query_map(params_refs.as_slice(), |row| Self::row_to_item(row))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    pub fn reset_in_progress(&self) -> Result<usize, QueueError> {
        let changed = self.conn.execute(
            "UPDATE queue_items SET status = 'queued', started_at = NULL
             WHERE status = 'in_progress'",
            [],
        )?;
        Ok(changed)
    }

    pub fn count_by_status(&self) -> Result<Vec<(ItemStatus, usize)>, QueueError> {
        let mut stmt = self.conn.prepare(
            "SELECT status, COUNT(*) FROM queue_items GROUP BY status"
        )?;
        let counts = stmt.query_map([], |row| {
            let status_str: String = row.get(0)?;
            let count: usize = row.get(1)?;
            Ok((status_str, count))
        })?.collect::<Result<Vec<_>, _>>()?;

        let mut result = Vec::new();
        for (status_str, count) in counts {
            if let Some(status) = ItemStatus::parse(&status_str) {
                result.push((status, count));
            }
        }
        Ok(result)
    }

    pub fn children_of(&self, parent_id: i64) -> Result<Vec<QueueItem>, QueueError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                    error_message, created_at, started_at, completed_at
             FROM queue_items WHERE parent_id = ?1 ORDER BY id ASC"
        )?;
        let items = stmt.query_map(params![parent_id], |row| Self::row_to_item(row))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    pub fn all_children_complete(&self, parent_id: i64) -> Result<bool, QueueError> {
        let children = self.children_of(parent_id)?;
        if children.is_empty() {
            return Ok(false);
        }
        Ok(children.iter().all(|c| c.status == ItemStatus::Complete))
    }

    fn row_to_item(row: &rusqlite::Row) -> rusqlite::Result<QueueItem> {
        let status_str: String = row.get(3)?;
        let file_type_str: String = row.get(2)?;
        let file_path_str: String = row.get(1)?;
        let created_at_str: String = row.get(7)?;
        let started_at_str: Option<String> = row.get(8)?;
        let completed_at_str: Option<String> = row.get(9)?;

        let parse_dt = |s: &str| -> DateTime<Utc> {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now())
        };

        Ok(QueueItem {
            id: row.get(0)?,
            file_path: PathBuf::from(file_path_str),
            file_type: FileType::parse(&file_type_str),
            status: ItemStatus::parse(&status_str).unwrap_or(ItemStatus::Error),
            parent_id: row.get(4)?,
            wiki_page_path: row.get(5)?,
            error_message: row.get(6)?,
            created_at: parse_dt(&created_at_str),
            started_at: started_at_str.as_deref().map(parse_dt),
            completed_at: completed_at_str.as_deref().map(parse_dt),
        })
    }
}

// Make optional() available on query_row
use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_queue() -> Queue {
        Queue::open_in_memory().unwrap()
    }

    #[test]
    fn test_enqueue_and_get() {
        let q = test_queue();
        let id = q.enqueue(Path::new("/tmp/test.pdf"), FileType::Pdf, None).unwrap();
        let item = q.get_item(id).unwrap();
        assert_eq!(item.file_path, PathBuf::from("/tmp/test.pdf"));
        assert_eq!(item.file_type, FileType::Pdf);
        assert_eq!(item.status, ItemStatus::Queued);
        assert!(item.parent_id.is_none());
    }

    #[test]
    fn test_enqueue_with_parent() {
        let q = test_queue();
        let parent_id = q.enqueue(Path::new("/tmp/book.pdf"), FileType::Pdf, None).unwrap();
        let child_id = q.enqueue(Path::new("/tmp/ch1.pdf"), FileType::Pdf, Some(parent_id)).unwrap();
        let child = q.get_item(child_id).unwrap();
        assert_eq!(child.parent_id, Some(parent_id));
    }

    #[test]
    fn test_get_next_queued() {
        let q = test_queue();
        assert!(q.get_next_queued().unwrap().is_none());

        let id1 = q.enqueue(Path::new("/tmp/a.md"), FileType::Markdown, None).unwrap();
        let _id2 = q.enqueue(Path::new("/tmp/b.md"), FileType::Markdown, None).unwrap();

        let next = q.get_next_queued().unwrap().unwrap();
        assert_eq!(next.id, id1);
    }

    #[test]
    fn test_status_transitions() {
        let q = test_queue();
        let id = q.enqueue(Path::new("/tmp/test.md"), FileType::Markdown, None).unwrap();

        q.mark_in_progress(id).unwrap();
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::InProgress);
        assert!(item.started_at.is_some());

        q.mark_complete(id, "sources/test.md").unwrap();
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Complete);
        assert_eq!(item.wiki_page_path.as_deref(), Some("sources/test.md"));
        assert!(item.completed_at.is_some());
    }

    #[test]
    fn test_mark_rejected() {
        let q = test_queue();
        let id = q.enqueue(Path::new("/tmp/secret.pdf"), FileType::Pdf, None).unwrap();
        q.mark_rejected(id, "sensitive content").unwrap();
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Rejected);
        assert_eq!(item.error_message.as_deref(), Some("sensitive content"));
    }

    #[test]
    fn test_mark_error() {
        let q = test_queue();
        let id = q.enqueue(Path::new("/tmp/bad.zip"), FileType::Zip, None).unwrap();
        q.mark_error(id, "corrupted archive").unwrap();
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Error);
        assert_eq!(item.error_message.as_deref(), Some("corrupted archive"));
    }

    #[test]
    fn test_list_items_all() {
        let q = test_queue();
        q.enqueue(Path::new("/tmp/a.md"), FileType::Markdown, None).unwrap();
        q.enqueue(Path::new("/tmp/b.pdf"), FileType::Pdf, None).unwrap();
        let items = q.list_items(None).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_list_items_filtered() {
        let q = test_queue();
        let id = q.enqueue(Path::new("/tmp/a.md"), FileType::Markdown, None).unwrap();
        q.enqueue(Path::new("/tmp/b.pdf"), FileType::Pdf, None).unwrap();
        q.mark_in_progress(id).unwrap();

        let queued = q.list_items(Some(&ItemStatus::Queued)).unwrap();
        assert_eq!(queued.len(), 1);
        let in_progress = q.list_items(Some(&ItemStatus::InProgress)).unwrap();
        assert_eq!(in_progress.len(), 1);
    }

    #[test]
    fn test_reset_in_progress() {
        let q = test_queue();
        let id1 = q.enqueue(Path::new("/tmp/a.md"), FileType::Markdown, None).unwrap();
        let id2 = q.enqueue(Path::new("/tmp/b.md"), FileType::Markdown, None).unwrap();
        q.mark_in_progress(id1).unwrap();
        q.mark_in_progress(id2).unwrap();

        let reset = q.reset_in_progress().unwrap();
        assert_eq!(reset, 2);

        let item1 = q.get_item(id1).unwrap();
        assert_eq!(item1.status, ItemStatus::Queued);
        assert!(item1.started_at.is_none());
    }

    #[test]
    fn test_children_and_completion_check() {
        let q = test_queue();
        let parent = q.enqueue(Path::new("/tmp/book.pdf"), FileType::Pdf, None).unwrap();
        let ch1 = q.enqueue(Path::new("/tmp/ch1.pdf"), FileType::Pdf, Some(parent)).unwrap();
        let ch2 = q.enqueue(Path::new("/tmp/ch2.pdf"), FileType::Pdf, Some(parent)).unwrap();

        assert!(!q.all_children_complete(parent).unwrap());

        q.mark_in_progress(ch1).unwrap();
        q.mark_complete(ch1, "sources/ch1.md").unwrap();
        assert!(!q.all_children_complete(parent).unwrap());

        q.mark_in_progress(ch2).unwrap();
        q.mark_complete(ch2, "sources/ch2.md").unwrap();
        assert!(q.all_children_complete(parent).unwrap());
    }

    #[test]
    fn test_count_by_status() {
        let q = test_queue();
        q.enqueue(Path::new("/tmp/a.md"), FileType::Markdown, None).unwrap();
        q.enqueue(Path::new("/tmp/b.md"), FileType::Markdown, None).unwrap();
        let id3 = q.enqueue(Path::new("/tmp/c.md"), FileType::Markdown, None).unwrap();
        q.mark_in_progress(id3).unwrap();
        q.mark_complete(id3, "sources/c.md").unwrap();

        let counts = q.count_by_status().unwrap();
        let queued_count = counts.iter().find(|(s, _)| *s == ItemStatus::Queued).map(|(_, c)| *c).unwrap_or(0);
        let complete_count = counts.iter().find(|(s, _)| *s == ItemStatus::Complete).map(|(_, c)| *c).unwrap_or(0);
        assert_eq!(queued_count, 2);
        assert_eq!(complete_count, 1);
    }

    #[test]
    fn test_mark_nonexistent_item_returns_error() {
        let q = test_queue();
        assert!(q.mark_in_progress(999).is_err());
        assert!(q.mark_complete(999, "page.md").is_err());
        assert!(q.mark_rejected(999, "reason").is_err());
        assert!(q.mark_error(999, "error").is_err());
    }
}
```

- [ ] **Step 2: Run `cargo check --tests -p ai-wiki-core`**

Expected: compiles clean.

- [ ] **Step 3: Run tests**

```bash
cargo test -p ai-wiki-core -- --nocapture 2>&1
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ai-wiki-core/src/queue.rs
git commit -m "feat: add queue module with SQLite-backed job queue"
```

---

## Chunk 3: Wiki Module

### Task 4: Wiki File Operations

**Files:**
- Create: `crates/ai-wiki-core/src/wiki.rs`
- Test: inline `#[cfg(test)] mod tests`

- [ ] **Step 1: Write wiki module**

In `crates/ai-wiki-core/src/wiki.rs`:

```rust
use std::path::{Path, PathBuf};
use std::fs;
use chrono::Utc;

pub struct Wiki {
    root: PathBuf,
}

impl Wiki {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Initialize the wiki directory structure if it doesn't exist.
    pub fn init(&self) -> anyhow::Result<()> {
        let dirs = ["entities", "concepts", "claims", "sources"];
        for dir in &dirs {
            fs::create_dir_all(self.root.join(dir))?;
        }
        let index_path = self.root.join("index.md");
        if !index_path.exists() {
            fs::write(&index_path, Self::default_index())?;
        }
        let log_path = self.root.join("log.md");
        if !log_path.exists() {
            fs::write(&log_path, "# Wiki Log\n")?;
        }
        Ok(())
    }

    pub fn read_page(&self, relative_path: &str) -> anyhow::Result<String> {
        let path = self.root.join(relative_path);
        let content = fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("failed to read wiki page {}: {}", path.display(), e))?;
        Ok(content)
    }

    pub fn write_page(&self, relative_path: &str, content: &str) -> anyhow::Result<()> {
        let path = self.root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)
            .map_err(|e| anyhow::anyhow!("failed to write wiki page {}: {}", path.display(), e))?;
        Ok(())
    }

    pub fn list_pages(&self, subdirectory: Option<&str>) -> anyhow::Result<Vec<String>> {
        let dir = match subdirectory {
            Some(sub) => self.root.join(sub),
            None => self.root.clone(),
        };
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut pages = Vec::new();
        Self::collect_md_files(&dir, &self.root, &mut pages)?;
        pages.sort();
        Ok(pages)
    }

    pub fn read_index(&self) -> anyhow::Result<String> {
        self.read_page("index.md")
    }

    pub fn update_index(&self, entry: &str) -> anyhow::Result<()> {
        let path = self.root.join("index.md");
        let mut content = fs::read_to_string(&path).unwrap_or_else(|_| Self::default_index());
        content.push_str(entry);
        content.push('\n');
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn append_log(&self, entry: &str) -> anyhow::Result<()> {
        let path = self.root.join("log.md");
        let mut content = fs::read_to_string(&path).unwrap_or_else(|_| "# Wiki Log\n".to_string());
        let timestamp = Utc::now().format("%Y-%m-%d").to_string();
        content.push_str(&format!("\n## [{}] {}\n", timestamp, entry));
        fs::write(&path, content)?;
        Ok(())
    }

    fn default_index() -> String {
        "# Wiki Index\n\n## Entities\n\n## Concepts\n\n## Claims\n\n## Sources\n".to_string()
    }

    fn collect_md_files(dir: &Path, root: &Path, pages: &mut Vec<String>) -> anyhow::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::collect_md_files(&path, root, pages)?;
            } else if path.extension().is_some_and(|ext| ext == "md") {
                if let Ok(relative) = path.strip_prefix(root) {
                    pages.push(relative.to_string_lossy().to_string());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_wiki() -> (tempfile::TempDir, Wiki) {
        let dir = tempfile::tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();
        (dir, wiki)
    }

    #[test]
    fn test_init_creates_directories() {
        let (dir, _wiki) = test_wiki();
        assert!(dir.path().join("entities").is_dir());
        assert!(dir.path().join("concepts").is_dir());
        assert!(dir.path().join("claims").is_dir());
        assert!(dir.path().join("sources").is_dir());
        assert!(dir.path().join("index.md").is_file());
        assert!(dir.path().join("log.md").is_file());
    }

    #[test]
    fn test_write_and_read_page() {
        let (_dir, wiki) = test_wiki();
        wiki.write_page("entities/rust.md", "---\ntype: entity\n---\n# Rust\n").unwrap();
        let content = wiki.read_page("entities/rust.md").unwrap();
        assert!(content.contains("# Rust"));
    }

    #[test]
    fn test_read_nonexistent_page_returns_error() {
        let (_dir, wiki) = test_wiki();
        assert!(wiki.read_page("nonexistent.md").is_err());
    }

    #[test]
    fn test_list_pages() {
        let (_dir, wiki) = test_wiki();
        wiki.write_page("entities/rust.md", "# Rust").unwrap();
        wiki.write_page("entities/python.md", "# Python").unwrap();
        wiki.write_page("concepts/memory-safety.md", "# Memory Safety").unwrap();

        let all_pages = wiki.list_pages(None).unwrap();
        // Should include index.md, log.md, and the 3 pages we created
        assert!(all_pages.len() >= 5);

        let entity_pages = wiki.list_pages(Some("entities")).unwrap();
        assert_eq!(entity_pages.len(), 2);
    }

    #[test]
    fn test_list_pages_empty_subdirectory() {
        let (_dir, wiki) = test_wiki();
        let pages = wiki.list_pages(Some("nonexistent")).unwrap();
        assert!(pages.is_empty());
    }

    #[test]
    fn test_read_index() {
        let (_dir, wiki) = test_wiki();
        let index = wiki.read_index().unwrap();
        assert!(index.contains("# Wiki Index"));
        assert!(index.contains("## Entities"));
    }

    #[test]
    fn test_update_index() {
        let (_dir, wiki) = test_wiki();
        wiki.update_index("- [[entities/rust]] — The Rust programming language").unwrap();
        let index = wiki.read_index().unwrap();
        assert!(index.contains("[[entities/rust]]"));
    }

    #[test]
    fn test_append_log() {
        let (_dir, wiki) = test_wiki();
        wiki.append_log("ingest | test-file.pdf").unwrap();
        let log = wiki.read_page("log.md").unwrap();
        assert!(log.contains("ingest | test-file.pdf"));
        assert!(log.contains("## ["));
    }

    #[test]
    fn test_init_is_idempotent() {
        let (_dir, wiki) = test_wiki();
        wiki.write_page("entities/rust.md", "# Rust").unwrap();
        wiki.update_index("- [[entities/rust]]").unwrap();
        // Re-init should not clobber existing files
        wiki.init().unwrap();
        let index = wiki.read_index().unwrap();
        assert!(index.contains("[[entities/rust]]"));
        let content = wiki.read_page("entities/rust.md").unwrap();
        assert!(content.contains("# Rust"));
    }
}
```

- [ ] **Step 2: Run `cargo check --tests -p ai-wiki-core`**

- [ ] **Step 3: Run tests**

```bash
cargo test -p ai-wiki-core -- --nocapture 2>&1
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ai-wiki-core/src/wiki.rs
git commit -m "feat: add wiki module with Obsidian vault read/write operations"
```

---

## Chunk 4: Preprocessing Module

### Task 5: File Type Detection

**Files:**
- Create: `crates/ai-wiki-core/src/preprocessing.rs`
- Create: `crates/ai-wiki-core/src/preprocessing/detect.rs`
- Create: `crates/ai-wiki-core/src/preprocessing/pdf.rs`
- Create: `crates/ai-wiki-core/src/preprocessing/zip.rs`
- Create: `crates/ai-wiki-core/src/preprocessing/media.rs`

Since preprocessing is the largest module, split it into submodules. Convert `preprocessing.rs` to `preprocessing/mod.rs`.

- [ ] **Step 1: Create preprocessing module structure**

`crates/ai-wiki-core/src/preprocessing/mod.rs`:
```rust
pub mod detect;
pub mod pdf;
pub mod zip_extract;
pub mod media;

pub use detect::{detect_file_type, FileClassification};
pub use pdf::{classify_pdf, split_pdf_chapters, extract_pdf_text};
pub use zip_extract::extract_zip;
pub use media::{extract_audio, transcribe_audio};
```

- [ ] **Step 2: Write file type detection**

`crates/ai-wiki-core/src/preprocessing/detect.rs`:
```rust
use std::path::Path;
use crate::config::Config;
use crate::queue::FileType;

#[derive(Debug, PartialEq)]
pub enum FileClassification {
    Ingestable(FileType),
    Rejected(String),
}

pub fn detect_file_type(path: &Path, config: &Config) -> FileClassification {
    let extension = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();

    // Check non-operative extensions
    if config.rejection.non_operative_extensions.iter().any(|ext| ext == &extension) {
        return FileClassification::Rejected(
            format!("non-operative file type: {}", extension)
        );
    }

    // Check sensitive filename patterns
    let filename = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    for pattern in &config.rejection.sensitive_filename_patterns {
        if filename.contains(&pattern.to_lowercase()) {
            return FileClassification::Rejected(
                format!("sensitive content pattern: {}", pattern)
            );
        }
    }

    let file_type = match extension.as_str() {
        ".md" | ".markdown" => FileType::Markdown,
        ".txt" | ".text" => FileType::Text,
        ".pdf" => FileType::Pdf,
        ".zip" => FileType::Zip,
        ".mp3" | ".wav" | ".flac" | ".ogg" | ".m4a" => FileType::Audio,
        ".mp4" | ".mkv" | ".avi" | ".mov" | ".webm" => FileType::Video,
        _ => FileType::Unknown,
    };

    FileClassification::Ingestable(file_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::default()
    }

    #[test]
    fn test_detect_markdown() {
        let config = test_config();
        assert_eq!(
            detect_file_type(Path::new("notes.md"), &config),
            FileClassification::Ingestable(FileType::Markdown)
        );
    }

    #[test]
    fn test_detect_pdf() {
        let config = test_config();
        assert_eq!(
            detect_file_type(Path::new("paper.pdf"), &config),
            FileClassification::Ingestable(FileType::Pdf)
        );
    }

    #[test]
    fn test_detect_zip() {
        let config = test_config();
        assert_eq!(
            detect_file_type(Path::new("archive.zip"), &config),
            FileClassification::Ingestable(FileType::Zip)
        );
    }

    #[test]
    fn test_detect_video() {
        let config = test_config();
        assert_eq!(
            detect_file_type(Path::new("lecture.mp4"), &config),
            FileClassification::Ingestable(FileType::Video)
        );
    }

    #[test]
    fn test_reject_dmg() {
        let config = test_config();
        match detect_file_type(Path::new("installer.dmg"), &config) {
            FileClassification::Rejected(reason) => assert!(reason.contains("non-operative")),
            _ => panic!("expected rejection"),
        }
    }

    #[test]
    fn test_reject_sensitive_filename() {
        let config = test_config();
        match detect_file_type(Path::new("divorce-decree-2024.pdf"), &config) {
            FileClassification::Rejected(reason) => assert!(reason.contains("sensitive")),
            _ => panic!("expected rejection"),
        }
    }

    #[test]
    fn test_reject_financial() {
        let config = test_config();
        match detect_file_type(Path::new("financial-statement-q4.pdf"), &config) {
            FileClassification::Rejected(reason) => assert!(reason.contains("sensitive")),
            _ => panic!("expected rejection"),
        }
    }

    #[test]
    fn test_unknown_extension() {
        let config = test_config();
        assert_eq!(
            detect_file_type(Path::new("data.xyz"), &config),
            FileClassification::Ingestable(FileType::Unknown)
        );
    }
}
```

- [ ] **Step 3: Write PDF submodule stubs**

`crates/ai-wiki-core/src/preprocessing/pdf.rs`:
```rust
use std::path::Path;
use crate::config::Config;

#[derive(Debug, PartialEq)]
pub enum PdfClassification {
    Simple,
    Book { chapter_count: usize },
    Sensitive(String),
}

/// Classify a PDF by inspecting its structure.
/// Returns Simple, Book (with chapter count from bookmarks), or Sensitive.
pub fn classify_pdf(path: &Path, config: &Config) -> anyhow::Result<PdfClassification> {
    let doc = lopdf::Document::load(path)
        .map_err(|e| anyhow::anyhow!("failed to open PDF {}: {}", path.display(), e))?;

    let page_count = doc.get_pages().len() as u32;

    // Check for bookmarks/outlines
    let has_outlines = doc.catalog()
        .and_then(|catalog| catalog.get(b"Outlines").ok())
        .is_some();

    if has_outlines && page_count >= config.pdf.book_min_pages {
        // Count top-level bookmarks as chapters
        let chapter_count = count_top_level_bookmarks(&doc).unwrap_or(1);
        Ok(PdfClassification::Book { chapter_count })
    } else {
        Ok(PdfClassification::Simple)
    }
}

fn count_top_level_bookmarks(doc: &lopdf::Document) -> Option<usize> {
    let catalog = doc.catalog().ok()?;
    let outlines_ref = catalog.get(b"Outlines").ok()?;
    let outlines = doc.get_object(outlines_ref.clone()).ok()?;
    if let lopdf::Object::Dictionary(dict) = outlines {
        if let Ok(count) = dict.get(b"Count") {
            if let lopdf::Object::Integer(n) = count {
                return Some(n.unsigned_abs() as usize);
            }
        }
    }
    None
}

/// Split a PDF into chapters using qpdf. Returns paths to the split chapter files.
pub fn split_pdf_chapters(
    path: &Path,
    output_dir: &Path,
    config: &Config,
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    // Extract bookmark page ranges from the PDF
    let doc = lopdf::Document::load(path)
        .map_err(|e| anyhow::anyhow!("failed to open PDF {}: {}", path.display(), e))?;

    let total_pages = doc.get_pages().len();
    let bookmark_pages = extract_bookmark_page_ranges(&doc, total_pages);

    if bookmark_pages.is_empty() {
        // No bookmarks found — return the original file as the single "chapter"
        return Ok(vec![path.to_path_buf()]);
    }

    std::fs::create_dir_all(output_dir)?;

    let stem = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("book");

    let mut chapter_paths = Vec::new();
    for (i, (start, end)) in bookmark_pages.iter().enumerate() {
        let chapter_path = output_dir.join(format!("{}-chapter-{:03}.pdf", stem, i + 1));
        let output = std::process::Command::new(&config.tools.qpdf_path)
            .args([
                path.to_str().unwrap_or_default(),
                "--pages", ".", &format!("{}-{}", start, end), "--",
                chapter_path.to_str().unwrap_or_default(),
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("failed to run qpdf: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("qpdf failed: {}", stderr));
        }
        chapter_paths.push(chapter_path);
    }

    Ok(chapter_paths)
}

fn extract_bookmark_page_ranges(doc: &lopdf::Document, total_pages: usize) -> Vec<(usize, usize)> {
    // Simplified: extract top-level bookmark destinations as page ranges
    // Each bookmark starts at its destination page; the range extends to the next bookmark's page
    let mut starts: Vec<usize> = Vec::new();

    if let Ok(catalog) = doc.catalog() {
        if let Ok(outlines_ref) = catalog.get(b"Outlines") {
            if let Ok(outlines) = doc.get_object(outlines_ref.clone()) {
                if let lopdf::Object::Dictionary(dict) = outlines {
                    if let Ok(first_ref) = dict.get(b"First") {
                        collect_bookmark_pages(doc, first_ref, &mut starts);
                    }
                }
            }
        }
    }

    starts.sort();
    starts.dedup();

    if starts.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    for i in 0..starts.len() {
        let start = starts[i];
        let end = if i + 1 < starts.len() {
            starts[i + 1] - 1
        } else {
            total_pages
        };
        if start <= end {
            ranges.push((start, end));
        }
    }
    ranges
}

fn collect_bookmark_pages(doc: &lopdf::Document, obj_ref: &lopdf::Object, pages: &mut Vec<usize>) {
    let obj = match doc.get_object(obj_ref.clone()) {
        Ok(o) => o,
        Err(_) => return,
    };

    if let lopdf::Object::Dictionary(dict) = obj {
        // Try to get destination page
        if let Ok(dest) = dict.get(b"Dest") {
            if let lopdf::Object::Array(arr) = dest {
                if let Some(lopdf::Object::Reference(page_ref)) = arr.first() {
                    if let Some(page_num) = doc.get_pages().iter()
                        .find(|(_, &r)| r == *page_ref)
                        .map(|(num, _)| *num as usize)
                    {
                        pages.push(page_num);
                    }
                }
            }
        }

        // Follow the linked list: Next sibling
        if let Ok(next_ref) = dict.get(b"Next") {
            collect_bookmark_pages(doc, next_ref, pages);
        }
    }
}

/// Extract text from a PDF. Tries pdf-extract first, falls back to pdftotext, then tesseract.
pub fn extract_pdf_text(path: &Path, config: &Config) -> anyhow::Result<String> {
    // Try pdf-extract crate first
    match pdf_extract::extract_text(path) {
        Ok(text) if !text.trim().is_empty() => return Ok(text),
        _ => {}
    }

    // Fallback: pdftotext (poppler)
    let output = std::process::Command::new(&config.tools.pdftotext_path)
        .args([path.to_str().unwrap_or_default(), "-"])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if !text.trim().is_empty() {
                return Ok(text);
            }
        }
    }

    // Fallback: tesseract OCR
    let output = std::process::Command::new(&config.tools.tesseract_path)
        .args([path.to_str().unwrap_or_default(), "stdout", "pdf"])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run tesseract: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(anyhow::anyhow!("all PDF text extraction methods failed for {}", path.display()))
    }
}
```

- [ ] **Step 4: Write ZIP submodule**

`crates/ai-wiki-core/src/preprocessing/zip_extract.rs`:
```rust
use std::path::{Path, PathBuf};
use std::fs;

/// Extract all files from a ZIP archive to the output directory.
/// Returns the paths of extracted files.
pub fn extract_zip(zip_path: &Path, output_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let file = fs::File::open(zip_path)
        .map_err(|e| anyhow::anyhow!("failed to open ZIP {}: {}", zip_path.display(), e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| anyhow::anyhow!("failed to read ZIP {}: {}", zip_path.display(), e))?;

    fs::create_dir_all(output_dir)?;

    let mut extracted = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let entry_name = entry.name().to_string();

        // Skip directories
        if entry.is_dir() {
            continue;
        }

        // Sanitize path to prevent zip-slip
        let out_path = output_dir.join(
            Path::new(&entry_name)
                .file_name()
                .unwrap_or_default()
        );

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut outfile = fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut outfile)?;
        extracted.push(out_path);
    }

    Ok(extracted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn create_test_zip(dir: &Path) -> PathBuf {
        let zip_path = dir.join("test.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut zip_writer = zip::ZipWriter::new(file);

        let options = zip::write::SimpleFileOptions::default();
        zip_writer.start_file("hello.txt", options).unwrap();
        zip_writer.write_all(b"hello world").unwrap();
        zip_writer.start_file("notes.md", options).unwrap();
        zip_writer.write_all(b"# Notes\nSome content").unwrap();
        zip_writer.finish().unwrap();

        zip_path
    }

    #[test]
    fn test_extract_zip() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = create_test_zip(dir.path());
        let output_dir = dir.path().join("extracted");

        let files = extract_zip(&zip_path, &output_dir).unwrap();
        assert_eq!(files.len(), 2);
        assert!(output_dir.join("hello.txt").exists());
        assert!(output_dir.join("notes.md").exists());

        let content = fs::read_to_string(output_dir.join("hello.txt")).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_extract_nonexistent_zip_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = extract_zip(Path::new("/nonexistent.zip"), dir.path());
        assert!(result.is_err());
    }
}
```

- [ ] **Step 5: Write media submodule stubs**

`crates/ai-wiki-core/src/preprocessing/media.rs`:
```rust
use std::path::{Path, PathBuf};
use crate::config::Config;

/// Extract audio from a video file using ffmpeg.
/// Returns the path to the extracted WAV file.
pub fn extract_audio(video_path: &Path, output_dir: &Path, config: &Config) -> anyhow::Result<PathBuf> {
    let stem = video_path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let audio_path = output_dir.join(format!("{}.wav", stem));

    std::fs::create_dir_all(output_dir)?;

    let output = std::process::Command::new(&config.tools.ffmpeg_path)
        .args([
            "-i", video_path.to_str().unwrap_or_default(),
            "-vn", "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1",
            "-y",
            audio_path.to_str().unwrap_or_default(),
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run ffmpeg: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("ffmpeg failed: {}", stderr));
    }

    Ok(audio_path)
}

/// Transcribe an audio file using whisper-cpp.
/// Returns the transcribed text.
pub fn transcribe_audio(audio_path: &Path, config: &Config) -> anyhow::Result<String> {
    let output = std::process::Command::new(&config.tools.whisper_cpp_path)
        .args([
            "-m", config.tools.whisper_model_path.to_str().unwrap_or_default(),
            "-f", audio_path.to_str().unwrap_or_default(),
            "--output-txt",
            "--no-timestamps",
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run whisper-cpp: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("whisper-cpp failed: {}", stderr));
    }

    // whisper-cpp with --output-txt writes to <input>.txt
    let txt_path = audio_path.with_extension("wav.txt");
    if txt_path.exists() {
        let text = std::fs::read_to_string(&txt_path)?;
        std::fs::remove_file(&txt_path)?; // clean up
        Ok(text)
    } else {
        // Some versions output to stdout
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
```

- [ ] **Step 6: Update lib.rs module declaration**

Change `crates/ai-wiki-core/src/lib.rs` — replace the single `pub mod preprocessing;` line. The module is now a directory (`preprocessing/mod.rs`), but the declaration stays the same.

- [ ] **Step 7: Run `cargo check --tests -p ai-wiki-core`**

Fix any compilation errors in a single batch.

- [ ] **Step 8: Run tests**

```bash
cargo test -p ai-wiki-core -- --nocapture 2>&1
```

Expected: all tests pass (note: PDF and media tests require external files/tools, so only the zip and detect tests run here).

- [ ] **Step 9: Run clippy**

```bash
cargo clippy -p ai-wiki-core 2>&1
```

Fix any warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/ai-wiki-core/src/preprocessing/
git commit -m "feat: add preprocessing module with file detection, PDF, ZIP, and media support"
```

---

## Chunk 5: CLI Binary

### Task 6: CLI Ingest Command

**Files:**
- Modify: `crates/ai-wiki/src/main.rs`
- Create: `crates/ai-wiki/src/ingest.rs`

- [ ] **Step 1: Write CLI argument parsing and ingest command**

`crates/ai-wiki/src/main.rs`:
```rust
mod ingest;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ai-wiki", version, about = "AI-powered wiki builder")]
struct Cli {
    /// Path to config file
    #[arg(long, default_value = "ai-wiki.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Ingest source files into the processing queue
    Ingest {
        /// File, glob pattern, or directory to ingest
        path: String,
    },
    /// Launch the TUI to monitor queue status
    Tui,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = if cli.config.exists() {
        ai_wiki_core::config::Config::load(&cli.config)?
    } else {
        let config = ai_wiki_core::config::Config::default();
        config.save(&cli.config)?;
        eprintln!("Created default config at {}", cli.config.display());
        config
    };

    match cli.command {
        Commands::Ingest { path } => ingest::run(&config, &path),
        Commands::Tui => {
            eprintln!("TUI not yet implemented");
            Ok(())
        }
    }
}
```

`crates/ai-wiki/src/ingest.rs`:
```rust
use std::path::{Path, PathBuf};
use ai_wiki_core::config::Config;
use ai_wiki_core::preprocessing::detect::{detect_file_type, FileClassification};
use ai_wiki_core::preprocessing::{extract_zip, split_pdf_chapters};
use ai_wiki_core::preprocessing::pdf::classify_pdf;
use ai_wiki_core::queue::{Queue, FileType};

pub fn run(config: &Config, path_str: &str) -> anyhow::Result<()> {
    let queue = Queue::open(&config.paths.database_path)?;
    let reset_count = queue.reset_in_progress()?;
    if reset_count > 0 {
        eprintln!("Reset {} in-progress items to queued", reset_count);
    }

    // Initialize wiki directory structure
    let wiki = ai_wiki_core::wiki::Wiki::new(config.paths.wiki_dir.clone());
    wiki.init()?;

    // Resolve files from path (file, glob, or directory)
    let files = resolve_files(path_str)?;

    let mut queued = 0usize;
    let mut rejected = 0usize;
    let mut errors = 0usize;

    for file in &files {
        match process_file(file, config, &queue, None) {
            Ok(result) => {
                queued += result.queued;
                rejected += result.rejected;
                errors += result.errors;
            }
            Err(e) => {
                eprintln!("Error processing {}: {}", file.display(), e);
                errors += 1;
            }
        }
    }

    println!("Ingest complete: {} queued, {} rejected, {} errors", queued, rejected, errors);
    Ok(())
}

struct IngestResult {
    queued: usize,
    rejected: usize,
    errors: usize,
}

fn process_file(
    path: &Path,
    config: &Config,
    queue: &Queue,
    parent_id: Option<i64>,
) -> anyhow::Result<IngestResult> {
    let mut result = IngestResult { queued: 0, rejected: 0, errors: 0 };

    match detect_file_type(path, config) {
        FileClassification::Rejected(reason) => {
            let id = queue.enqueue(path, FileType::Unknown, parent_id)?;
            queue.mark_rejected(id, &reason)?;
            eprintln!("Rejected: {} ({})", path.display(), reason);
            result.rejected += 1;
        }
        FileClassification::Ingestable(file_type) => {
            match file_type {
                FileType::Zip => {
                    let zip_id = queue.enqueue(path, FileType::Zip, parent_id)?;
                    let extract_dir = config.paths.processed_dir.join(format!("zip-{}", zip_id));
                    match extract_zip(path, &extract_dir) {
                        Ok(extracted_files) => {
                            for extracted_file in &extracted_files {
                                match process_file(extracted_file, config, queue, Some(zip_id)) {
                                    Ok(sub) => {
                                        result.queued += sub.queued;
                                        result.rejected += sub.rejected;
                                        result.errors += sub.errors;
                                    }
                                    Err(e) => {
                                        eprintln!("Error processing extracted file {}: {}", extracted_file.display(), e);
                                        result.errors += 1;
                                    }
                                }
                            }
                            result.queued += 1; // count the ZIP itself
                        }
                        Err(e) => {
                            queue.mark_error(zip_id, &e.to_string())?;
                            eprintln!("Error extracting ZIP {}: {}", path.display(), e);
                            result.errors += 1;
                        }
                    }
                }
                FileType::Pdf => {
                    match classify_pdf(path, config) {
                        Ok(ai_wiki_core::preprocessing::pdf::PdfClassification::Book { .. }) => {
                            let book_id = queue.enqueue(path, FileType::Pdf, parent_id)?;
                            let split_dir = config.paths.processed_dir.join(format!("book-{}", book_id));
                            match split_pdf_chapters(path, &split_dir, config) {
                                Ok(chapters) => {
                                    for chapter in &chapters {
                                        let chapter_id = queue.enqueue(chapter, FileType::Pdf, Some(book_id))?;
                                        result.queued += 1;
                                        // Extract text for each chapter
                                        if let Err(e) = extract_and_store_text(chapter, chapter_id, config) {
                                            queue.mark_error(chapter_id, &e.to_string())?;
                                            result.errors += 1;
                                            result.queued -= 1;
                                        }
                                    }
                                    result.queued += 1; // the book parent
                                }
                                Err(e) => {
                                    queue.mark_error(book_id, &e.to_string())?;
                                    eprintln!("Error splitting PDF {}: {}", path.display(), e);
                                    result.errors += 1;
                                }
                            }
                        }
                        Ok(ai_wiki_core::preprocessing::pdf::PdfClassification::Sensitive(reason)) => {
                            let id = queue.enqueue(path, FileType::Pdf, parent_id)?;
                            queue.mark_rejected(id, &reason)?;
                            result.rejected += 1;
                        }
                        Ok(ai_wiki_core::preprocessing::pdf::PdfClassification::Simple) => {
                            let id = queue.enqueue(path, FileType::Pdf, parent_id)?;
                            if let Err(e) = extract_and_store_text(path, id, config) {
                                queue.mark_error(id, &e.to_string())?;
                                result.errors += 1;
                            } else {
                                result.queued += 1;
                            }
                        }
                        Err(e) => {
                            let id = queue.enqueue(path, FileType::Pdf, parent_id)?;
                            queue.mark_error(id, &e.to_string())?;
                            eprintln!("Error classifying PDF {}: {}", path.display(), e);
                            result.errors += 1;
                        }
                    }
                }
                FileType::Markdown | FileType::Text => {
                    let id = queue.enqueue(path, file_type, parent_id)?;
                    // Copy source text to processed directory
                    let processed_path = config.paths.processed_dir.join(format!("{}.txt", id));
                    std::fs::create_dir_all(&config.paths.processed_dir)?;
                    std::fs::copy(path, &processed_path)?;
                    result.queued += 1;
                }
                FileType::Audio | FileType::Video => {
                    let id = queue.enqueue(path, file_type, parent_id)?;
                    // Audio/video transcription: extract audio if video, then transcribe
                    match transcribe_source(path, id, &file_type, config) {
                        Ok(_) => result.queued += 1,
                        Err(e) => {
                            queue.mark_error(id, &e.to_string())?;
                            eprintln!("Error transcribing {}: {}", path.display(), e);
                            result.errors += 1;
                        }
                    }
                }
                FileType::Unknown => {
                    let id = queue.enqueue(path, FileType::Unknown, parent_id)?;
                    queue.mark_rejected(id, "unknown file type")?;
                    result.rejected += 1;
                }
            }
        }
    }

    Ok(result)
}

fn extract_and_store_text(path: &Path, item_id: i64, config: &Config) -> anyhow::Result<()> {
    let text = ai_wiki_core::preprocessing::extract_pdf_text(path, config)?;
    let processed_path = config.paths.processed_dir.join(format!("{}.txt", item_id));
    std::fs::create_dir_all(&config.paths.processed_dir)?;
    std::fs::write(&processed_path, &text)?;
    Ok(())
}

fn transcribe_source(path: &Path, item_id: i64, file_type: &FileType, config: &Config) -> anyhow::Result<()> {
    let audio_path = if *file_type == FileType::Video {
        let extract_dir = config.paths.processed_dir.join(format!("audio-{}", item_id));
        ai_wiki_core::preprocessing::extract_audio(path, &extract_dir, config)?
    } else {
        path.to_path_buf()
    };

    let text = ai_wiki_core::preprocessing::transcribe_audio(&audio_path, config)?;
    let processed_path = config.paths.processed_dir.join(format!("{}.txt", item_id));
    std::fs::create_dir_all(&config.paths.processed_dir)?;
    std::fs::write(&processed_path, &text)?;
    Ok(())
}

fn resolve_files(path_str: &str) -> anyhow::Result<Vec<PathBuf>> {
    let path = Path::new(path_str);

    // If it's a directory, walk it
    if path.is_dir() {
        let mut files = Vec::new();
        walk_dir(path, &mut files)?;
        return Ok(files);
    }

    // If it's a file, return it
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    // Try as a glob pattern
    let entries: Vec<PathBuf> = glob::glob(path_str)
        .map_err(|e| anyhow::anyhow!("invalid glob pattern: {}", e))?
        .filter_map(|entry| entry.ok())
        .filter(|p| p.is_file())
        .collect();

    if entries.is_empty() {
        anyhow::bail!("no files matched: {}", path_str);
    }

    Ok(entries)
}

fn walk_dir(dir: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Add unit tests for resolve_files**

Add to the bottom of `crates/ai-wiki/src/ingest.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.md");
        fs::write(&file_path, "# Test").unwrap();

        let files = resolve_files(file_path.to_str().unwrap()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], file_path);
    }

    #[test]
    fn test_resolve_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "# A").unwrap();
        fs::write(dir.path().join("b.txt"), "B").unwrap();

        let files = resolve_files(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_resolve_glob() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "# A").unwrap();
        fs::write(dir.path().join("b.md"), "# B").unwrap();
        fs::write(dir.path().join("c.txt"), "C").unwrap();

        let pattern = format!("{}/*.md", dir.path().display());
        let files = resolve_files(&pattern).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_resolve_no_match_returns_error() {
        let result = resolve_files("/nonexistent/path/*.xyz");
        assert!(result.is_err());
    }
}
```

Add `tempfile = "3"` to `[dev-dependencies]` in `crates/ai-wiki/Cargo.toml`.

- [ ] **Step 3: Run `cargo check --tests -p ai-wiki`**

Fix all compilation errors in a single batch.

- [ ] **Step 4: Run tests**

```bash
cargo test -p ai-wiki -- --nocapture 2>&1
```

Expected: all resolve_files tests pass.

- [ ] **Step 5: Test the ingest command manually**

```bash
cargo run -p ai-wiki -- ingest docs/
```

Expected: ingests the markdown files from docs/, creates the queue database, prints summary.

- [ ] **Step 6: Commit**

```bash
git add crates/ai-wiki/src/ crates/ai-wiki/Cargo.toml
git commit -m "feat: add CLI ingest command with file processing pipeline"
```

### Task 7: TUI Monitor

**Files:**
- Create: `crates/ai-wiki/src/tui.rs`
- Modify: `crates/ai-wiki/src/main.rs`

- [ ] **Step 1: Write TUI implementation**

`crates/ai-wiki/src/tui.rs`:
```rust
use std::io;
use std::time::{Duration, Instant};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::prelude::*;
use ratatui::widgets::*;
use ai_wiki_core::config::Config;
use ai_wiki_core::queue::{Queue, QueueItem, ItemStatus};

pub fn run(config: &Config) -> anyhow::Result<()> {
    let queue = Queue::open(&config.paths.database_path)?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let result = run_app(&mut terminal, &queue);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    queue: &Queue,
) -> anyhow::Result<()> {
    let mut last_refresh = Instant::now();
    let refresh_interval = Duration::from_secs(2);
    let mut items: Vec<QueueItem> = Vec::new();
    let mut table_state = TableState::default();

    loop {
        // Refresh data periodically
        if last_refresh.elapsed() >= refresh_interval || items.is_empty() {
            items = queue.list_items(None)?;
            last_refresh = Instant::now();
        }

        terminal.draw(|frame| {
            let area = frame.area();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // status bar
                    Constraint::Min(0),    // table
                    Constraint::Length(1), // help
                ])
                .split(area);

            // Status bar with counts
            let counts = queue.count_by_status().unwrap_or_default();
            let count_text = format_counts(&counts);
            let status_block = Block::default().borders(Borders::ALL).title("Queue Status");
            let status = Paragraph::new(count_text).block(status_block);
            frame.render_widget(status, chunks[0]);

            // Queue table
            let header = Row::new(vec!["ID", "File", "Type", "Status", "Started", "Parent", "Wiki Page"])
                .style(Style::default().add_modifier(Modifier::BOLD))
                .bottom_margin(1);

            let rows: Vec<Row> = items.iter().map(|item| {
                let status_style = match item.status {
                    ItemStatus::Queued => Style::default().fg(Color::DarkGray),
                    ItemStatus::InProgress => Style::default().fg(Color::Yellow),
                    ItemStatus::Complete => Style::default().fg(Color::Green),
                    ItemStatus::Rejected => Style::default().fg(Color::Red),
                    ItemStatus::Error => Style::default().fg(Color::Red),
                };

                let filename = item.file_path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();

                let started = item.started_at
                    .map(|dt| dt.format("%H:%M:%S").to_string())
                    .unwrap_or_default();

                let parent = item.parent_id
                    .map(|id| id.to_string())
                    .unwrap_or_default();

                let wiki_page = item.wiki_page_path.as_deref().unwrap_or("").to_string();

                Row::new(vec![
                    item.id.to_string(),
                    filename,
                    item.file_type.as_str().to_string(),
                    item.status.as_str().to_string(),
                    started,
                    parent,
                    wiki_page,
                ]).style(status_style)
            }).collect();

            let widths = [
                Constraint::Length(6),
                Constraint::Min(20),
                Constraint::Length(10),
                Constraint::Length(12),
                Constraint::Length(10),
                Constraint::Length(8),
                Constraint::Min(15),
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .block(Block::default().borders(Borders::ALL).title("Queue Items"))
                .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            frame.render_stateful_widget(table, chunks[1], &mut table_state);

            // Help line
            let help = Paragraph::new(" q: quit | ↑↓: scroll | r: refresh")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(help, chunks[2]);
        })?;

        // Handle input
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('r') => {
                            items = queue.list_items(None)?;
                            last_refresh = Instant::now();
                        }
                        KeyCode::Down => {
                            let i = table_state.selected().unwrap_or(0);
                            if i < items.len().saturating_sub(1) {
                                table_state.select(Some(i + 1));
                            }
                        }
                        KeyCode::Up => {
                            let i = table_state.selected().unwrap_or(0);
                            if i > 0 {
                                table_state.select(Some(i - 1));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn format_counts(counts: &[(ItemStatus, usize)]) -> String {
    let get = |status: &ItemStatus| -> usize {
        counts.iter().find(|(s, _)| s == status).map(|(_, c)| *c).unwrap_or(0)
    };

    format!(
        "Queued: {} | In Progress: {} | Complete: {} | Rejected: {} | Error: {}",
        get(&ItemStatus::Queued),
        get(&ItemStatus::InProgress),
        get(&ItemStatus::Complete),
        get(&ItemStatus::Rejected),
        get(&ItemStatus::Error),
    )
}
```

- [ ] **Step 2: Update main.rs to wire in TUI**

In `crates/ai-wiki/src/main.rs`, add `mod tui;` and change the `Tui` match arm:
```rust
Commands::Tui => tui::run(&config),
```

- [ ] **Step 3: Run `cargo check -p ai-wiki`**

- [ ] **Step 4: Test manually**

```bash
cargo run -p ai-wiki -- tui
```

Expected: TUI launches, shows queue items (if any from previous ingest), responds to keyboard input.

- [ ] **Step 5: Commit**

```bash
git add crates/ai-wiki/src/tui.rs crates/ai-wiki/src/main.rs
git commit -m "feat: add TUI monitor for queue status"
```

---

## Chunk 6: MCP Server

### Task 8: MCP Server Implementation

**Files:**
- Modify: `crates/ai-wiki-mcp/src/main.rs`

**IMPORTANT NOTE:** The `rmcp` crate API uses `#[tool_router]` for the tool impl block, `Parameters<T>` wrapper structs with `schemars::JsonSchema` for tool parameters, and `rmcp::transport::stdio` for the transport. The implementer MUST verify the exact API against `rmcp` 1.x docs/examples at build time, as macro-based APIs are fragile across minor versions. The code below reflects the best-known API as of writing; adjust if `cargo check` reveals differences.

- [ ] **Step 1: Write parameter structs and MCP server**

`crates/ai-wiki-mcp/src/main.rs`:
```rust
use std::path::PathBuf;
use std::sync::Mutex;
use anyhow::Result;
use rmcp::{ServerHandler, ServiceExt, tool, tool_router, transport::stdio};
use rmcp::model::*;
use rmcp::handler::server::wrapper::Parameters;
use schemars::JsonSchema;
use serde::Deserialize;
use ai_wiki_core::config::Config;
use ai_wiki_core::queue::{Queue, ItemStatus};
use ai_wiki_core::wiki::Wiki;

// --- Parameter structs for tools that take arguments ---

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompleteItemRequest {
    #[schemars(description = "Queue item ID")]
    pub id: i64,
    #[schemars(description = "Relative path to the wiki page created for this source")]
    pub wiki_page_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RejectItemRequest {
    #[schemars(description = "Queue item ID")]
    pub id: i64,
    #[schemars(description = "Reason for rejection")]
    pub reason: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ErrorItemRequest {
    #[schemars(description = "Queue item ID")]
    pub id: i64,
    #[schemars(description = "Error message")]
    pub message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListItemsRequest {
    #[schemars(description = "Optional status filter: queued, in_progress, complete, rejected, error")]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadSourceRequest {
    #[schemars(description = "Queue item ID")]
    pub id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadPageRequest {
    #[schemars(description = "Relative path to the wiki page (e.g. 'entities/rust.md')")]
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WritePageRequest {
    #[schemars(description = "Relative path for the wiki page (e.g. 'entities/rust.md')")]
    pub path: String,
    #[schemars(description = "Full markdown content including YAML frontmatter")]
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPagesRequest {
    #[schemars(description = "Optional subdirectory to filter by (entities, concepts, claims, sources)")]
    pub directory: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateIndexRequest {
    #[schemars(description = "Index entry to add (e.g. '- [[entities/rust]] — The Rust programming language')")]
    pub entry: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppendLogRequest {
    #[schemars(description = "Log entry (e.g. 'ingest | Article Title')")]
    pub entry: String,
}

// --- Server struct ---

struct WikiServer {
    queue: Mutex<Queue>,
    wiki: Wiki,
    config: Config,
}

fn mcp_err(msg: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(msg.to_string(), None)
}

#[tool_router]
impl WikiServer {
    #[tool(description = "Get the next queued item from the processing queue and mark it in-progress")]
    fn get_next_item(&self) -> Result<String, ErrorData> {
        let queue = self.queue.lock().map_err(|e| mcp_err(e))?;
        let item = queue.get_next_queued().map_err(|e| mcp_err(e))?;

        match item {
            Some(item) => {
                queue.mark_in_progress(item.id).map_err(|e| mcp_err(e))?;
                let json = serde_json::json!({
                    "id": item.id,
                    "file_path": item.file_path.to_string_lossy(),
                    "file_type": item.file_type.as_str(),
                    "parent_id": item.parent_id,
                });
                Ok(serde_json::to_string_pretty(&json).unwrap())
            }
            None => Ok("null (no queued items)".to_string()),
        }
    }

    #[tool(description = "Mark a queue item as complete with the path to the created wiki page")]
    fn complete_item(&self, Parameters(req): Parameters<CompleteItemRequest>) -> Result<String, ErrorData> {
        let queue = self.queue.lock().map_err(|e| mcp_err(e))?;
        queue.mark_complete(req.id, &req.wiki_page_path).map_err(|e| mcp_err(e))?;

        // Check if parent has all children complete
        let item = queue.get_item(req.id).map_err(|e| mcp_err(e))?;
        if let Some(parent_id) = item.parent_id {
            if queue.all_children_complete(parent_id).map_err(|e| mcp_err(e))? {
                return Ok(format!(
                    "Item {} completed. Parent {} has all children complete — book summary page needed.",
                    req.id, parent_id
                ));
            }
        }

        Ok(format!("Item {} marked complete.", req.id))
    }

    #[tool(description = "Mark a queue item as rejected with a reason")]
    fn reject_item(&self, Parameters(req): Parameters<RejectItemRequest>) -> Result<String, ErrorData> {
        let queue = self.queue.lock().map_err(|e| mcp_err(e))?;
        queue.mark_rejected(req.id, &req.reason).map_err(|e| mcp_err(e))?;
        Ok(format!("Item {} rejected: {}", req.id, req.reason))
    }

    #[tool(description = "Mark a queue item as errored with an error message")]
    fn error_item(&self, Parameters(req): Parameters<ErrorItemRequest>) -> Result<String, ErrorData> {
        let queue = self.queue.lock().map_err(|e| mcp_err(e))?;
        queue.mark_error(req.id, &req.message).map_err(|e| mcp_err(e))?;
        Ok(format!("Item {} marked as error: {}", req.id, req.message))
    }

    #[tool(description = "List queue items, optionally filtered by status (queued, in_progress, complete, rejected, error)")]
    fn list_items(&self, Parameters(req): Parameters<ListItemsRequest>) -> Result<String, ErrorData> {
        let queue = self.queue.lock().map_err(|e| mcp_err(e))?;
        let status_filter = req.status.as_deref().and_then(ItemStatus::parse);
        let items = queue.list_items(status_filter.as_ref()).map_err(|e| mcp_err(e))?;

        let json: Vec<serde_json::Value> = items.iter().map(|item| {
            serde_json::json!({
                "id": item.id,
                "file_path": item.file_path.to_string_lossy(),
                "file_type": item.file_type.as_str(),
                "status": item.status.as_str(),
                "parent_id": item.parent_id,
                "wiki_page_path": item.wiki_page_path,
                "error_message": item.error_message,
            })
        }).collect();

        Ok(serde_json::to_string_pretty(&json).unwrap())
    }

    #[tool(description = "Read the preprocessed text content of a source file by queue item ID")]
    fn read_source(&self, Parameters(req): Parameters<ReadSourceRequest>) -> Result<String, ErrorData> {
        let processed_path = self.config.paths.processed_dir.join(format!("{}.txt", req.id));
        std::fs::read_to_string(&processed_path)
            .map_err(|e| mcp_err(format!("failed to read processed text for item {}: {}", req.id, e)))
    }

    #[tool(description = "Read an existing wiki page by its relative path")]
    fn read_page(&self, Parameters(req): Parameters<ReadPageRequest>) -> Result<String, ErrorData> {
        self.wiki.read_page(&req.path).map_err(|e| mcp_err(e))
    }

    #[tool(description = "Create or overwrite a wiki page with markdown content")]
    fn write_page(&self, Parameters(req): Parameters<WritePageRequest>) -> Result<String, ErrorData> {
        self.wiki.write_page(&req.path, &req.content).map_err(|e| mcp_err(e))?;
        Ok(format!("Written: {}", req.path))
    }

    #[tool(description = "List wiki pages, optionally within a subdirectory (entities, concepts, claims, sources)")]
    fn list_pages(&self, Parameters(req): Parameters<ListPagesRequest>) -> Result<String, ErrorData> {
        let pages = self.wiki.list_pages(req.directory.as_deref()).map_err(|e| mcp_err(e))?;
        Ok(serde_json::to_string_pretty(&pages).unwrap())
    }

    #[tool(description = "Read the current wiki index.md")]
    fn read_index(&self) -> Result<String, ErrorData> {
        self.wiki.read_index().map_err(|e| mcp_err(e))
    }

    #[tool(description = "Add or update an entry in index.md")]
    fn update_index(&self, Parameters(req): Parameters<UpdateIndexRequest>) -> Result<String, ErrorData> {
        self.wiki.update_index(&req.entry).map_err(|e| mcp_err(e))?;
        Ok("Index updated.".to_string())
    }

    #[tool(description = "Append a timestamped entry to log.md")]
    fn append_log(&self, Parameters(req): Parameters<AppendLogRequest>) -> Result<String, ErrorData> {
        self.wiki.append_log(&req.entry).map_err(|e| mcp_err(e))?;
        Ok("Log entry appended.".to_string())
    }
}

impl ServerHandler for WikiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("AI Wiki MCP server. Provides tools to process a queue of source files and build an Obsidian wiki.")
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load config
    let config_path = std::env::var("AI_WIKI_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("ai-wiki.toml"));

    let config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };

    let queue = Queue::open(&config.paths.database_path)?;
    let wiki = Wiki::new(config.paths.wiki_dir.clone());
    wiki.init()?;

    let server = WikiServer {
        queue: Mutex::new(queue),
        wiki,
        config,
    };

    // Run MCP server over stdio
    let transport = stdio();
    let server_handle = server.serve(transport).await?;
    server_handle.waiting().await?;

    Ok(())
}
```

- [ ] **Step 2: Run `cargo check -p ai-wiki-mcp`**

This is the riskiest compile. The `rmcp` macro API may have changed since the plan was written. If `cargo check` fails:
- Check `rmcp` examples in the crate source or docs.rs for exact macro syntax
- The `#[tool_router]`, `Parameters<T>`, `ErrorData`, and `ServerHandler` patterns above are based on `rmcp` 1.x but macro APIs can shift between minor versions
- Common issues: import paths may differ, `ErrorData::internal_error` signature may differ, `ServerInfo::new()` may take different args
- Fix all errors in a single batch before recompiling

- [ ] **Step 3: Commit**

```bash
git add crates/ai-wiki-mcp/
git commit -m "feat: add MCP server with queue, source, and wiki tools"
```

---

## Chunk 7: Integration and Polish

### Task 9: Wiki CLAUDE.md Schema

**Files:**
- Create: initial `wiki/CLAUDE.md` content generated by the wiki init

- [ ] **Step 1: Add CLAUDE.md generation to wiki init**

In `crates/ai-wiki-core/src/wiki.rs`, add to the `init()` method:

```rust
let claude_md_path = self.root.join("CLAUDE.md");
if !claude_md_path.exists() {
    fs::write(&claude_md_path, Self::default_claude_md())?;
}
```

Add the method:
```rust
fn default_claude_md() -> String {
    r#"# AI Wiki Schema

## Wiki Structure

This is an Obsidian-native wiki maintained by an LLM via MCP tools.

### Directories
- `entities/` — People, places, organizations
- `concepts/` — Ideas, themes, theories
- `claims/` — Specific assertions from sources (tag data points with `data-point: true`)
- `sources/` — Summaries of ingested source files

### Page Format

Every page must have YAML frontmatter:

```yaml
---
type: entity | concept | claim | source
tags: [relevant, tags]
sources: [source-filename.pdf]
created: YYYY-MM-DD
updated: YYYY-MM-DD
data-point: true  # only for claims that are data points
contradicted: true  # only if contradicted by another source
---
```

Use `[[wikilinks]]` for all cross-references between pages.

### Contradictions

When a new source contradicts an existing claim or page, add a callout:

```markdown
> [!warning] Contradiction
> Source A claims X, but Source B (this source) claims Y.
```

Tag the page with `contradicted: true` in frontmatter.

### Ingestion Workflow

For each source item from the queue:

1. Call `get_next_item` to receive the next source
2. Call `read_source` to read the preprocessed text
3. Call `read_index` to understand what exists in the wiki
4. Read relevant existing pages with `read_page`
5. Extract entities, concepts, claims, and data points
6. Create or update wiki pages with `write_page`
7. Update cross-references using `[[wikilinks]]`
8. Flag any contradictions with existing content
9. Call `update_index` for each new page
10. Call `append_log` with a summary of what was ingested
11. Call `complete_item` with the primary wiki page path

### Index Format

Entries in index.md follow this format:
```
- [[directory/page-name]] — One-line summary
```

Organized under section headings: ## Entities, ## Concepts, ## Claims, ## Sources

### Log Format

Each log entry is prefixed: `## [YYYY-MM-DD] action | Title`

Actions: `ingest`, `update`, `query`, `lint`
"#.to_string()
}
```

- [ ] **Step 2: Run `cargo check -p ai-wiki-core`**

- [ ] **Step 3: Run tests**

```bash
cargo test -p ai-wiki-core -- --nocapture 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add crates/ai-wiki-core/src/wiki.rs
git commit -m "feat: generate CLAUDE.md wiki schema on init"
```

### Task 10: Release Profile and Final Checks

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Add release profile to workspace Cargo.toml**

```toml
[profile.release]
codegen-units = 1
debug = false
lto = true
opt-level = "z"
panic = "abort"
strip = true
```

- [ ] **Step 2: Run full test suite**

```bash
cargo test --workspace 2>&1
```

Expected: all tests pass.

- [ ] **Step 3: Run clippy on full workspace**

```bash
cargo clippy --workspace 2>&1
```

Fix any warnings.

- [ ] **Step 4: Run `cargo fmt --check`**

```bash
cargo fmt --check 2>&1
```

Apply `cargo fmt` if needed.

- [ ] **Step 5: Build release binaries**

```bash
cargo build --release 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml
git commit -m "chore: add release profile for optimized binaries"
```

### Task 11: Manual Integration Test

- [ ] **Step 1: Test full workflow end-to-end**

```bash
# Create a test source
mkdir -p /tmp/ai-wiki-test/raw
echo "# Test Article\n\nThis is about Rust programming." > /tmp/ai-wiki-test/raw/test-article.md

# Ingest it
cargo run -p ai-wiki -- --config /tmp/ai-wiki-test/config.toml ingest /tmp/ai-wiki-test/raw/test-article.md

# Verify queue has the item
cargo run -p ai-wiki -- --config /tmp/ai-wiki-test/config.toml tui
# (check the TUI shows the item as queued, then press q to exit)

# Register MCP server with Claude Code
claude mcp add ai-wiki -- cargo run -p ai-wiki-mcp

# Now Claude Code can use the MCP tools to process the queue
```

- [ ] **Step 2: Commit any fixes from integration testing**

```bash
git add -A
git commit -m "fix: integration test fixes"
```
