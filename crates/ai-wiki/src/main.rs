mod ingest;
mod process;
mod tui;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use ai_wiki_core::config::{AppConfig, WikiConfig};

#[derive(Parser)]
#[command(
    name = "ai-wiki",
    version,
    about = "AI-powered wiki builder",
    long_about = "Build and maintain a personal knowledge base by having an LLM incrementally\n\
                  process your documents into an interlinked Obsidian wiki.\n\n\
                  Workflow:\n  \
                  1. ai-wiki ingest <sources>   — classify, extract text, queue for processing\n  \
                  2. ai-wiki process            — invoke Claude to build wiki pages\n  \
                  3. ai-wiki tui                — monitor queue status in a terminal UI\n\n\
                  See 'ai-wiki help <command>' for details on each command."
)]
struct Cli {
    /// Name of the wiki to operate on (overrides CWD detection)
    #[arg(long)]
    wiki: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new wiki and register it in the global config
    #[command(
        long_about = "Creates the directory structure for a new wiki and registers it in\n\
                      ~/.ai-wiki/config.toml. If no directory is given, uses the current directory.\n\
                      If no name is given, uses the directory's basename.\n\n\
                      Creates:\n  \
                      - wiki/            — Obsidian vault (entities/, concepts/, claims/, sources/)\n  \
                      - wiki/index.md    — page catalog\n  \
                      - wiki/log.md      — ingestion log\n  \
                      - wiki/CLAUDE.md   — LLM wiki schema\n  \
                      - processed/       — extracted text from source files\n  \
                      - raw/             — split PDFs and extracted archives\n  \
                      - ai-wiki.db       — SQLite queue database\n\n\
                      After init, you can ingest files and process them:\n  \
                      ai-wiki ingest ~/Downloads/*.pdf\n  \
                      ai-wiki process"
    )]
    Init {
        /// Name for this wiki (defaults to directory basename)
        #[arg(long)]
        name: Option<String>,

        /// Directory to initialize (defaults to current directory)
        directory: Option<PathBuf>,
    },

    /// List all registered wikis
    List,

    /// Ingest source files into the processing queue
    #[command(
        long_about = "Reads source files, classifies them by type, extracts text, and adds them\n\
                      to the processing queue. No LLM is involved — this is pure preprocessing.\n\n\
                      Supported file types:\n  \
                      - PDF: text extracted via pdf-extract, pdftotext, or OCR. Books (with\n    \
                        outlines and 50+ pages) are split into chapters automatically.\n  \
                      - Markdown/Text: copied directly to the processed directory.\n  \
                      - ZIP: extracted and each contained file processed recursively.\n  \
                      - Audio/Video: audio extracted with ffmpeg, transcribed with whisper-cpp.\n\n\
                      Duplicate files are detected and skipped automatically.\n\n\
                      Examples:\n  \
                      ai-wiki ingest ~/Downloads/paper.pdf\n  \
                      ai-wiki ingest ~/Downloads/rust-books/\n  \
                      ai-wiki ingest \"~/Downloads/*.pdf\"\n  \
                      ai-wiki ingest @my-reading-list.txt"
    )]
    Ingest {
        /// File path, directory, glob pattern, or @filelist to ingest
        ///
        /// Use @filename to read a list of files (one per line, # comments allowed,
        /// quoted paths are supported).
        path: String,
    },

    /// Launch the TUI to monitor queue status
    #[command(
        long_about = "Opens a terminal UI showing all queue items with color-coded status:\n  \
                      Gray = queued, Yellow = in progress, Green = complete, Red = error/rejected.\n\n\
                      Keyboard:\n  \
                      ↑/↓     Navigate items\n  \
                      Enter   View details (error message, rejection reason, or wiki page content)\n  \
                      R       Retry an errored/rejected item (requeue it)\n  \
                      r       Force refresh\n  \
                      q/Esc   Quit"
    )]
    Tui,

    /// Process all queued items using Claude to build wiki pages
    #[command(
        long_about = "Invokes the Claude CLI to process every queued item in the database.\n\
                      Claude reads each item's extracted text, identifies entities, concepts,\n\
                      and claims, creates wiki pages with YAML frontmatter and [[wikilinks]],\n\
                      updates the index and log, and marks items complete.\n\n\
                      Requires the 'claude' CLI to be installed and on PATH.\n\n\
                      Book parents (items with chapters) are summarized from their children's text."
    )]
    Process,

    /// Retry errored items that have processed text available
    #[command(
        long_about = "Requeues errored items that have extracted text in the processed directory,\n\
                      then runs 'process' to have Claude build their wiki pages.\n\n\
                      This is for items where text extraction succeeded but wiki page creation\n\
                      failed (e.g., Claude timeout, network error). Items without processed text\n\
                      are left as errors — use 'clear' to remove them, then re-ingest."
    )]
    Retry,

    /// Remove all errored items from the queue
    #[command(
        long_about = "Deletes all items with 'error' status from the queue database.\n\
                      Use this to clean up items that failed text extraction and cannot be\n\
                      retried without re-ingesting the original files.\n\n\
                      After clearing, you can re-ingest the original files:\n  \
                      ai-wiki clear\n  \
                      ai-wiki ingest ~/Downloads/*.pdf\n\n\
                      The dedup check will skip files that were already successfully processed\n\
                      and only pick up the ones that previously failed."
    )]
    Clear,

    /// Queue operations (used by the Claude process prompt)
    #[command(subcommand)]
    Queue(QueueCommands),
}

