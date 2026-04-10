use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Context;

use ai_wiki_core::config::Config;
use ai_wiki_core::queue::Queue;

/// Characters that are unsafe to interpolate into shell commands or SQL.
const UNSAFE_PATH_CHARS: &[char] = &['\'', '"', '`', ';', '|', '&', '$', '\\', '\n', '\r'];

/// Validate that a path string contains no shell/SQL metacharacters.
fn validate_path_for_prompt(path: &str, label: &str) -> anyhow::Result<()> {
    if let Some(c) = path.chars().find(|c| UNSAFE_PATH_CHARS.contains(c)) {
        anyhow::bail!(
            "{label} path contains unsafe character {c:?}: {path}\n\
             Rename the path to remove shell/SQL metacharacters."
        );
    }
    Ok(())
}

pub fn run(config: &Config) -> anyhow::Result<()> {
    // Validate paths before embedding them in the prompt
    let db_path = config.paths.database_path.display().to_string();
    let wiki_dir = config.paths.wiki_dir.display().to_string();
    let processed_dir = config.paths.processed_dir.display().to_string();

    validate_path_for_prompt(&db_path, "Database")?;
    validate_path_for_prompt(&wiki_dir, "Wiki directory")?;
    validate_path_for_prompt(&processed_dir, "Processed text directory")?;

    let queue = Queue::open(&config.paths.database_path).with_context(|| {
        format!(
            "failed to open queue database at {}",
            config.paths.database_path.display()
        )
    })?;

    let counts = queue.count_by_status()?;
    let queued_count: u64 = counts
        .iter()
        .find(|(name, _)| name == "queued")
        .map(|(_, n)| *n)
        .unwrap_or(0);

    if queued_count == 0 {
        println!("No queued items to process.");
        return Ok(());
    }

    let total = queued_count as usize;
    println!("Queue has {total} item(s). Processing all.");

    let prompt = build_prompt(config, total);

    eprintln!("WARNING: This will grant the Claude CLI permission to run commands on your system.");
    eprintln!("Source documents may contain prompt injection attacks that could lead to");
    eprintln!("arbitrary command execution. Only proceed if you trust all queued sources.");
    eprintln!();

    println!("Launching Claude to process the queue...");
    println!();

    let mut child = Command::new("claude")
        .arg("--print")
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

    // Show summary after Claude finishes
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
        "Queue status: {} complete, {} queued, {} error, {} rejected",
        get("complete"),
        get("queued"),
        get("error"),
        get("rejected"),
    );

    Ok(())
}

fn build_prompt(config: &Config, total: usize) -> String {
    format!(
        r#"You are processing source documents from an ai-wiki queue into an Obsidian wiki.

## Setup

- **Wiki directory:** {wiki_dir}
- **Processed text directory:** {processed_dir}
- **Total items:** {total}
- **CLI binary:** `ai-wiki --config {config_path}`

## Instructions

Process queued items one at a time using the `ai-wiki queue` subcommands. For each item:

1. **Claim the next item:**
   ```bash
   ai-wiki --config {config_path} queue claim
   ```
   This atomically claims the oldest queued item and prints its details as:
   `<ID>|<file_path>|<file_type>|<parent_id_or_none>`
   If the output is `EMPTY`, the queue is exhausted — stop processing.

2. **Read the processed text** from `{processed_dir}/<ID>.txt`. If the file doesn't exist:
   - If the item has children (it's a book parent), read the children's processed text instead.
   - If no text is available, mark as error and move to the next item:
     ```bash
     ai-wiki --config {config_path} queue error <ID> "No processed text available"
     ```

3. **Extract knowledge** from the text:
   - Key entities (people, organizations, tools)
   - Important concepts
   - Specific claims or data points

4. **Create wiki pages** in `{wiki_dir}/`:
   - `sources/<slug>.md` — source summary with YAML frontmatter and [[wikilinks]]
   - `entities/<slug>.md` — for people/organizations (check existing files first!)
   - `concepts/<slug>.md` — for important concepts (check existing files first!)
   - `claims/<slug>.md` — for specific data points (add `data-point: true` to frontmatter)

   Every page must have YAML frontmatter:
   ```yaml
   ---
   type: source | entity | concept | claim
   tags: [relevant, tags]
   sources: [original-filename.pdf]
   created: {today}
   updated: {today}
   ---
   ```

5. **Update the index** — append new entries to `{wiki_dir}/index.md` under the correct ## heading.

6. **Update the log** — append `## [{today}] ingest | Title` to `{wiki_dir}/log.md`.

7. **Mark complete:**
   ```bash
   ai-wiki --config {config_path} queue complete <ID> "sources/<slug>.md"
   ```
   For book parents, also mark all children complete with the same wiki_page_path.

8. **Print progress** for each item:
   ```
   [N/{total}] <filename> ... done (created X pages)
   ```

9. **Repeat** until the queue is empty.

## Important Rules

- Check `{wiki_dir}/concepts/` and `{wiki_dir}/entities/` before creating pages to avoid duplicates.
- Book parents (items with children) should be summarized from their children's text.
- Use [[wikilinks]] for all cross-references between pages.
- Keep pages concise but informative.
- If an item has no processable text, mark it as error with a descriptive message.

Begin processing now.
"#,
        config_path = "ai-wiki.toml",
        wiki_dir = config.paths.wiki_dir.display(),
        processed_dir = config.paths.processed_dir.display(),
        total = total,
        today = chrono::Utc::now().format("%Y-%m-%d"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt_contains_expected_values() {
        let config = Config::default();
        let prompt = build_prompt(&config, 5);

        assert!(prompt.contains("ai-wiki --config ai-wiki.toml queue claim"));
        assert!(prompt.contains("ai-wiki --config ai-wiki.toml queue complete"));
        assert!(prompt.contains("ai-wiki --config ai-wiki.toml queue error"));
        assert!(prompt.contains("**Total items:** 5"));
        assert!(prompt.contains(&config.paths.wiki_dir.display().to_string()));
        assert!(prompt.contains(&config.paths.processed_dir.display().to_string()));
        // Verify date format YYYY-MM-DD appears
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        assert!(prompt.contains(&today));
        // Verify no raw sqlite3 commands
        assert!(!prompt.contains("sqlite3"));
    }

    #[test]
    fn test_validate_path_rejects_unsafe_chars() {
        assert!(validate_path_for_prompt("/normal/path", "test").is_ok());
        assert!(validate_path_for_prompt("/path with spaces/ok", "test").is_ok());
        assert!(validate_path_for_prompt("/path'with/quote", "test").is_err());
        assert!(validate_path_for_prompt("/path;semicolon", "test").is_err());
        assert!(validate_path_for_prompt("/path`backtick", "test").is_err());
        assert!(validate_path_for_prompt("/path|pipe", "test").is_err());
        assert!(validate_path_for_prompt("/path$var", "test").is_err());
    }
}
