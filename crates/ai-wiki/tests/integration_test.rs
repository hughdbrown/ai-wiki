//! End-to-end integration tests for the ai-wiki pipeline.
//!
//! These tests invoke `cargo run -p ai-wiki` for the ingest step, then use the
//! ai-wiki-core library directly to verify queue state and exercise the wiki API.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ai_wiki_core::queue::{ItemStatus, Queue};
use ai_wiki_core::wiki::Wiki;

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Creates a temp environment with config, raw, wiki, processed, and db paths.
/// Returns (TempDir, config_path, raw_dir, wiki_dir, db_path).
/// The TempDir must be kept alive for the duration of the test.
fn setup_test_env() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let raw_dir = tmp.path().join("raw");
    let wiki_dir = tmp.path().join("wiki");
    let processed_dir = tmp.path().join("processed");
    let db_path = tmp.path().join("test.db");
    let config_path = tmp.path().join("config.toml");

    fs::create_dir_all(&raw_dir).expect("failed to create raw_dir");

    // Use display() with forward slashes for TOML — on macOS/Linux this is fine.
    let config = format!(
        r#"[paths]
raw_dir = "{raw}"
wiki_dir = "{wiki}"
database_path = "{db}"
processed_dir = "{processed}"

[pdf]
book_min_pages = 50

[rejection]
non_operative_extensions = [".dmg"]
sensitive_filename_patterns = ["divorce", "court", "bank.statement", "tax.return", "report.card", "financial", "lease"]

[tools]
qpdf_path = "qpdf"
pdftotext_path = "pdftotext"
pdftoppm_path = "pdftoppm"
tesseract_path = "tesseract"
ffmpeg_path = "ffmpeg"
whisper_cpp_path = "whisper-cpp"
whisper_model_path = "models/ggml-large-v3.bin"
"#,
        raw = raw_dir.display(),
        wiki = wiki_dir.display(),
        db = db_path.display(),
        processed = processed_dir.display(),
    );

    fs::write(&config_path, config).expect("failed to write config.toml");

    (tmp, config_path, raw_dir, wiki_dir, db_path)
}

/// Run `cargo run -p ai-wiki -- --config <config> ingest <target>` and return the output.
fn run_ingest(config_path: &Path, target: &str) -> Output {
    Command::new("cargo")
        .args([
            "run",
            "-p",
            "ai-wiki",
            "--",
            "--config",
            config_path.to_str().expect("config path is not UTF-8"),
            "ingest",
            target,
        ])
        .output()
        .expect("failed to run `cargo run -p ai-wiki`")
}

// ─── Test 1: ingest markdown files end-to-end ─────────────────────────────────

#[test]
fn test_ingest_markdown_files_end_to_end() {
    let (_tmp, config_path, raw_dir, wiki_dir, db_path) = setup_test_env();

    // Create 3 markdown files in raw_dir.
    let files = ["alpha.md", "beta.md", "gamma.md"];
    for name in &files {
        fs::write(raw_dir.join(name), format!("# {name}\n\nContent of {name}."))
            .expect("failed to write md file");
    }

    // Run ingest on the raw directory.
    let output = run_ingest(&config_path, raw_dir.to_str().unwrap());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "ingest failed.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Verify the summary line reports 3 queued items.
    assert!(
        stdout.contains("queued: 3"),
        "expected 'queued: 3' in stdout, got:\n{stdout}"
    );

    // Open the queue and verify 3 items are present with status Queued.
    let queue = Queue::open(&db_path).expect("failed to open queue db");
    let queued = queue
        .list_items(Some(&ItemStatus::Queued))
        .expect("failed to list queued items");
    assert_eq!(queued.len(), 3, "expected 3 queued items, got {}", queued.len());

    // Verify processed/{id}.txt files exist and have correct content.
    for item in &queued {
        let processed_path = config_path
            .parent()
            .unwrap()
            .join("processed")
            .join(format!("{}.txt", item.id));
        assert!(
            processed_path.exists(),
            "expected processed file {} to exist",
            processed_path.display()
        );
        let content = fs::read_to_string(&processed_path).expect("failed to read processed file");
        let source_name = item
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        assert!(
            content.contains(source_name.trim_end_matches(".md")),
            "processed file should contain source filename stem; got:\n{content}"
        );
    }

    // Verify the wiki directory was NOT auto-initialized by ingest (it doesn't do that),
    // but initialize it ourselves and verify the structure.
    let wiki = Wiki::new(wiki_dir.clone());
    wiki.init().expect("failed to init wiki");
    assert!(wiki_dir.join("entities").is_dir(), "missing entities/");
    assert!(wiki_dir.join("concepts").is_dir(), "missing concepts/");
    assert!(wiki_dir.join("claims").is_dir(), "missing claims/");
    assert!(wiki_dir.join("sources").is_dir(), "missing sources/");
    assert!(wiki_dir.join("index.md").is_file(), "missing index.md");
    assert!(wiki_dir.join("log.md").is_file(), "missing log.md");
    assert!(wiki_dir.join("CLAUDE.md").is_file(), "missing CLAUDE.md");
}

