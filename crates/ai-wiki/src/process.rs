use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Context;

use ai_wiki_core::config::Config;
use ai_wiki_core::queue::{ItemStatus, Queue};

pub fn run(config: &Config, batch_size: usize) -> anyhow::Result<()> {
    let queue = Queue::open(&config.paths.database_path).with_context(|| {
        format!(
            "failed to open queue database at {}",
            config.paths.database_path.display()
        )
    })?;

    let queued_items = queue.list_items(Some(&ItemStatus::Queued))?;
    if queued_items.is_empty() {
        println!("No queued items to process.");
        return Ok(());
    }

    let to_process = queued_items.len().min(batch_size);
    println!(
        "Queue has {} item(s). Processing up to {to_process}.",
        queued_items.len()
    );

    let prompt = build_prompt(config, to_process);

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

fn build_prompt(config: &Config, batch_size: usize) -> String {
    format!(
        r#"You are processing source documents from an ai-wiki queue into an Obsidian wiki.

## Setup

- **Database:** {db_path}
- **Wiki directory:** {wiki_dir}
- **Processed text directory:** {processed_dir}
- **Batch size:** Process up to {batch_size} items.

## Instructions

Process queued items from the SQLite database one at a time. For each item:

1. **Claim the next item:**
   ```bash
   sqlite3 {db_path} "SELECT id, file_path, file_type, parent_id FROM queue_items WHERE status='queued' ORDER BY id ASC LIMIT 1;"
   ```
   Then mark it in-progress:
   ```bash
   sqlite3 {db_path} "UPDATE queue_items SET status='in_progress', started_at=datetime('now') WHERE id=<ID>;"
   ```

2. **Read the processed text** from `{processed_dir}/<ID>.txt`. If the file doesn't exist:
   - If the item has children (it's a book parent), read the children's processed text instead.
   - If no text is available, mark as error and move to the next item.

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
   sqlite3 {db_path} "UPDATE queue_items SET status='complete', wiki_page_path='sources/<slug>.md', completed_at=datetime('now') WHERE id=<ID>;"
   ```
   For book parents, also mark all children complete with the same wiki_page_path.

8. **Print progress** for each item:
   ```
   [N/{batch_size}] <filename> ... done (created X pages)
   ```

9. **Repeat** until you've processed {batch_size} items or the queue is empty.

## Important Rules

- Check `{wiki_dir}/concepts/` and `{wiki_dir}/entities/` before creating pages to avoid duplicates.
- Book parents (items with children) should be summarized from their children's text.
- Use [[wikilinks]] for all cross-references between pages.
- Keep pages concise but informative.
- If an item has no processable text, mark it as error with a descriptive message.

Begin processing now.
"#,
        db_path = config.paths.database_path.display(),
        wiki_dir = config.paths.wiki_dir.display(),
        processed_dir = config.paths.processed_dir.display(),
        batch_size = batch_size,
        today = chrono::Utc::now().format("%Y-%m-%d"),
    )
}
