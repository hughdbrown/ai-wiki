use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Context;

use ai_wiki_core::config::WikiConfig;
use ai_wiki_core::queue::{ItemStatus, Queue, QueueItem};

/// Validate that a path string contains only safe characters for prompt embedding.
/// Permits alphanumerics, `.`, `_`, `/`, `-`, and space.
fn validate_path_for_prompt(path: &str, label: &str) -> anyhow::Result<()> {
    if let Some(c) = path
        .chars()
        .find(|c| !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '/' | '-' | ' '))
    {
        anyhow::bail!(
            "{label} contains unsafe character {c:?}: {path}\n\
             Rename the value to remove shell metacharacters."
        );
    }
    Ok(())
}

pub fn run(wiki: &WikiConfig) -> anyhow::Result<()> {
    let wiki_dir = wiki.wiki_dir().display().to_string();
    let processed_dir = wiki.processed_dir().display().to_string();

    validate_path_for_prompt(&wiki_dir, "Wiki directory")?;
    validate_path_for_prompt(&processed_dir, "Processed text directory")?;
    validate_path_for_prompt(&wiki.name, "Wiki name")?;

    let queue = Queue::open(&wiki.database_path()).with_context(|| {
        format!(
            "failed to open queue database at {}",
            wiki.database_path().display()
        )
    })?;

    // Count only top-level queued items (parent_id IS NULL).
    // Children are processed as part of their parent's session.
    let queued_parents = queue
        .list_items(Some(&ai_wiki_core::queue::ItemStatus::Queued))?
        .iter()
        .filter(|item| item.parent_id.is_none())
        .count();

    if queued_parents == 0 {
        println!("No queued items to process.");
        return Ok(());
    }

    let total = queued_parents;
    println!("Queue has {total} item(s). Processing one item per session.");

    eprintln!("WARNING: This will grant the Claude CLI permission to run commands on your system.");
    eprintln!("Source documents may contain prompt injection attacks that could lead to");
    eprintln!("arbitrary command execution. Only proceed if you trust all queued sources.");
    eprintln!();

    let mut processed = 0usize;
    let mut errors = 0usize;

    loop {
        // Claim the next top-level item (skip children, they're handled via their parent)
        let item = match claim_next_parent(&queue)? {
            Some(item) => item,
            None => break,
        };

        processed += 1;
        let file_name = item
            .file_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| item.file_path.to_string_lossy().into_owned());

        eprint!("[{processed}/{total}] {file_name} ... ");

        // Collect processed text paths for this item
        let text_paths = gather_text_paths(&item, &queue, wiki);
        if text_paths.is_empty() {
            queue
                .mark_error(item.id, "No processed text available")
                .context("failed to mark item error")?;
            eprintln!("error (no text)");
            errors += 1;
            continue;
        }

        let prompt = build_item_prompt(wiki, &item, &text_paths);

        let item_start = std::time::Instant::now();
        match run_claude_session(&prompt) {
            Ok(()) => {
                let elapsed = item_start.elapsed();
                // Verify Claude marked it complete; if not, mark error
                let updated = queue.get_item(item.id)?;
                if updated.status == ItemStatus::InProgress {
                    queue
                        .mark_error(item.id, "Claude session ended without marking item complete")
                        .context("failed to mark incomplete item")?;
                    eprintln!("error (not marked complete) ({:.1}s)", elapsed.as_secs_f64());
                    errors += 1;
                } else {
                    eprintln!("done ({:.1}s)", elapsed.as_secs_f64());
                }
            }
            Err(e) => {
                let elapsed = item_start.elapsed();
                queue
                    .mark_error(item.id, &format!("Claude session failed: {e:#}"))
                    .context("failed to mark session-error item")?;
                eprintln!("error ({:.1}s): {e:#}", elapsed.as_secs_f64());
                errors += 1;
            }
        }
    }

    println!();
    let counts = queue.count_by_status()?;
    let get = |s: &str| -> u64 {
        counts
            .iter()
            .find(|(name, _)| name == s)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    };
    println!(
        "Done. Processed {processed} item(s), {errors} error(s).\n\
         Queue status: {} complete, {} queued, {} error, {} rejected",
        get("complete"),
        get("queued"),
        get("error"),
        get("rejected"),
    );

    Ok(())
}

/// Claim the next queued item that has no parent (top-level item).
/// Children are processed as part of their parent's session.
fn claim_next_parent(queue: &Queue) -> anyhow::Result<Option<QueueItem>> {
    // Get all queued items sorted by creation time
    let queued = queue.list_items(Some(&ItemStatus::Queued))?;
    for item in queued {
        if item.parent_id.is_none() {
            queue.mark_in_progress(item.id)?;
            return Ok(Some(item));
        }
    }
    Ok(None)
}