// ─── Test 2: ingest rejects .dmg files ────────────────────────────────────────

#[test]
fn test_ingest_rejects_dmg_files() {
    let (_tmp, config_path, raw_dir, _wiki_dir, db_path) = setup_test_env();

    // Create one valid markdown file and one .dmg file.
    fs::write(raw_dir.join("notes.md"), "# Notes\n\nSome notes.").unwrap();
    fs::write(raw_dir.join("installer.dmg"), b"\x00\x01\x02dmg content").unwrap();

    let output = run_ingest(&config_path, raw_dir.to_str().unwrap());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "ingest failed.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Summary should report 1 queued, 1 rejected.
    assert!(
        stdout.contains("queued: 1"),
        "expected 'queued: 1' in stdout, got:\n{stdout}"
    );
    assert!(
        stdout.contains("rejected: 1"),
        "expected 'rejected: 1' in stdout, got:\n{stdout}"
    );

    // Open the queue and verify counts.
    let queue = Queue::open(&db_path).expect("failed to open queue db");

    let queued = queue
        .list_items(Some(&ItemStatus::Queued))
        .expect("list queued");
    assert_eq!(queued.len(), 1, "expected 1 queued item");

    let rejected = queue
        .list_items(Some(&ItemStatus::Rejected))
        .expect("list rejected");
    assert_eq!(rejected.len(), 1, "expected 1 rejected item");

    // Verify the rejection reason mentions the extension.
    let reason = rejected[0]
        .error_message
        .as_deref()
        .unwrap_or("");
    assert!(
        reason.contains(".dmg") || reason.contains("non-operative"),
        "unexpected rejection reason: {reason}"
    );
}

// ─── Test 3: dedup prevents double processing ─────────────────────────────────

#[test]
fn test_ingest_dedup_prevents_double_processing() {
    let (_tmp, config_path, raw_dir, _wiki_dir, db_path) = setup_test_env();

    // Create 2 markdown files.
    fs::write(raw_dir.join("doc1.md"), "# Doc 1").unwrap();
    fs::write(raw_dir.join("doc2.md"), "# Doc 2").unwrap();

    // First ingest — should queue 2.
    let output1 = run_ingest(&config_path, raw_dir.to_str().unwrap());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let stderr1 = String::from_utf8_lossy(&output1.stderr);
    assert!(
        output1.status.success(),
        "first ingest failed.\nstdout: {stdout1}\nstderr: {stderr1}"
    );
    assert!(
        stdout1.contains("queued: 2"),
        "first ingest: expected 'queued: 2', got:\n{stdout1}"
    );

    // Second ingest on the same directory — should skip both files.
    let output2 = run_ingest(&config_path, raw_dir.to_str().unwrap());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let stderr2 = String::from_utf8_lossy(&output2.stderr);
    assert!(
        output2.status.success(),
        "second ingest failed.\nstdout: {stdout2}\nstderr: {stderr2}"
    );

    // The second run should queue 0 new items.
    assert!(
        stdout2.contains("queued: 0"),
        "second ingest: expected 'queued: 0', got:\n{stdout2}"
    );

    // Stderr should mention "already ingested" for the skipped files.
    assert!(
        stderr2.contains("already ingested"),
        "expected 'already ingested' in stderr, got:\n{stderr2}"
    );

    // Total queue should still be 2 — no duplicates added.
    let queue = Queue::open(&db_path).expect("failed to open queue db");
    let all_items = queue.list_items(None).expect("list all items");
    assert_eq!(
        all_items.len(),
        2,
        "expected exactly 2 total items in queue after two ingests, got {}",
        all_items.len()
    );
}

// ─── Test 4: ingest from a file list ─────────────────────────────────────────

