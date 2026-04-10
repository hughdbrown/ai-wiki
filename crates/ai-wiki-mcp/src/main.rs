use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::io::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;

use ai_wiki_core::config::Config;
use ai_wiki_core::queue::{ItemStatus, Queue, QueueItem};
use ai_wiki_core::wiki::Wiki;

// ─── Parameter Structs ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompleteItemRequest {
    #[schemars(description = "The queue item ID to mark as complete")]
    id: i64,
    #[schemars(description = "The relative path to the generated wiki page")]
    wiki_page_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RejectItemRequest {
    #[schemars(description = "The queue item ID to reject")]
    id: i64,
    #[schemars(description = "The reason for rejection")]
    reason: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ErrorItemRequest {
    #[schemars(description = "The queue item ID to mark as error")]
    id: i64,
    #[schemars(description = "The error message")]
    message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListItemsRequest {
    #[schemars(
        description = "Optional status filter: queued, in_progress, complete, rejected, error"
    )]
    status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadSourceRequest {
    #[schemars(description = "The queue item ID whose preprocessed text to read")]
    id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadPageRequest {
    #[schemars(description = "Relative path to the wiki page (e.g. entities/rust.md)")]
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WritePageRequest {
    #[schemars(description = "Relative path to the wiki page (e.g. entities/rust.md)")]
    path: String,
    #[schemars(description = "The markdown content to write")]
    content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPagesRequest {
    #[schemars(description = "Optional subdirectory to list (e.g. entities, concepts)")]
    directory: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateIndexRequest {
    #[schemars(description = "The index entry to append (e.g. '- [[entities/rust]]')")]
    entry: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppendLogRequest {
    #[schemars(description = "The log entry text to append")]
    entry: String,
}

// ─── Server ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct WikiServer {
    queue: std::sync::Arc<Mutex<Queue>>,
    wiki: std::sync::Arc<Wiki>,
    config: std::sync::Arc<Config>,
    tool_router: ToolRouter<Self>,
}

fn item_to_json(item: &QueueItem) -> serde_json::Value {
    serde_json::json!({
        "id": item.id,
        "file_path": item.file_path.to_string_lossy(),
        "file_type": item.file_type.as_str(),
        "status": item.status.as_str(),
        "parent_id": item.parent_id,
        "wiki_page_path": item.wiki_page_path,
        "error_message": item.error_message,
        "created_at": item.created_at.to_rfc3339(),
        "started_at": item.started_at.map(|t| t.to_rfc3339()),
        "completed_at": item.completed_at.map(|t| t.to_rfc3339()),
    })
}

#[tool_router]
impl WikiServer {
    // ─── Queue Tools ─────────────────────────────────────────────────────────

    /// Get the next queued item and mark it as in_progress.
    #[tool(
        description = "Get the next queued item and mark it as in_progress. Returns JSON with item details or null if queue is empty."
    )]
    async fn get_next_item(&self) -> Result<String, String> {
        let queue = self.queue.clone();
        tokio::task::spawn_blocking(move || {
            let queue = queue.lock().map_err(|e| format!("Lock error: {e}"))?;
            let item = queue
                .claim_next_queued()
                .map_err(|e| format!("Queue error: {e}"))?;
            match item {
                Some(item) => {
                    Ok(serde_json::to_string(&item_to_json(&item))
                        .map_err(|e| format!("JSON error: {e}"))?)
                }
                None => Ok("null".to_string()),
            }
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    /// Mark a queue item as complete with the path to its wiki page.
    #[tool(
        description = "Mark a queue item as complete. Provide the item ID and the relative path to the generated wiki page. Returns whether all sibling items under the same parent are also complete."
    )]
    async fn complete_item(
        &self,
        Parameters(req): Parameters<CompleteItemRequest>,
    ) -> Result<String, String> {
        let queue = self.queue.clone();
        tokio::task::spawn_blocking(move || {
            let queue = queue.lock().map_err(|e| format!("Lock error: {e}"))?;
            queue
                .mark_complete(req.id, &req.wiki_page_path)
                .map_err(|e| format!("Failed to mark complete: {e}"))?;

            let item = queue
                .get_item(req.id)
                .map_err(|e| format!("Failed to read item: {e}"))?;

            let all_siblings_complete = if let Some(parent_id) = item.parent_id {
                queue
                    .all_children_complete(parent_id)
                    .map_err(|e| format!("Failed to check siblings: {e}"))?
            } else {
                true
            };

            Ok(serde_json::json!({
                "status": "complete",
                "id": req.id,
                "all_siblings_complete": all_siblings_complete,
            })
            .to_string())
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    /// Mark a queue item as rejected with a reason.
    #[tool(
        description = "Mark a queue item as rejected. Provide the item ID and reason for rejection."
    )]
    async fn reject_item(
        &self,
        Parameters(req): Parameters<RejectItemRequest>,
    ) -> Result<String, String> {
        let queue = self.queue.clone();
        tokio::task::spawn_blocking(move || {
            let queue = queue.lock().map_err(|e| format!("Lock error: {e}"))?;
            queue
                .mark_rejected(req.id, &req.reason)
                .map_err(|e| format!("Failed to mark rejected: {e}"))?;
            Ok(serde_json::json!({
                "status": "rejected",
                "id": req.id,
            })
            .to_string())
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    /// Mark a queue item as errored with an error message.
    #[tool(description = "Mark a queue item as errored. Provide the item ID and error message.")]
    async fn error_item(
        &self,
        Parameters(req): Parameters<ErrorItemRequest>,
    ) -> Result<String, String> {
        let queue = self.queue.clone();
        tokio::task::spawn_blocking(move || {
            let queue = queue.lock().map_err(|e| format!("Lock error: {e}"))?;
            queue
                .mark_error(req.id, &req.message)
                .map_err(|e| format!("Failed to mark error: {e}"))?;
            Ok(serde_json::json!({
                "status": "error",
                "id": req.id,
            })
            .to_string())
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    /// List queue items, optionally filtered by status.
    #[tool(
        description = "List queue items. Optionally filter by status: queued, in_progress, complete, rejected, error."
    )]
    async fn list_items(
        &self,
        Parameters(req): Parameters<ListItemsRequest>,
    ) -> Result<String, String> {
        let queue = self.queue.clone();
        tokio::task::spawn_blocking(move || {
            let queue = queue.lock().map_err(|e| format!("Lock error: {e}"))?;
            let status_filter: Option<ItemStatus> = match &req.status {
                Some(s) => {
                    let parsed =
                        ItemStatus::parse(s).ok_or_else(|| format!("Invalid status: {s}"))?;
                    Some(parsed)
                }
                None => None,
            };
            let items = queue
                .list_items(status_filter.as_ref())
                .map_err(|e| format!("Queue error: {e}"))?;
            let json_items: Vec<serde_json::Value> = items.iter().map(item_to_json).collect();
            serde_json::to_string(&json_items).map_err(|e| format!("JSON error: {e}"))
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    // ─── Source Tools ────────────────────────────────────────────────────────

    /// Read preprocessed source text for a queue item.
    #[tool(
        description = "Read the preprocessed text for a queue item. Reads from the processed/ directory using the item ID."
    )]
    async fn read_source(
        &self,
        Parameters(req): Parameters<ReadSourceRequest>,
    ) -> Result<String, String> {
        let config = self.config.clone();
        let queue = self.queue.clone();
        tokio::task::spawn_blocking(move || {
            // Verify item exists in queue before reading
            {
                let queue = queue.lock().map_err(|e| format!("lock error: {e}"))?;
                queue
                    .get_item(req.id)
                    .map_err(|e| format!("item {}: {e}", req.id))?;
            }
            let processed_dir = &config.paths.processed_dir;
            let path = processed_dir.join(format!("{}.txt", req.id));
            std::fs::read_to_string(&path).map_err(|_| {
                format!(
                    "failed to read processed text for item {}: file not found or unreadable",
                    req.id
                )
            })
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    // ─── Wiki Tools ──────────────────────────────────────────────────────────

    /// Read a wiki page by relative path.
    #[tool(description = "Read a wiki page. Provide the relative path (e.g. 'entities/rust.md').")]
    async fn read_page(
        &self,
        Parameters(req): Parameters<ReadPageRequest>,
    ) -> Result<String, String> {
        let wiki = self.wiki.clone();
        tokio::task::spawn_blocking(move || wiki.read_page(&req.path).map_err(|e| format!("{e}")))
            .await
            .map_err(|e| format!("task join error: {e}"))?
    }

    /// Write content to a wiki page.
    #[tool(
        description = "Write or overwrite a wiki page. Provide the relative path and markdown content."
    )]
    async fn write_page(
        &self,
        Parameters(req): Parameters<WritePageRequest>,
    ) -> Result<String, String> {
        let wiki = self.wiki.clone();
        tokio::task::spawn_blocking(move || {
            wiki.write_page(&req.path, &req.content)
                .map_err(|e| format!("{e}"))?;
            Ok(format!("Wrote wiki page: {}", req.path))
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    /// List wiki pages, optionally within a subdirectory.
    #[tool(
        description = "List wiki pages. Optionally specify a subdirectory like 'entities' or 'concepts'."
    )]
    async fn list_pages(
        &self,
        Parameters(req): Parameters<ListPagesRequest>,
    ) -> Result<String, String> {
        let wiki = self.wiki.clone();
        tokio::task::spawn_blocking(move || {
            let pages = wiki
                .list_pages(req.directory.as_deref())
                .map_err(|e| format!("{e}"))?;
            serde_json::to_string(&pages).map_err(|e| format!("JSON error: {e}"))
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    /// Read the wiki index page.
    #[tool(description = "Read the wiki index.md file.")]
    async fn read_index(&self) -> Result<String, String> {
        let wiki = self.wiki.clone();
        tokio::task::spawn_blocking(move || wiki.read_index().map_err(|e| format!("{e}")))
            .await
            .map_err(|e| format!("task join error: {e}"))?
    }

    /// Append an entry to the wiki index.
    #[tool(description = "Append an entry to the wiki index.md (e.g. '- [[entities/rust]]').")]
    async fn update_index(
        &self,
        Parameters(req): Parameters<UpdateIndexRequest>,
    ) -> Result<String, String> {
        let wiki = self.wiki.clone();
        tokio::task::spawn_blocking(move || {
            wiki.update_index(&req.entry).map_err(|e| format!("{e}"))?;
            Ok(format!("Updated index with: {}", req.entry))
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }

    /// Append a timestamped entry to the wiki log.
    #[tool(description = "Append a timestamped entry to the wiki log.md.")]
    async fn append_log(
        &self,
        Parameters(req): Parameters<AppendLogRequest>,
    ) -> Result<String, String> {
        let wiki = self.wiki.clone();
        tokio::task::spawn_blocking(move || {
            wiki.append_log(&req.entry).map_err(|e| format!("{e}"))?;
            Ok(format!("Logged: {}", req.entry))
        })
        .await
        .map_err(|e| format!("task join error: {e}"))?
    }
}

#[tool_handler]
impl ServerHandler for WikiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "AI Wiki MCP server. Use queue tools to get items, \
                 source tools to read preprocessed text, and wiki tools \
                 to read/write wiki pages.",
        )
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use tempfile::tempdir;

    use ai_wiki_core::config::{Config, PathsConfig};
    use ai_wiki_core::queue::{FileType, ItemStatus, Queue};
    use ai_wiki_core::wiki::Wiki;

    use super::WikiServer;

    // ─── Helper ───────────────────────────────────────────────────────────────

    fn make_server() -> (WikiServer, tempfile::TempDir, tempfile::TempDir) {
        let wiki_dir = tempdir().unwrap();
        let processed_dir = tempdir().unwrap();

        let wiki = Wiki::new(wiki_dir.path().to_path_buf());
        wiki.init().unwrap();

        let queue = Queue::open_in_memory().unwrap();

        let mut config = Config::default();
        config.paths = PathsConfig {
            raw_dir: wiki_dir.path().join("raw"),
            wiki_dir: wiki_dir.path().to_path_buf(),
            database_path: wiki_dir.path().join("queue.db"),
            processed_dir: processed_dir.path().to_path_buf(),
        };

        let server = WikiServer {
            queue: Arc::new(Mutex::new(queue)),
            wiki: Arc::new(wiki),
            config: Arc::new(config),
            tool_router: WikiServer::tool_router(),
        };

        (server, wiki_dir, processed_dir)
    }

    // ─── Queue Tool Tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_get_next_item_empty_queue() {
        let (server, _wiki_dir, _processed_dir) = make_server();
        let result = server.get_next_item().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "null");
    }

    #[tokio::test]
    async fn test_get_next_item_returns_item_and_marks_in_progress() {
        let (server, _wiki_dir, _processed_dir) = make_server();

        // Enqueue a file first
        {
            let queue = server.queue.lock().unwrap();
            queue
                .enqueue(Path::new("docs/readme.md"), FileType::Markdown, None)
                .unwrap();
        }

        let result = server.get_next_item().await;
        assert!(result.is_ok());
        let json_str = result.unwrap();
        assert_ne!(json_str, "null");

        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(value["status"], "in_progress");
        assert_eq!(value["file_path"], "docs/readme.md");

        // Verify item is actually marked in_progress in the queue
        let queue = server.queue.lock().unwrap();
        let item = queue.get_item(1).unwrap();
        assert_eq!(item.status, ItemStatus::InProgress);
        assert!(item.started_at.is_some());
    }

    #[tokio::test]
    async fn test_complete_item() {
        use super::CompleteItemRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        // Enqueue and advance to in_progress
        let id = {
            let queue = server.queue.lock().unwrap();
            let id = queue
                .enqueue(Path::new("docs/test.md"), FileType::Markdown, None)
                .unwrap();
            queue.mark_in_progress(id).unwrap();
            id
        };

        let result = server
            .complete_item(Parameters(CompleteItemRequest {
                id,
                wiki_page_path: "entities/test.md".to_string(),
            }))
            .await;
        assert!(result.is_ok(), "complete_item failed: {:?}", result);

        let value: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(value["status"], "complete");
        assert_eq!(value["id"], id);

        // Verify in queue
        let queue = server.queue.lock().unwrap();
        let item = queue.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Complete);
        assert_eq!(item.wiki_page_path.as_deref(), Some("entities/test.md"));
    }

    #[tokio::test]
    async fn test_reject_item() {
        use super::RejectItemRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        let id = {
            let queue = server.queue.lock().unwrap();
            queue
                .enqueue(Path::new("sensitive.pdf"), FileType::Pdf, None)
                .unwrap()
        };

        let result = server
            .reject_item(Parameters(RejectItemRequest {
                id,
                reason: "sensitive content".to_string(),
            }))
            .await;
        assert!(result.is_ok(), "reject_item failed: {:?}", result);

        let value: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(value["status"], "rejected");

        let queue = server.queue.lock().unwrap();
        let item = queue.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Rejected);
        assert_eq!(item.error_message.as_deref(), Some("sensitive content"));
    }

    #[tokio::test]
    async fn test_error_item() {
        use super::ErrorItemRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        let id = {
            let queue = server.queue.lock().unwrap();
            queue
                .enqueue(Path::new("corrupt.pdf"), FileType::Pdf, None)
                .unwrap()
        };

        let result = server
            .error_item(Parameters(ErrorItemRequest {
                id,
                message: "parse failed".to_string(),
            }))
            .await;
        assert!(result.is_ok(), "error_item failed: {:?}", result);

        let value: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(value["status"], "error");

        let queue = server.queue.lock().unwrap();
        let item = queue.get_item(id).unwrap();
        assert_eq!(item.status, ItemStatus::Error);
        assert_eq!(item.error_message.as_deref(), Some("parse failed"));
    }

    #[tokio::test]
    async fn test_list_items_with_filter() {
        use super::ListItemsRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        // Enqueue three items; mark one in_progress and one rejected
        let (id1, _id2, id3) = {
            let queue = server.queue.lock().unwrap();
            let id1 = queue
                .enqueue(Path::new("a.md"), FileType::Markdown, None)
                .unwrap();
            let id2 = queue
                .enqueue(Path::new("b.md"), FileType::Markdown, None)
                .unwrap();
            let id3 = queue
                .enqueue(Path::new("c.md"), FileType::Markdown, None)
                .unwrap();
            queue.mark_in_progress(id1).unwrap();
            queue.mark_rejected(id3, "not needed").unwrap();
            (id1, id2, id3)
        };
        let _ = (id1, id3); // suppress unused warnings

        // Filter by queued — should be 1
        let result = server
            .list_items(Parameters(ListItemsRequest {
                status: Some("queued".to_string()),
            }))
            .await;
        assert!(result.is_ok());
        let items: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["status"], "queued");

        // Filter by rejected — should be 1
        let result = server
            .list_items(Parameters(ListItemsRequest {
                status: Some("rejected".to_string()),
            }))
            .await;
        assert!(result.is_ok());
        let items: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["status"], "rejected");

        // No filter — should be 3
        let result = server
            .list_items(Parameters(ListItemsRequest { status: None }))
            .await;
        assert!(result.is_ok());
        let items: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 3);
    }

    // ─── Source Tool Tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_read_source_with_processed_file() {
        use super::ReadSourceRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, processed_dir) = make_server();

        // Enqueue an item to get a valid ID
        let id = {
            let queue = server.queue.lock().unwrap();
            queue
                .enqueue(Path::new("docs/source.pdf"), FileType::Pdf, None)
                .unwrap()
        };

        // Create the processed text file
        let txt_path = processed_dir.path().join(format!("{id}.txt"));
        std::fs::write(&txt_path, "Preprocessed content for source.").unwrap();

        let result = server
            .read_source(Parameters(ReadSourceRequest { id }))
            .await;
        assert!(result.is_ok(), "read_source failed: {:?}", result);
        assert_eq!(result.unwrap(), "Preprocessed content for source.");
    }

    #[tokio::test]
    async fn test_read_source_missing_file() {
        use super::ReadSourceRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        // Enqueue item but do NOT create a processed file
        let id = {
            let queue = server.queue.lock().unwrap();
            queue
                .enqueue(Path::new("docs/orphan.pdf"), FileType::Pdf, None)
                .unwrap()
        };

        let result = server
            .read_source(Parameters(ReadSourceRequest { id }))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("not found") || msg.contains("unreadable"),
            "unexpected error message: {msg}"
        );
    }

    #[tokio::test]
    async fn test_read_source_nonexistent_queue_item() {
        use super::ReadSourceRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        // ID 999 was never enqueued
        let result = server
            .read_source(Parameters(ReadSourceRequest { id: 999 }))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("999"), "error should mention item ID: {msg}");
    }

    // ─── Wiki Tool Tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_write_and_read_page() {
        use super::{ReadPageRequest, WritePageRequest};
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        let content = "# Rust\n\nA systems programming language.";
        let write_result = server
            .write_page(Parameters(WritePageRequest {
                path: "entities/rust.md".to_string(),
                content: content.to_string(),
            }))
            .await;
        assert!(
            write_result.is_ok(),
            "write_page failed: {:?}",
            write_result
        );

        let read_result = server
            .read_page(Parameters(ReadPageRequest {
                path: "entities/rust.md".to_string(),
            }))
            .await;
        assert!(read_result.is_ok(), "read_page failed: {:?}", read_result);
        assert_eq!(read_result.unwrap(), content);
    }

    #[tokio::test]
    async fn test_update_index_and_read_index() {
        use super::UpdateIndexRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        let result = server
            .update_index(Parameters(UpdateIndexRequest {
                entry: "- [[entities/rust]]".to_string(),
            }))
            .await;
        assert!(result.is_ok(), "update_index failed: {:?}", result);

        let index_result = server.read_index().await;
        assert!(
            index_result.is_ok(),
            "read_index failed: {:?}",
            index_result
        );
        let index = index_result.unwrap();
        assert!(
            index.contains("- [[entities/rust]]"),
            "index should contain added entry"
        );
    }

    #[tokio::test]
    async fn test_append_log() {
        use super::AppendLogRequest;
        use super::ReadPageRequest;
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        let result = server
            .append_log(Parameters(AppendLogRequest {
                entry: "ingest | Rust programming language".to_string(),
            }))
            .await;
        assert!(result.is_ok(), "append_log failed: {:?}", result);

        let log = server
            .read_page(Parameters(ReadPageRequest {
                path: "log.md".to_string(),
            }))
            .await;
        assert!(log.is_ok(), "read_page log.md failed: {:?}", log);
        let log_content = log.unwrap();
        assert!(
            log_content.contains("Rust programming language"),
            "log should contain appended entry"
        );
        // Verify date prefix format ## [YYYY-MM-DD]
        assert!(
            log_content.lines().any(|line| line.starts_with("## [")),
            "log should have ## [YYYY-MM-DD] formatted entry"
        );
    }

    #[tokio::test]
    async fn test_list_pages() {
        use super::{ListPagesRequest, WritePageRequest};
        use rmcp::handler::server::wrapper::Parameters;

        let (server, _wiki_dir, _processed_dir) = make_server();

        // Write several pages
        for (path, content) in &[
            ("entities/rust.md", "# Rust"),
            ("entities/go.md", "# Go"),
            ("concepts/ownership.md", "# Ownership"),
        ] {
            server
                .write_page(Parameters(WritePageRequest {
                    path: path.to_string(),
                    content: content.to_string(),
                }))
                .await
                .unwrap();
        }

        // List all pages — should include the 3 written + index.md + log.md + CLAUDE.md
        let result = server
            .list_pages(Parameters(ListPagesRequest { directory: None }))
            .await;
        assert!(result.is_ok(), "list_pages failed: {:?}", result);
        let pages: Vec<String> = serde_json::from_str(&result.unwrap()).unwrap();
        assert!(
            pages.len() >= 3,
            "expected at least 3 pages, got {}",
            pages.len()
        );

        // Filter by entities subdirectory — should have exactly 2
        let result = server
            .list_pages(Parameters(ListPagesRequest {
                directory: Some("entities".to_string()),
            }))
            .await;
        assert!(result.is_ok());
        let entity_pages: Vec<String> = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(entity_pages.len(), 2, "expected 2 entity pages");
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let config_path = std::env::var("AI_WIKI_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("ai-wiki.toml"));

    let config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };
    config.validate()?;

    let queue = Queue::open(&config.paths.database_path)?;

    let reset_count = queue
        .reset_in_progress()
        .map_err(|e| anyhow::anyhow!("failed to reset in-progress items: {e}"))?;
    if reset_count > 0 {
        eprintln!("Reset {reset_count} in-progress item(s) back to queued.");
    }

    let wiki = Wiki::new(config.paths.wiki_dir.clone());
    wiki.init()?;

    let server = WikiServer {
        queue: std::sync::Arc::new(Mutex::new(queue)),
        wiki: std::sync::Arc::new(wiki),
        config: std::sync::Arc::new(config),
        tool_router: WikiServer::tool_router(),
    };

    let transport = stdio();
    let server_handle = server.serve(transport).await?;
    server_handle.waiting().await?;
    Ok(())
}