/// Gather the processed text file paths for an item.
/// For a book parent, returns the children's text files.
/// For a leaf item, returns its own text file.
fn gather_text_paths(item: &QueueItem, queue: &Queue, wiki: &WikiConfig) -> Vec<(i64, String)> {
    let mut paths = Vec::new();

    // Check if this item has its own processed text
    let own_path = wiki.processed_text_path(item.id);
    if own_path.exists() {
        paths.push((
            item.id,
            own_path.display().to_string(),
        ));
    }

    // For book parents, also gather children's text
    if let Ok(children) = queue.children_of(item.id) {
        for child in &children {
            if matches!(child.status, ItemStatus::Rejected) {
                continue; // skip rejected front matter
            }
            let child_path = wiki.processed_text_path(child.id);
            if child_path.exists() {
                paths.push((
                    child.id,
                    child_path.display().to_string(),
                ));
            }
        }
    }

    paths
}

/// Spawn a fresh Claude session with the given prompt.
fn run_claude_session(prompt: &str) -> anyhow::Result<()> {
    let mut child = Command::new("claude")
        .arg("--print")
        .arg("--verbose")
        .arg("--dangerously-skip-permissions")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to launch 'claude' CLI — is it installed and on PATH?")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .context("failed to write prompt to claude stdin")?;
    }

    let status = child.wait().context("failed to wait for claude process")?;
    if !status.success() {
        anyhow::bail!("claude exited with status: {status}");
    }

    Ok(())
}

