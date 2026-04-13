use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Context;

use ai_wiki_core::config::WikiConfig;
use ai_wiki_core::queue::{ItemStatus, Queue, QueueItem};

pub struct ProcessOptions {
    /// If true, pass ANTHROPIC_API_KEY to child process (pay-as-you-go).
    /// If false, strip it so Claude CLI uses the Pro subscription.
    pub use_api_key: bool,
    /// Optional model override (e.g., "sonnet", "opus").
    pub model: Option<String>,
}

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

pub fn run(wiki: &WikiConfig, opts: &ProcessOptions) -> anyhow::Result<()> {
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
    let queued_parents = queue.count_queued_parents().with_context(|| {
        "failed to count queued parents"
    })?;

    if queued_parents == 0 {
        println!("No queued items to process.");
        return Ok(());
    }

    println!("Queue has {queued_parents} item(s). Processing one item per session.");

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

        if let Err(e) = validate_path_for_prompt(&file_name, "Source filename") {
            queue
                .mark_error(item.id, &format!("Unsafe filename: {e:#}"))
                .context("failed to mark item error")?;
            eprintln!("[{processed}] {file_name:?} ... error (unsafe filename)");
            errors += 1;
            continue;
        }

        eprint!("[{processed}] {file_name} ... ");

        // Read processed text in Rust — embed in prompt to avoid file-read tool calls
        let texts = match gather_text(&item, &queue, wiki) {
            Ok(t) => t,
            Err(e) => {
                queue
                    .mark_error(item.id, &format!("Failed to read text: {e:#}"))
                    .context("failed to mark item error")?;
                eprintln!("error (read text): {e:#}");
                errors += 1;
                continue;
            }
        };
        if texts.is_empty() {
            queue
                .mark_error(item.id, "No processed text available")
                .context("failed to mark item error")?;
            eprintln!("error (no text)");
            errors += 1;
            continue;
        }

        let prompt = build_item_prompt(wiki, &item, &texts, &file_name);

        let item_start = std::time::Instant::now();
        match run_claude_session(&prompt, opts) {
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

/// Atomically claim the next queued parent item (skipping children).
fn claim_next_parent(queue: &Queue) -> anyhow::Result<Option<QueueItem>> {
    Ok(queue.claim_next_queued_parent()?)
}

/// Collect processed-text entries for an item and its children.
/// Only reads file contents when total size is below MAX_EMBED_SIZE;
/// otherwise returns entries with empty strings (the prompt builder
/// will provide file paths instead).
fn gather_text(
    item: &QueueItem,
    queue: &Queue,
    wiki: &WikiConfig,
) -> anyhow::Result<Vec<(i64, String)>> {
    let mut entries: Vec<(i64, std::path::PathBuf)> = Vec::new();
    let mut total_size: u64 = 0;

    let own_path = wiki.processed_text_path(item.id);
    if own_path.exists() {
        let size = std::fs::metadata(&own_path)
            .with_context(|| format!("failed to stat {}", own_path.display()))?
            .len();
        total_size += size;
        entries.push((item.id, own_path));
    }

    let children = queue
        .children_of(item.id)
        .with_context(|| format!("failed to query children of item {}", item.id))?;
    for child in &children {
        if matches!(child.status, ItemStatus::Rejected) {
            continue;
        }
        let child_path = wiki.processed_text_path(child.id);
        if child_path.exists() {
            let size = std::fs::metadata(&child_path)
                .with_context(|| format!("failed to stat {}", child_path.display()))?
                .len();
            total_size += size;
            entries.push((child.id, child_path));
        }
    }

    if total_size as usize <= MAX_EMBED_SIZE {
        entries
            .into_iter()
            .map(|(id, path)| {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                Ok((id, content))
            })
            .collect()
    } else {
        Ok(entries.into_iter().map(|(id, _)| (id, String::new())).collect())
    }
}

/// Spawn a fresh Claude session with the given prompt.
fn run_claude_session(prompt: &str, opts: &ProcessOptions) -> anyhow::Result<()> {
    let mut cmd = Command::new("claude");
    cmd.arg("--print")
        .arg("--verbose")
        .arg("--dangerously-skip-permissions")
        // Skip hooks, plugins, MCP servers, CLAUDE.md — none needed for wiki generation.
        .arg("--bare")
        // Don't persist sessions to disk — these are one-shot.
        .arg("--no-session-persistence")
        // Cap turns to prevent runaway sessions.
        .arg("--max-turns").arg("30");

    if let Some(ref model) = opts.model {
        cmd.arg("--model").arg(model);
    }

    // Strip ANTHROPIC_API_KEY so Claude CLI uses Pro subscription auth,
    // unless the user explicitly chose API key auth.
    if !opts.use_api_key {
        cmd.env_remove("ANTHROPIC_API_KEY");
    }

    let mut child = cmd
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

/// Maximum total text size (in bytes) to embed directly in the prompt.
/// Above this, provide file paths and let Claude read them to avoid
/// exhausting the context window. 500KB ≈ 125K tokens, leaving plenty
/// of room for instructions and output.
const MAX_EMBED_SIZE: usize = 500_000;

fn build_item_prompt(wiki: &WikiConfig, item: &QueueItem, texts: &[(i64, String)], file_name: &str) -> String {
    let wiki_dir = wiki.wiki_dir().display().to_string();
    let wiki_name = &wiki.name;
    let today = chrono::Utc::now().format("%Y-%m-%d");
    let item_id = item.id;
    let is_book = texts.len() > 1
        || (item.parent_id.is_none() && texts.iter().any(|(id, _)| *id != item.id));

    let total_text_size: usize = texts.iter().map(|(_, c)| c.len()).sum();

    // Embed text directly when small enough; otherwise provide file paths.
    // When gather_text detects oversized input it returns empty strings,
    // so also fall back to file paths when entries exist but contain no text.
    let has_inline_content = texts.iter().any(|(_, c)| !c.is_empty());
    let (source_text_section, needs_file_reads) = if texts.is_empty() {
        ("**Note:** No source text available.".to_string(), false)
    } else if has_inline_content && total_text_size <= MAX_EMBED_SIZE {
        let embedded: String = texts
            .iter()
            .enumerate()
            .map(|(i, (id, content))| {
                format!("### Part {} (ID {id})\n\n{content}", i + 1)
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        (embedded, false)
    } else {
        let file_list: String = texts
            .iter()
            .map(|(id, _)| format!("  - `{}`", wiki.processed_text_path(*id).display()))
            .collect::<Vec<_>>()
            .join("\n");
        (
            format!(
                "**Note:** Source text is too large to include inline ({:.1}MB, {} parts).\n\
                 Read these files to get the source text:\n{file_list}",
                total_text_size as f64 / 1_000_000.0,
                texts.len(),
            ),
            true,
        )
    };

    // Build list of children IDs for the mark-complete step
    let child_ids: Vec<i64> = texts
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

    let book_hint = if is_book && needs_file_reads {
        "This is a **book with multiple chapters**. Read all the files listed below."
    } else if is_book {
        "This is a **book with multiple chapters**. The full text is provided below."
    } else if needs_file_reads {
        "This is a single document. Read the file listed below."
    } else {
        "This is a single document. The full text is provided below."
    };

    format!(
        r#"You are building an Obsidian wiki from a source document. This is a two-pass process.

## Context

- **Wiki:** {wiki_name}
- **Wiki directory:** {wiki_dir}
- **Source file:** {file_name}
- **Item ID:** {item_id}
- {book_hint}

## Source Text

{source_text_section}

## Pass 1: Source Summary

Create a source summary page from the source text.

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

Now extract knowledge from the source text into dedicated pages.

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
        source_text_section = source_text_section,
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
        let texts = vec![(42, "Sample extracted text from a PDF source document.".to_string())];
        let prompt = build_item_prompt(&wiki, &item, &texts, "source.pdf");

        assert!(prompt.contains("Item ID:** 42"));
        assert!(prompt.contains("source.pdf"));
        assert!(prompt.contains("Pass 1: Source Summary"));
        assert!(prompt.contains("Pass 2: Concept and Claim Extraction"));
        assert!(prompt.contains("at least 5 key concepts"));
        assert!(prompt.contains("queue complete 42"));
        assert!(prompt.contains("Sample extracted text"));
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
        let texts = vec![
            (11, "Chapter 1 content.".to_string()),
            (12, "Chapter 2 content.".to_string()),
        ];
        let prompt = build_item_prompt(&wiki, &item, &texts, "book.pdf");

        assert!(prompt.contains("queue complete 10"));
        assert!(prompt.contains("queue complete 11"));
        assert!(prompt.contains("queue complete 12"));
        assert!(prompt.contains("book with multiple chapters"));
        assert!(prompt.contains("Chapter 1 content."));
        assert!(prompt.contains("Chapter 2 content."));
    }

    #[test]
    fn test_build_item_prompt_large_input_uses_file_paths() {
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
        let big_text = "x".repeat(MAX_EMBED_SIZE + 1);
        let texts = vec![
            (11, big_text.clone()),
            (12, "Chapter 2 content.".to_string()),
        ];
        let prompt = build_item_prompt(&wiki, &item, &texts, "book.pdf");

        let expected_path_11 = wiki.processed_text_path(11).display().to_string();
        let expected_path_12 = wiki.processed_text_path(12).display().to_string();
        assert!(prompt.contains(&expected_path_11), "prompt should list processed path for id 11");
        assert!(prompt.contains(&expected_path_12), "prompt should list processed path for id 12");
        assert!(!prompt.contains(&big_text), "prompt should not embed the large text inline");
        assert!(prompt.contains("too large to include inline"));
    }

    #[test]
    fn test_build_item_prompt_empty_strings_use_file_paths() {
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
        // Simulate what gather_text returns for oversized input: entries with empty strings
        let texts = vec![
            (11, String::new()),
            (12, String::new()),
        ];
        let prompt = build_item_prompt(&wiki, &item, &texts, "book.pdf");

        let expected_path_11 = wiki.processed_text_path(11).display().to_string();
        let expected_path_12 = wiki.processed_text_path(12).display().to_string();
        assert!(prompt.contains(&expected_path_11), "prompt should list processed path for id 11");
        assert!(prompt.contains(&expected_path_12), "prompt should list processed path for id 12");
        assert!(prompt.contains("too large to include inline"), "prompt should indicate file-path fallback");
        assert!(prompt.contains("Read these files"), "prompt should instruct Claude to read files");
        assert!(!prompt.contains("### Part 1"), "prompt should not contain inline embed markers");
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