#[test]
fn test_ingest_file_list() {
    let (_tmp, config_path, raw_dir, _wiki_dir, db_path) = setup_test_env();

    // Create 2 files.
    let file_a = raw_dir.join("list_a.md");
    let file_b = raw_dir.join("list_b.md");
    fs::write(&file_a, "# File A\n\nContent A.").unwrap();
    fs::write(&file_b, "# File B\n\nContent B.").unwrap();

    // Write a file list referencing both.
    let list_file = config_path.parent().unwrap().join("files.txt");
    fs::write(
        &list_file,
        format!(
            "# List of files\n{}\n{}\n",
            file_a.display(),
            file_b.display()
        ),
    )
    .unwrap();

    // Run ingest with @list_file syntax.
    let at_arg = format!("@{}", list_file.display());
    let output = run_ingest(&config_path, &at_arg);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "file-list ingest failed.\nstdout: {stdout}\nstderr: {stderr}"
    );

    assert!(
        stdout.contains("queued: 2"),
        "expected 'queued: 2' in stdout, got:\n{stdout}"
    );

    // Verify 2 items were queued.
    let queue = Queue::open(&db_path).expect("failed to open queue db");
    let queued = queue
        .list_items(Some(&ItemStatus::Queued))
        .expect("list queued");
    assert_eq!(queued.len(), 2, "expected 2 queued items from file list");
}

// ─── Test 5: queue-to-wiki workflow ──────────────────────────────────────────

#[test]
fn test_queue_to_wiki_workflow() {
    let (_tmp, config_path, raw_dir, wiki_dir, db_path) = setup_test_env();
    let processed_dir = config_path.parent().unwrap().join("processed");

    // Create and ingest a markdown file.
    let source_content = "# Rust\n\nRust is a systems programming language focused on safety.";
    let source_file = raw_dir.join("rust.md");
    fs::write(&source_file, source_content).unwrap();

    let output = run_ingest(&config_path, source_file.to_str().unwrap());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "ingest failed.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Open queue and claim the next item.
    let queue = Queue::open(&db_path).expect("failed to open queue db");
    let item = queue
        .claim_next_queued()
        .expect("claim_next_queued failed")
        .expect("expected one queued item");

    // claim_next_queued returns the item snapshot from before the status update;
    // verify the actual in-progress status by re-reading from the database.
    let item_after_claim = queue.get_item(item.id).expect("get_item after claim failed");
    assert_eq!(
        item_after_claim.status,
        ItemStatus::InProgress,
        "claimed item should be InProgress in the database"
    );

    // Read the preprocessed text from processed/{id}.txt.
    let processed_path = processed_dir.join(format!("{}.txt", item.id));
    assert!(
        processed_path.exists(),
        "processed file {} should exist",
        processed_path.display()
    );
    let processed_text = fs::read_to_string(&processed_path).expect("read processed file");
    assert_eq!(
        processed_text, source_content,
        "processed text should match original source"
    );

    // Initialize wiki and write a page.
    let wiki = Wiki::new(wiki_dir.clone());
    wiki.init().expect("wiki init failed");

    let page_content = "---\ntype: entity\ntags: [rust, programming]\ncreated: 2026-04-09\n---\n\n# Rust\n\nA systems programming language.";
    let page_path = "entities/rust.md";
    wiki.write_page(page_path, page_content)
        .expect("write_page failed");

    // Update index.
    wiki.update_index("- [[entities/rust]] — Systems programming language")
        .expect("update_index failed");

    // Append to log.
    wiki.append_log("ingest | Rust — ingested entities/rust.md")
        .expect("append_log failed");

    // Mark the queue item complete.
    queue
        .mark_complete(item.id, page_path)
        .expect("mark_complete failed");

    // ── Verify ────────────────────────────────────────────────────────────────

    // Wiki page exists with correct content.
    let read_back = wiki.read_page(page_path).expect("read_page failed");
    assert_eq!(
        read_back, page_content,
        "wiki page content should match what was written"
    );

    // index.md contains the new entry.
    let index = wiki.read_index().expect("read_index failed");
    assert!(
        index.contains("[[entities/rust]]"),
        "index.md should contain the new entity link"
    );
    assert!(
        index.contains("Systems programming language"),
        "index.md should contain the entry summary"
    );

    // log.md has the entry with a timestamp prefix.
    let log = wiki.read_page("log.md").expect("read log.md");
    assert!(
        log.contains("## ["),
        "log.md should have timestamp entries starting with '## ['"
    );
    assert!(
        log.contains("Rust"),
        "log.md should reference Rust"
    );

    // Queue item is now Complete with the wiki page path recorded.
    let completed_item = queue.get_item(item.id).expect("get_item failed");
    assert_eq!(
        completed_item.status,
        ItemStatus::Complete,
        "item should be Complete after mark_complete"
    );
    assert_eq!(
        completed_item.wiki_page_path.as_deref(),
        Some(page_path),
        "wiki_page_path should be recorded on the completed item"
    );
    assert!(
        completed_item.completed_at.is_some(),
        "completed_at should be set"
    );
}