fn build_item_prompt(wiki: &WikiConfig, item: &QueueItem, text_paths: &[(i64, String)]) -> String {
    let wiki_dir = wiki.wiki_dir().display().to_string();
    let wiki_name = &wiki.name;
    let today = chrono::Utc::now().format("%Y-%m-%d");
    let item_id = item.id;

    let file_name = item
        .file_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| item.file_path.to_string_lossy().into_owned());

    let is_book = text_paths.len() > 1 || item.parent_id.is_none() && {
        // Check if it has children by seeing if any text_path id != item.id
        text_paths.iter().any(|(id, _)| *id != item.id)
    };

    let text_file_list: String = text_paths
        .iter()
        .map(|(id, path)| format!("  - ID {id}: `{path}`"))
        .collect::<Vec<_>>()
        .join("\n");

    // Build list of children IDs for the mark-complete step
    let child_ids: Vec<i64> = text_paths
        .iter()
        .filter(|(id, _)| *id != item.id)
        .map(|(id, _)| *id)
        .collect();

    let mark_children_complete = if child_ids.is_empty() {
        String::new()
    } else {
        let cmds: Vec<String> = child_ids
            .iter()
            .map(|id| {
                format!(
                    "ai-wiki --wiki '{wiki_name}' queue complete {id} \"sources/<slug>.md\""
                )
            })
            .collect();
        format!(
            "\n   Also mark all children complete:\n   ```bash\n   {}\n   ```",
            cmds.join("\n   ")
        )
    };

    let book_hint = if is_book {
        "This is a **book with multiple chapters**. Read ALL chapter files to build a comprehensive summary."
    } else {
        "This is a single document."
    };

    format!(
        r#"You are building an Obsidian wiki from a source document. This is a two-pass process.

## Context

- **Wiki:** {wiki_name}
- **Wiki directory:** {wiki_dir}
- **Source file:** {file_name}
- **Item ID:** {item_id}
- {book_hint}

### Processed text files to read:
{text_file_list}

## Pass 1: Source Summary

Read all the processed text files listed above, then create a source summary page.

1. **Create `{wiki_dir}/sources/<slug>.md`** with:
   - YAML frontmatter: type, tags, sources, created, updated
   - Title, author, publisher info
   - Chapter-by-chapter summary with key topics
   - Use `[[wikilinks]]` for cross-references to concepts and entities

   ```yaml
   ---
   type: source
   tags: [relevant, tags]
   sources: [{file_name}]
   created: {today}
   updated: {today}
   ---
   ```

2. **Create or update entity pages** in `{wiki_dir}/entities/`:
   - One page per author, publisher, or organization mentioned
   - Check existing files first: `ls {wiki_dir}/entities/`
   - If the entity page exists, append to it rather than overwriting

## Pass 2: Concept and Claim Extraction

Now re-read the source text and extract knowledge into dedicated pages.

3. **Create concept pages** in `{wiki_dir}/concepts/`:
   - Extract **at least 5 key concepts** from the source material
   - Each concept gets its own page: `{wiki_dir}/concepts/<concept-slug>.md`
   - Check existing files first: `ls {wiki_dir}/concepts/`
   - If a concept page already exists, **update it** by appending the new source's perspective
   - Each concept page must have:
     - YAML frontmatter with `type: concept`
     - A clear definition/explanation
     - `[[wikilinks]]` back to the source and to related concepts
     - A `## Sources` section listing which sources discuss this concept

   ```yaml
   ---
   type: concept
   tags: [relevant, tags]
   sources: [{file_name}]
   created: {today}
   updated: {today}
   ---
   ```

4. **Create claim pages** in `{wiki_dir}/claims/`:
   - Extract specific factual claims, theorems, formulas, or data points
   - Each claim gets its own page: `{wiki_dir}/claims/<claim-slug>.md`
   - Include `data-point: true` in frontmatter for quantitative claims
   - Link back to the source

5. **Update the index** — append new entries to `{wiki_dir}/index.md` under the correct `##` heading (Sources, Concepts, Entities, Claims).

6. **Update the log** — append `## [{today}] ingest | <title>` to `{wiki_dir}/log.md`.

7. **Mark complete:**
   ```bash
   ai-wiki --wiki '{wiki_name}' queue complete {item_id} "sources/<slug>.md"
   ```{mark_children_complete}

## Requirements

- You MUST create concept pages. Do not skip this step. Source summaries alone are not sufficient.
- Each concept page must be a standalone page in `{wiki_dir}/concepts/`, not just a wikilink in the source page.
- For math/science texts, concepts include: theorems, definitions, techniques, mathematical objects.
- For technical texts, concepts include: patterns, protocols, algorithms, architectures.
- Verify your work: after creating pages, run `ls {wiki_dir}/concepts/` to confirm they exist.
"#,
        wiki_name = wiki_name,
        wiki_dir = wiki_dir,
        file_name = file_name,
        item_id = item_id,
        text_file_list = text_file_list,
        book_hint = book_hint,
        today = today,
        mark_children_complete = mark_children_complete,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_item_prompt_contains_expected_values() {
        let wiki = WikiConfig {
            name: "test-wiki".to_string(),
            root: PathBuf::from("/tmp/test-wiki"),
        };
        let item = QueueItem {
            id: 42,
            file_path: PathBuf::from("/tmp/source.pdf"),
            file_type: ai_wiki_core::queue::FileType::Pdf,
            status: ItemStatus::InProgress,
            parent_id: None,
            wiki_page_path: None,
            error_message: None,
            created_at: chrono::Utc::now(),
            started_at: Some(chrono::Utc::now()),
            completed_at: None,
        };
        let text_paths = vec![(42, "/tmp/test-wiki/processed/42.txt".to_string())];
        let prompt = build_item_prompt(&wiki, &item, &text_paths);

        assert!(prompt.contains("Item ID:** 42"));
        assert!(prompt.contains("source.pdf"));
        assert!(prompt.contains("Pass 1: Source Summary"));
        assert!(prompt.contains("Pass 2: Concept and Claim Extraction"));
        assert!(prompt.contains("at least 5 key concepts"));
        assert!(prompt.contains("queue complete 42"));
        assert!(prompt.contains(&wiki.wiki_dir().display().to_string()));
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        assert!(prompt.contains(&today));
        assert!(!prompt.contains("sqlite3"));
    }

    #[test]
    fn test_build_item_prompt_includes_child_complete_commands() {
        let wiki = WikiConfig {
            name: "test-wiki".to_string(),
            root: PathBuf::from("/tmp/test-wiki"),
        };
        let item = QueueItem {
            id: 10,
            file_path: PathBuf::from("/tmp/book.pdf"),
            file_type: ai_wiki_core::queue::FileType::Pdf,
            status: ItemStatus::InProgress,
            parent_id: None,
            wiki_page_path: None,
            error_message: None,
            created_at: chrono::Utc::now(),
            started_at: Some(chrono::Utc::now()),
            completed_at: None,
        };
        let text_paths = vec![
            (11, "/tmp/test-wiki/processed/11.txt".to_string()),
            (12, "/tmp/test-wiki/processed/12.txt".to_string()),
        ];
        let prompt = build_item_prompt(&wiki, &item, &text_paths);

        assert!(prompt.contains("queue complete 10"));
        assert!(prompt.contains("queue complete 11"));
        assert!(prompt.contains("queue complete 12"));
        assert!(prompt.contains("book with multiple chapters"));
    }

    #[test]
    fn test_validate_path_rejects_single_quote_in_wiki_name() {
        assert!(validate_path_for_prompt("my'wiki", "Wiki name").is_err());
        assert!(validate_path_for_prompt("my-wiki", "Wiki name").is_ok());
        assert!(validate_path_for_prompt("my_wiki", "Wiki name").is_ok());
    }

    #[test]
    fn test_validate_path_rejects_unsafe_chars() {
        assert!(validate_path_for_prompt("/normal/path", "test").is_ok());
        assert!(validate_path_for_prompt("/path with spaces/ok", "test").is_ok());
        assert!(validate_path_for_prompt("/path-with_dots.toml", "test").is_ok());
        assert!(validate_path_for_prompt("/path'with/quote", "test").is_err());
        assert!(validate_path_for_prompt("/path;semicolon", "test").is_err());
        assert!(validate_path_for_prompt("/path`backtick", "test").is_err());
        assert!(validate_path_for_prompt("/path|pipe", "test").is_err());
        assert!(validate_path_for_prompt("/path$var", "test").is_err());
        assert!(validate_path_for_prompt("/path(parens)", "test").is_err());
        assert!(validate_path_for_prompt("/path{braces}", "test").is_err());
        assert!(validate_path_for_prompt("/path<angle>", "test").is_err());
        assert!(validate_path_for_prompt("/path!bang", "test").is_err());
        assert!(validate_path_for_prompt("/path#hash", "test").is_err());
    }
}
