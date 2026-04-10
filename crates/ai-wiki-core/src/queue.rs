use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

// ─── Enums ───────────────────────────────────────────────────────────────────

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

// ─── Structs ──────────────────────────────────────────────────────────────────

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

// ─── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("item not found: {0}")]
    NotFound(i64),

    #[error("invalid status: {0}")]
    InvalidStatus(String),

    #[error("file already enqueued: {0}")]
    AlreadyEnqueued(String),

    #[error("invalid status transition for item {id}: expected {expected}, got {actual}")]
    InvalidTransition {
        id: i64,
        expected: String,
        actual: String,
    },
}

// ─── Queue ────────────────────────────────────────────────────────────────────

pub struct Queue {
    conn: Connection,
}

impl Queue {
    /// Open a persistent SQLite database at `db_path`.
    pub fn open(db_path: &Path) -> Result<Self, QueueError> {
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        let mut queue = Self { conn };
        queue.create_tables()?;
        Ok(queue)
    }

    /// Open an in-memory SQLite database (useful for tests).
    pub fn open_in_memory() -> Result<Self, QueueError> {
        let conn = Connection::open_in_memory()?;
        let mut queue = Self { conn };
        queue.create_tables()?;
        Ok(queue)
    }

    /// Create the queue_items table and its indexes if they do not already exist.
    fn create_tables(&mut self) -> Result<(), QueueError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS queue_items (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path       TEXT    NOT NULL,
                file_type       TEXT    NOT NULL,
                status          TEXT    NOT NULL DEFAULT 'queued',
                parent_id       INTEGER REFERENCES queue_items(id),
                wiki_page_path  TEXT,
                error_message   TEXT,
                created_at      TEXT    NOT NULL,
                started_at      TEXT,
                completed_at    TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_queue_status    ON queue_items (status);
            CREATE INDEX IF NOT EXISTS idx_queue_parent_id ON queue_items (parent_id);
            CREATE INDEX IF NOT EXISTS idx_queue_created   ON queue_items (created_at);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_queue_file_parent ON queue_items (file_path, COALESCE(parent_id, 0));",
        )?;
        Ok(())
    }

    // ─── Write operations ─────────────────────────────────────────────────────

    /// Insert a new item with status `Queued`.
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

    /// Check if a file has already been enqueued (by file_path and parent_id).
    pub fn is_already_enqueued(
        &self,
        file_path: &Path,
        parent_id: Option<i64>,
    ) -> Result<bool, QueueError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM queue_items WHERE file_path = ?1 AND parent_id IS ?2",
            params![file_path.to_string_lossy().as_ref(), parent_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Set status to `InProgress` and record `started_at`.
    /// Returns `InvalidTransition` if the item is not currently `queued`.
    pub fn mark_in_progress(&self, id: i64) -> Result<(), QueueError> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE queue_items SET status = ?1, started_at = ?2 WHERE id = ?3 AND status = ?4",
            params![
                ItemStatus::InProgress.as_str(),
                now,
                id,
                ItemStatus::Queued.as_str(),
            ],
        )?;
        if rows == 0 {
            return Err(self.not_found_or_transition(
                id,
                ItemStatus::Queued.as_str(),
            )?);
        }
        Ok(())
    }

    /// Set status to `Complete` and record the output wiki page path and `completed_at`.
    /// Returns `InvalidTransition` if the item is not currently `in_progress`.
    pub fn mark_complete(&self, id: i64, wiki_page_path: &str) -> Result<(), QueueError> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE queue_items SET status = ?1, wiki_page_path = ?2, completed_at = ?3 WHERE id = ?4 AND status = ?5",
            params![ItemStatus::Complete.as_str(), wiki_page_path, now, id, ItemStatus::InProgress.as_str()],
        )?;
        if rows == 0 {
            return Err(self.not_found_or_transition(
                id,
                ItemStatus::InProgress.as_str(),
            )?);
        }
        Ok(())
    }

    /// Set status to `Rejected` and record the rejection reason and `completed_at`.
    /// Returns `InvalidTransition` if the item is not currently `queued`.
    pub fn mark_rejected(&self, id: i64, reason: &str) -> Result<(), QueueError> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE queue_items SET status = ?1, error_message = ?2, completed_at = ?3 WHERE id = ?4 AND status = ?5",
            params![ItemStatus::Rejected.as_str(), reason, now, id, ItemStatus::Queued.as_str()],
        )?;
        if rows == 0 {
            return Err(self.not_found_or_transition(
                id,
                ItemStatus::Queued.as_str(),
            )?);
        }
        Ok(())
    }

    /// Set status to `Error` and record the error message and `completed_at`.
    /// Errors can happen from any state, so no status guard is applied.
    pub fn mark_error(&self, id: i64, message: &str) -> Result<(), QueueError> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE queue_items SET status = ?1, error_message = ?2, completed_at = ?3 WHERE id = ?4",
            params![ItemStatus::Error.as_str(), message, now, id],
        )?;
        if rows == 0 {
            return Err(QueueError::NotFound(id));
        }
        Ok(())
    }

    /// Reset all `in_progress` items back to `queued` (used on startup after a crash).
    pub fn reset_in_progress(&self) -> Result<u64, QueueError> {
        let rows = self.conn.execute(
            "UPDATE queue_items SET status = ?1, started_at = NULL WHERE status = ?2",
            params![ItemStatus::Queued.as_str(), ItemStatus::InProgress.as_str()],
        )?;
        Ok(rows as u64)
    }

    // ─── Read operations ──────────────────────────────────────────────────────

    /// Retrieve a single item by its id.
    pub fn get_item(&self, id: i64) -> Result<QueueItem, QueueError> {
        let item = self
            .conn
            .query_row(
                "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                    error_message, created_at, started_at, completed_at
             FROM queue_items WHERE id = ?1",
                params![id],
                Self::row_to_item,
            )
            .optional()?;
        item.ok_or(QueueError::NotFound(id))
    }

    /// Return the oldest queued item, or `None` if the queue is empty.
    pub fn get_next_queued(&self) -> Result<Option<QueueItem>, QueueError> {
        let item = self
            .conn
            .query_row(
                "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                    error_message, created_at, started_at, completed_at
             FROM queue_items
             WHERE status = ?1
             ORDER BY created_at ASC
             LIMIT 1",
                params![ItemStatus::Queued.as_str()],
                Self::row_to_item,
            )
            .optional()?;
        Ok(item)
    }

    /// List all items, optionally filtered by status.
    pub fn list_items(
        &self,
        status_filter: Option<&ItemStatus>,
    ) -> Result<Vec<QueueItem>, QueueError> {
        let items = if let Some(status) = status_filter {
            let mut stmt = self.conn.prepare(
                "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                        error_message, created_at, started_at, completed_at
                 FROM queue_items
                 WHERE status = ?1
                 ORDER BY created_at ASC",
            )?;
            stmt.query_map(params![status.as_str()], Self::row_to_item)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                        error_message, created_at, started_at, completed_at
                 FROM queue_items
                 ORDER BY created_at ASC",
            )?;
            stmt.query_map([], Self::row_to_item)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(items)
    }

    /// Return counts of items grouped by status as a `Vec<(status_string, count)>`.
    pub fn count_by_status(&self) -> Result<Vec<(String, u64)>, QueueError> {
        let mut stmt = self
            .conn
            .prepare("SELECT status, COUNT(*) FROM queue_items GROUP BY status")?;
        let counts = stmt
            .query_map([], |row| {
                let status: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((status, count as u64))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(counts)
    }

    /// Return all direct children of `parent_id`.
    pub fn children_of(&self, parent_id: i64) -> Result<Vec<QueueItem>, QueueError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                    error_message, created_at, started_at, completed_at
             FROM queue_items
             WHERE parent_id = ?1
             ORDER BY created_at ASC",
        )?;
        let items = stmt
            .query_map(params![parent_id], Self::row_to_item)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    /// Return `true` if every child of `parent_id` has status `Complete`.
    /// Also returns `true` if there are no children at all.
    pub fn all_children_complete(&self, parent_id: i64) -> Result<bool, QueueError> {
        let incomplete: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM queue_items
             WHERE parent_id = ?1 AND status != ?2",
            params![parent_id, ItemStatus::Complete.as_str()],
            |row| row.get(0),
        )?;
        Ok(incomplete == 0)
    }

    /// Return the oldest queued item and atomically mark it `in_progress`.
    /// Uses an explicit transaction so no other worker can claim the same item.
    pub fn claim_next_queued(&self) -> Result<Option<QueueItem>, QueueError> {
        let tx = self.conn.unchecked_transaction()?;
        let item = tx
            .query_row(
                "SELECT id, file_path, file_type, status, parent_id, wiki_page_path,
                    error_message, created_at, started_at, completed_at
             FROM queue_items
             WHERE status = ?1
             ORDER BY created_at ASC
             LIMIT 1",
                params![ItemStatus::Queued.as_str()],
                Self::row_to_item,
            )
            .optional()?;

        if let Some(ref item) = item {
            let now = Utc::now().to_rfc3339();
            tx.execute(
                "UPDATE queue_items SET status = ?1, started_at = ?2 WHERE id = ?3",
                params![ItemStatus::InProgress.as_str(), now, item.id],
            )?;
        }

        tx.commit()?;
        Ok(item)
    }

    // ─── Helper ───────────────────────────────────────────────────────────────

    /// Determine whether a zero-row update was due to the item not existing or
    /// a status mismatch, and return the appropriate error.
    fn not_found_or_transition(&self, id: i64, expected: &str) -> Result<QueueError, QueueError> {
        match self.get_item(id) {
            Ok(item) => Ok(QueueError::InvalidTransition {
                id,
                expected: expected.to_owned(),
                actual: item.status.as_str().to_owned(),
            }),
            Err(QueueError::NotFound(_)) => Ok(QueueError::NotFound(id)),
            Err(e) => Err(e),
        }
    }

    fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueueItem> {
        let file_path_str: String = row.get(1)?;
        let file_type_str: String = row.get(2)?;
        let status_str: String = row.get(3)?;
        let created_at_str: String = row.get(7)?;
        let started_at_str: Option<String> = row.get(8)?;
        let completed_at_str: Option<String> = row.get(9)?;

        let status = ItemStatus::parse(&status_str).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!(
                    "unknown status: {status_str}"
                ))),
            )
        })?;

        let parse_ts = |s: String| -> rusqlite::Result<DateTime<Utc>> {
            DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
        };

        Ok(QueueItem {
            id: row.get(0)?,
            file_path: PathBuf::from(file_path_str),
            file_type: FileType::parse(&file_type_str),
            status,
            parent_id: row.get(4)?,
            wiki_page_path: row.get(5)?,
            error_message: row.get(6)?,
            created_at: parse_ts(created_at_str)?,
            started_at: started_at_str.map(parse_ts).transpose()?,
            completed_at: completed_at_str.map(parse_ts).transpose()?,
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_queue() -> Queue {
        Queue::open_in_memory().expect("in-memory queue")
    }

    #[test]
    fn test_enqueue_and_get() {
        let q = make_queue();
        let id = q
            .enqueue(Path::new("docs/readme.md"), FileType::Markdown, None)
            .unwrap();
        assert_eq!(id, 1);

        let item = q.get_item(id).unwrap();
        assert_eq!(item.id, 1);
        assert_eq!(item.file_path, PathBuf::from("docs/readme.md"));
        assert_eq!(item.file_type, FileType::Markdown);
        assert_eq!(item.status, ItemStatus::Queued);
        assert!(item.parent_id.is_none());
        assert!(item.wiki_page_path.is_none());
        assert!(item.error_message.is_none());
        assert!(item.started_at.is_none());
        assert!(item.completed_at.is_none());
    }

    #[test]
    fn test_enqueue_with_parent() {
        let q = make_queue();
        let parent_id = q
            .enqueue(Path::new("archive.zip"), FileType::Zip, None)
            .unwrap();
        let child_id = q
            .enqueue(
                Path::new("archive/file.txt"),
                FileType::Text,
                Some(parent_id),
            )
            .unwrap();

        let child = q.get_item(child_id).unwrap();
        assert_eq!(child.parent_id, Some(parent_id));
    }

    #[test]
    fn test_get_next_queued() {
        let q = make_queue();

        // Empty queue returns None.
        let next = q.get_next_queued().unwrap();
        assert!(next.is_none());

        // Enqueue two items; oldest should come back first.
        let id1 = q.enqueue(Path::new("a.txt"), FileType::Text, None).unwrap();
        let id2 = q.enqueue(Path::new("b.txt"), FileType::Text, None).unwrap();

        let next = q.get_next_queued().unwrap().unwrap();
        assert_eq!(next.id, id1);

        // After marking the first in_progress the second becomes next.
        q.mark_in_progress(id1).unwrap();
        let next = q.get_next_queued().unwrap().unwrap();
        assert_eq!(next.id, id2);
    }

    #[test]
    fn test_status_transitions() {
        let q = make_queue();
        let id = q
            .enqueue(Path::new("doc.md"), FileType::Markdown, None)
            .unwrap();

        q.mark_in_progress(id).unwrap();
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::InProgress);
        assert!(item.started_at.is_some());

        q.mark_complete(id, "wiki/doc.md").unwrap();
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Complete);
        assert_eq!(item.wiki_page_path.as_deref(), Some("wiki/doc.md"));
        assert!(item.completed_at.is_some());
    }

    #[test]
    fn test_mark_rejected() {
        let q = make_queue();
        let id = q
            .enqueue(Path::new("sensitive.pdf"), FileType::Pdf, None)
            .unwrap();
        q.mark_rejected(id, "sensitive filename").unwrap();

        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Rejected);
        assert_eq!(item.error_message.as_deref(), Some("sensitive filename"));
        assert!(item.completed_at.is_some());
    }

    #[test]
    fn test_mark_error() {
        let q = make_queue();
        let id = q
            .enqueue(Path::new("corrupt.pdf"), FileType::Pdf, None)
            .unwrap();
        q.mark_error(id, "failed to parse PDF").unwrap();

        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Error);
        assert_eq!(item.error_message.as_deref(), Some("failed to parse PDF"));
        assert!(item.completed_at.is_some());
    }

    #[test]
    fn test_list_items_all() {
        let q = make_queue();
        q.enqueue(Path::new("a.md"), FileType::Markdown, None)
            .unwrap();
        q.enqueue(Path::new("b.txt"), FileType::Text, None).unwrap();

        let items = q.list_items(None).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_list_items_filtered() {
        let q = make_queue();
        let id1 = q
            .enqueue(Path::new("a.md"), FileType::Markdown, None)
            .unwrap();
        q.enqueue(Path::new("b.txt"), FileType::Text, None).unwrap();
        q.mark_in_progress(id1).unwrap();

        let queued = q.list_items(Some(&ItemStatus::Queued)).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].file_path, PathBuf::from("b.txt"));

        let in_progress = q.list_items(Some(&ItemStatus::InProgress)).unwrap();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].id, id1);
    }

    #[test]
    fn test_reset_in_progress() {
        let q = make_queue();
        let id1 = q
            .enqueue(Path::new("a.md"), FileType::Markdown, None)
            .unwrap();
        let id2 = q
            .enqueue(Path::new("b.md"), FileType::Markdown, None)
            .unwrap();
        q.mark_in_progress(id1).unwrap();
        q.mark_in_progress(id2).unwrap();

        let reset = q.reset_in_progress().unwrap();
        assert_eq!(reset, 2);

        let item = q.get_item(id1).unwrap();
        assert_eq!(item.status, ItemStatus::Queued);
        assert!(item.started_at.is_none());
    }

    #[test]
    fn test_children_and_completion_check() {
        let q = make_queue();
        let parent_id = q
            .enqueue(Path::new("archive.zip"), FileType::Zip, None)
            .unwrap();
        let c1 = q
            .enqueue(Path::new("archive/a.txt"), FileType::Text, Some(parent_id))
            .unwrap();
        let c2 = q
            .enqueue(Path::new("archive/b.txt"), FileType::Text, Some(parent_id))
            .unwrap();

        let children = q.children_of(parent_id).unwrap();
        assert_eq!(children.len(), 2);

        assert!(!q.all_children_complete(parent_id).unwrap());

        q.mark_in_progress(c1).unwrap();
        q.mark_complete(c1, "wiki/a.md").unwrap();
        assert!(!q.all_children_complete(parent_id).unwrap());

        q.mark_in_progress(c2).unwrap();
        q.mark_complete(c2, "wiki/b.md").unwrap();
        assert!(q.all_children_complete(parent_id).unwrap());
    }

    #[test]
    fn test_count_by_status() {
        let q = make_queue();
        let id1 = q
            .enqueue(Path::new("a.md"), FileType::Markdown, None)
            .unwrap();
        let id2 = q
            .enqueue(Path::new("b.md"), FileType::Markdown, None)
            .unwrap();
        q.enqueue(Path::new("c.md"), FileType::Markdown, None)
            .unwrap();

        q.mark_in_progress(id1).unwrap();
        q.mark_complete(id1, "wiki/a.md").unwrap();
        q.mark_in_progress(id2).unwrap();
        q.mark_error(id2, "oops").unwrap();

        let counts = q.count_by_status().unwrap();
        let as_map: std::collections::HashMap<_, _> = counts.into_iter().collect();
        assert_eq!(as_map.get("queued").copied(), Some(1));
        assert_eq!(as_map.get("complete").copied(), Some(1));
        assert_eq!(as_map.get("error").copied(), Some(1));
    }

    #[test]
    fn test_mark_nonexistent_item_returns_error() {
        let q = make_queue();

        let result = q.mark_in_progress(999);
        assert!(matches!(result, Err(QueueError::NotFound(999))));

        let result = q.mark_complete(999, "wiki/x.md");
        assert!(matches!(result, Err(QueueError::NotFound(999))));

        let result = q.mark_rejected(999, "reason");
        assert!(matches!(result, Err(QueueError::NotFound(999))));

        let result = q.mark_error(999, "msg");
        assert!(matches!(result, Err(QueueError::NotFound(999))));

        let result = q.get_item(999);
        assert!(matches!(result, Err(QueueError::NotFound(999))));
    }

    #[test]
    fn test_is_already_enqueued() {
        let q = make_queue();
        assert!(!q.is_already_enqueued(Path::new("a.txt"), None).unwrap());

        q.enqueue(Path::new("a.txt"), FileType::Text, None).unwrap();
        assert!(q.is_already_enqueued(Path::new("a.txt"), None).unwrap());

        // Same path with different parent is not a duplicate
        let parent = q.enqueue(Path::new("archive.zip"), FileType::Zip, None).unwrap();
        assert!(!q.is_already_enqueued(Path::new("a.txt"), Some(parent)).unwrap());

        q.enqueue(Path::new("a.txt"), FileType::Text, Some(parent)).unwrap();
        assert!(q.is_already_enqueued(Path::new("a.txt"), Some(parent)).unwrap());
    }

    #[test]
    fn test_duplicate_enqueue_rejected_by_unique_index() {
        let q = make_queue();
        q.enqueue(Path::new("a.txt"), FileType::Text, None).unwrap();
        let result = q.enqueue(Path::new("a.txt"), FileType::Text, None);
        assert!(result.is_err()); // UNIQUE constraint violation
    }
}