#[derive(Subcommand)]
enum QueueCommands {
    /// Atomically claim the next queued item and print its details as JSON
    Claim,

    /// Mark an in-progress item as complete
    Complete {
        /// Queue item ID
        id: i64,
        /// Path to the created wiki page (relative to wiki dir)
        wiki_page_path: String,
    },

    /// Mark an item as errored
    Error {
        /// Queue item ID
        id: i64,
        /// Error description
        message: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, directory } => init(name, directory),
        Commands::List => list(),
        _ => {
            let app_config = AppConfig::load()?;
            app_config.validate_tools()?;
            let wiki = app_config.resolve_wiki_auto(cli.wiki.as_deref())?;

            match cli.command {
                Commands::Ingest { path } => ingest::run(&app_config.tools, &wiki, &path),
                Commands::Tui => tui::run(&wiki),
                Commands::Process => process::run(&app_config.tools, &wiki),
                Commands::Retry => retry(&app_config, &wiki),
                Commands::Clear => clear(&wiki),
                Commands::Queue(cmd) => queue_cmd(&wiki, cmd),
                Commands::Init { .. } | Commands::List => unreachable!(),
            }
        }
    }
}

fn retry(app_config: &AppConfig, wiki: &WikiConfig) -> anyhow::Result<()> {
    let queue = ai_wiki_core::queue::Queue::open(&wiki.database_path())?;

    let error_items = queue.list_items(Some(&ai_wiki_core::queue::ItemStatus::Error))?;
    if error_items.is_empty() {
        println!("No errored items to retry.");
        return Ok(());
    }

    let retryable_ids: Vec<i64> = error_items
        .iter()
        .filter(|item| wiki.processed_text_path(item.id).exists())
        .map(|item| item.id)
        .collect();

    let retried = queue.requeue_items(&retryable_ids)?;
    let skipped = error_items.len().saturating_sub(retried);

    println!(
        "Retry: {retried} item(s) requeued, {skipped} skipped (no processed text or already changed)."
    );

    if retried > 0 {
        println!("Running process to build wiki pages...");
        println!();
        process::run(&app_config.tools, wiki)?;
    }

    Ok(())
}

fn clear(wiki: &WikiConfig) -> anyhow::Result<()> {
    let queue = ai_wiki_core::queue::Queue::open(&wiki.database_path())?;

    let error_items = queue.list_items(Some(&ai_wiki_core::queue::ItemStatus::Error))?;
    if error_items.is_empty() {
        println!("No errored items to clear.");
        return Ok(());
    }

    let (errors, children) = queue.delete_errors()?;

    let total = errors + children;
    if children > 0 {
        println!(
            "Cleared {total} item(s) from the queue ({errors} errored + {children} errored children)."
        );
    } else {
        println!("Cleared {errors} errored item(s) from the queue.");
    }
    println!("You can now re-ingest the original files:");
    println!("  ai-wiki ingest <path>");

    Ok(())
}

