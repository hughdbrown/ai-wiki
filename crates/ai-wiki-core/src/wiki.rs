use std::fs;
use std::path::{Path, PathBuf};

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
        for subdir in &["entities", "concepts", "claims", "sources"] {
            let dir = self.root.join(subdir);
            fs::create_dir_all(&dir).map_err(|e| {
                anyhow::anyhow!("Failed to create directory {}: {}", dir.display(), e)
            })?;
        }

        let index_path = self.root.join("index.md");
        if !index_path.exists() {
            fs::write(&index_path, Self::default_index())
                .map_err(|e| anyhow::anyhow!("Failed to write {}: {}", index_path.display(), e))?;
        }

        let log_path = self.root.join("log.md");
        if !log_path.exists() {
            fs::write(&log_path, "# Wiki Log\n")
                .map_err(|e| anyhow::anyhow!("Failed to write {}: {}", log_path.display(), e))?;
        }

        let claude_md_path = self.root.join("CLAUDE.md");
        if !claude_md_path.exists() {
            fs::write(&claude_md_path, Self::default_claude_md()).map_err(|e| {
                anyhow::anyhow!("Failed to write {}: {}", claude_md_path.display(), e)
            })?;
        }

        Ok(())
    }

    /// Resolve a relative path safely within the wiki root.
    /// Rejects paths containing `..` components to prevent path traversal.
    fn safe_resolve(&self, relative_path: &str) -> anyhow::Result<PathBuf> {
        let rel = Path::new(relative_path);
        for component in rel.components() {
            if matches!(component, std::path::Component::ParentDir) {
                anyhow::bail!("path traversal rejected: {}", relative_path);
            }
            if matches!(component, std::path::Component::RootDir) {
                anyhow::bail!("absolute path rejected: {}", relative_path);
            }
        }
        Ok(self.root.join(relative_path))
    }

    pub fn read_page(&self, relative_path: &str) -> anyhow::Result<String> {
        let path = self.safe_resolve(relative_path)?;
        fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))
    }

    pub fn write_page(&self, relative_path: &str, content: &str) -> anyhow::Result<()> {
        let path = self.safe_resolve(relative_path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!("Failed to create directory {}: {}", parent.display(), e)
            })?;
        }
        fs::write(&path, content)
            .map_err(|e| anyhow::anyhow!("Failed to write {}: {}", path.display(), e))
    }

    pub fn list_pages(&self, subdirectory: Option<&str>) -> anyhow::Result<Vec<String>> {
        let dir = match subdirectory {
            Some(sub) => self.safe_resolve(sub)?,
            None => self.root.clone(),
        };

        if !dir.exists() {
            return Ok(vec![]);
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
        let path = self.safe_resolve("index.md")?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", path.display(), e))?;
        use std::io::Write;
        writeln!(file, "{}", entry)
            .map_err(|e| anyhow::anyhow!("Failed to append to {}: {}", path.display(), e))?;
        Ok(())
    }

    pub fn append_log(&self, entry: &str) -> anyhow::Result<()> {
        let path = self.safe_resolve("log.md")?;
        let date = Utc::now().format("%Y-%m-%d");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", path.display(), e))?;
        use std::io::Write;
        writeln!(file, "## [{date}] {entry}")
            .map_err(|e| anyhow::anyhow!("Failed to append to {}: {}", path.display(), e))?;
        Ok(())
    }

    fn default_index() -> String {
        "# Wiki Index\n\n## Entities\n\n## Concepts\n\n## Claims\n\n## Sources\n".to_string()
    }

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
"#
        .to_string()
    }

    fn collect_md_files(dir: &Path, root: &Path, pages: &mut Vec<String>) -> anyhow::Result<()> {
        let entries = fs::read_dir(dir)
            .map_err(|e| anyhow::anyhow!("Failed to read directory {}: {}", dir.display(), e))?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                anyhow::anyhow!("Failed to read directory entry in {}: {}", dir.display(), e)
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|e| {
                anyhow::anyhow!("Failed to get file type in {}: {}", dir.display(), e)
            })?;

            if file_type.is_dir() {
                Self::collect_md_files(&path, root, pages)?;
            } else if file_type.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to strip prefix {} from {}: {}",
                            root.display(),
                            path.display(),
                            e
                        )
                    })?
                    .to_string_lossy()
                    .into_owned();
                pages.push(relative);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_init_creates_directories() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        assert!(dir.path().join("entities").is_dir());
        assert!(dir.path().join("concepts").is_dir());
        assert!(dir.path().join("claims").is_dir());
        assert!(dir.path().join("sources").is_dir());
        assert!(dir.path().join("index.md").is_file());
        assert!(dir.path().join("log.md").is_file());
    }

    #[test]
    fn test_write_and_read_page() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        let content = "# Rust\n\nA systems programming language.";
        wiki.write_page("entities/rust.md", content).unwrap();
        let read_back = wiki.read_page("entities/rust.md").unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn test_read_nonexistent_page_returns_error() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        let result = wiki.read_page("entities/nonexistent.md");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_pages() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        wiki.write_page("entities/rust.md", "# Rust").unwrap();
        wiki.write_page("concepts/ownership.md", "# Ownership")
            .unwrap();
        wiki.write_page("claims/safe.md", "# Safe").unwrap();

        let pages = wiki.list_pages(None).unwrap();
        // 3 written pages + index.md + log.md + CLAUDE.md = 6
        assert_eq!(pages.len(), 6);

        let entity_pages = wiki.list_pages(Some("entities")).unwrap();
        assert_eq!(entity_pages.len(), 1);
    }

    #[test]
    fn test_list_pages_empty_subdirectory() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        let pages = wiki.list_pages(Some("nonexistent_subdir")).unwrap();
        assert!(pages.is_empty());
    }

    #[test]
    fn test_read_index() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        let index = wiki.read_index().unwrap();
        assert!(index.contains("# Wiki Index"));
        assert!(index.contains("## Entities"));
        assert!(index.contains("## Concepts"));
        assert!(index.contains("## Claims"));
        assert!(index.contains("## Sources"));
    }

    #[test]
    fn test_update_index() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        wiki.update_index("- [[entities/rust]]").unwrap();
        let index = wiki.read_index().unwrap();
        assert!(index.contains("- [[entities/rust]]"));
    }

    #[test]
    fn test_append_log() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        wiki.append_log("Added Rust entity page").unwrap();
        let log = wiki.read_page("log.md").unwrap();
        assert!(log.contains("## ["));
        // Verify timestamp format "## [YYYY-MM-DD]"
        assert!(log.contains("Added Rust entity page"));
        // Check that a date pattern exists
        let re_check = log.lines().any(|line| {
            line.starts_with("## [")
                && line.len() > 8
                && line.chars().nth(7).map_or(false, |c| c.is_ascii_digit())
        });
        assert!(re_check, "Log entry should have ## [YYYY-MM-DD] format");
    }

    #[test]
    fn test_read_page_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        let result = wiki.read_page("../../etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("traversal"));
    }

    #[test]
    fn test_write_page_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        let result = wiki.write_page("../escape.md", "malicious");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_pages_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        let result = wiki.list_pages(Some("../../"));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_page_rejects_absolute_path() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        let result = wiki.read_page("/etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn test_init_is_idempotent() {
        let dir = tempdir().unwrap();
        let wiki = Wiki::new(dir.path().to_path_buf());
        wiki.init().unwrap();

        // Write a page and customize the index
        wiki.write_page("entities/rust.md", "# Rust").unwrap();
        wiki.update_index("- [[entities/rust]]").unwrap();

        // Re-init should not overwrite existing files
        wiki.init().unwrap();

        // Page should still exist
        let page = wiki.read_page("entities/rust.md").unwrap();
        assert_eq!(page, "# Rust");

        // Customized index should survive
        let index = wiki.read_index().unwrap();
        assert!(index.contains("- [[entities/rust]]"));
    }
}