fn init(name: Option<String>, directory: Option<PathBuf>) -> anyhow::Result<()> {
    let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    let wiki_name = name.unwrap_or_else(|| {
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("wiki")
            .to_string()
    });

    // Load or create app config
    let mut app_config = AppConfig::load().unwrap_or_else(|_| AppConfig::default());

    if app_config.wikis.contains_key(&wiki_name) {
        anyhow::bail!("Wiki '{}' already registered", wiki_name);
    }

    // Create directory structure
    let wiki_config = WikiConfig {
        name: wiki_name.clone(),
        root: dir.clone(),
    };
    std::fs::create_dir_all(wiki_config.wiki_dir())?;
    std::fs::create_dir_all(wiki_config.processed_dir())?;
    std::fs::create_dir_all(wiki_config.raw_dir())?;

    // Init wiki vault
    let wiki = ai_wiki_core::wiki::Wiki::new(wiki_config.wiki_dir());
    wiki.init()?;

    // Create database
    let _queue = ai_wiki_core::queue::Queue::open(&wiki_config.database_path())?;

    // Register in config
    app_config.register_wiki(wiki_name.clone(), dir);
    app_config.save()?;

    // Print results
    println!(
        "Initialized wiki '{}' at {}",
        wiki_name,
        wiki_config.root.display()
    );
    println!("Created wiki/");
    println!("  wiki/entities/");
    println!("  wiki/concepts/");
    println!("  wiki/claims/");
    println!("  wiki/sources/");
    println!("  wiki/index.md");
    println!("  wiki/log.md");
    println!("  wiki/CLAUDE.md");
    println!("Created processed/");
    println!("Created raw/");
    println!("Created ai-wiki.db");

    println!();
    println!(
        "Initialized ai-wiki project in {}",
        wiki_config.root.display()
    );
    println!();
    println!("Next steps:");
    println!("  ai-wiki ingest ~/Downloads/*.pdf   # queue files for processing");
    println!("  ai-wiki process                    # invoke Claude to build wiki pages");
    println!("  ai-wiki tui                        # monitor queue status");

    Ok(())
}

fn list() -> anyhow::Result<()> {
    let app_config = AppConfig::load()?;
    if app_config.wikis.is_empty() {
        println!("No wikis registered. Run 'ai-wiki init' to create one.");
        return Ok(());
    }
    for (name, entry) in &app_config.wikis {
        println!("  {} — {}", name, entry.root.display());
    }
    Ok(())
}

fn queue_cmd(wiki: &WikiConfig, cmd: QueueCommands) -> anyhow::Result<()> {
    let queue = ai_wiki_core::queue::Queue::open(&wiki.database_path())?;

    match cmd {
        QueueCommands::Claim => match queue.claim_next_queued()? {
            Some(item) => {
                let file_path_str = item.file_path.display().to_string();
                if file_path_str.contains('\t') {
                    anyhow::bail!(
                        "Claimed file path contains a tab character, which would break \
                             the tab-delimited output format: {file_path_str:?}"
                    );
                }
                println!(
                    "{}\t{}\t{}\t{}",
                    item.id,
                    file_path_str,
                    item.file_type.as_str(),
                    item.parent_id
                        .map_or("none".to_owned(), |pid| pid.to_string()),
                );
            }
            None => {
                println!("EMPTY");
            }
        },
        QueueCommands::Complete { id, wiki_page_path } => {
            queue.mark_complete(id, &wiki_page_path)?;
            println!("Marked item {id} as complete.");
        }
        QueueCommands::Error { id, message } => {
            queue.mark_error(id, &message)?;
            println!("Marked item {id} as error.");
        }
    }

    Ok(())
}
