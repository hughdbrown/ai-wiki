use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

use ai_wiki_core::config::Config;
use ai_wiki_core::preprocessing::pdf::PdfClassification;
use ai_wiki_core::preprocessing::{
    FileClassification, classify_pdf, detect_file_type, extract_audio, extract_pdf_text,
    extract_zip, split_pdf_chapters, transcribe_audio,
};
use ai_wiki_core::queue::{FileType, Queue};

// ─── Result accumulator ──────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct IngestResult {
    queued: usize,
    rejected: usize,
    errors: usize,
}

impl IngestResult {
    fn merge(&mut self, other: IngestResult) {
        self.queued += other.queued;
        self.rejected += other.rejected;
        self.errors += other.errors;
    }
}

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn run(config: &Config, path_str: &str) -> anyhow::Result<()> {
    let queue = Queue::open(&config.paths.database_path).with_context(|| {
        format!(
            "failed to open queue database at {}",
            config.paths.database_path.display()
        )
    })?;

    let reset_count = queue
        .reset_in_progress()
        .context("failed to reset in-progress items")?;
    if reset_count > 0 {
        eprintln!("Reset {reset_count} in-progress item(s) back to queued.");
    }

    let files = resolve_files(path_str)?;
    eprintln!("Resolved {} file(s) to ingest.", files.len());

    let mut totals = IngestResult::default();

    for file in &files {
        match process_file(file, config, &queue, None, 0) {
            Ok(result) => totals.merge(result),
            Err(e) => {
                eprintln!("Error processing {}: {e:#}", file.display());
                totals.errors += 1;
            }
        }
    }

    println!(
        "Ingest complete — queued: {}, rejected: {}, errors: {}",
        totals.queued, totals.rejected, totals.errors
    );

    Ok(())
}

// ─── Per-file processing ──────────────────────────────────────────────────────

const MAX_RECURSION_DEPTH: usize = 3;

fn process_file(
    path: &Path,
    config: &Config,
    queue: &Queue,
    parent_id: Option<i64>,
    depth: usize,
) -> anyhow::Result<IngestResult> {
    if depth > MAX_RECURSION_DEPTH {
        anyhow::bail!(
            "maximum nesting depth ({MAX_RECURSION_DEPTH}) exceeded for {}",
            path.display()
        );
    }

    let mut result = IngestResult::default();

    // Skip files that have already been enqueued
    if queue.is_already_enqueued(path, parent_id)? {
        eprintln!("Skipping (already ingested): {}", path.display());
        return Ok(result);
    }

    match detect_file_type(path, config) {
        FileClassification::Rejected(reason) => {
            let id = queue
                .enqueue(path, FileType::Unknown, parent_id)
                .context("failed to enqueue rejected file")?;
            queue
                .mark_rejected(id, &reason)
                .context("failed to mark item rejected")?;
            result.rejected += 1;
        }

        FileClassification::Ingestable(FileType::Zip) => {
            let id = queue
                .enqueue(path, FileType::Zip, parent_id)
                .context("failed to enqueue zip file")?;
            result.queued += 1;

            let extract_dir = config.paths.raw_dir.join(format!("zip_{id}"));
            match extract_zip(path, &extract_dir) {
                Ok(extracted) => {
                    for child_path in &extracted {
                        match process_file(child_path, config, queue, Some(id), depth + 1) {
                            Ok(child_result) => result.merge(child_result),
                            Err(e) => {
                                eprintln!(
                                    "Error processing zip child {}: {e:#}",
                                    child_path.display()
                                );
                                result.errors += 1;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to extract zip {}: {e:#}", path.display());
                    queue
                        .mark_error(id, &format!("{e:#}"))
                        .context("failed to mark zip error")?;
                    result.queued = result.queued.saturating_sub(1);
                    result.errors += 1;
                }
            }
        }

        FileClassification::Ingestable(FileType::Pdf) => {
            let id = queue
                .enqueue(path, FileType::Pdf, parent_id)
                .context("failed to enqueue PDF file")?;
            result.queued += 1;

            match classify_pdf(path, config) {
                Ok(PdfClassification::Book { .. }) => {
                    let chapter_dir = config.paths.raw_dir.join(format!("pdf_{id}_chapters"));
                    match split_pdf_chapters(path, &chapter_dir, config) {
                        Ok(chapters) => {
                            for chapter_path in &chapters {
                                let chapter_id = queue
                                    .enqueue(chapter_path, FileType::Pdf, Some(id))
                                    .context("failed to enqueue PDF chapter")?;
                                result.queued += 1;
                                if let Err(e) =
                                    extract_and_store_text(chapter_path, chapter_id, config)
                                {
                                    eprintln!(
                                        "Failed to extract text for chapter {}: {e:#}",
                                        chapter_path.display()
                                    );
                                    queue
                                        .mark_error(chapter_id, &format!("{e:#}"))
                                        .context("failed to mark chapter error")?;
                                    result.errors += 1;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to split PDF chapters for {}: {e:#}", path.display());
                            queue
                                .mark_error(id, &format!("{e:#}"))
                                .context("failed to mark PDF chapter-split error")?;
                            result.queued = result.queued.saturating_sub(1);
                            result.errors += 1;
                        }
                    }
                }

                Ok(PdfClassification::Simple) => {
                    if let Err(e) = extract_and_store_text(path, id, config) {
                        eprintln!("Failed to extract text for {}: {e:#}", path.display());
                        queue
                            .mark_error(id, &format!("{e:#}"))
                            .context("failed to mark PDF text-extraction error")?;
                        result.errors += 1;
                    }
                }

                Err(e) => {
                    eprintln!("Failed to classify PDF {}: {e:#}", path.display());
                    queue
                        .mark_error(id, &format!("{e:#}"))
                        .context("failed to mark PDF classification error")?;
                    result.queued = result.queued.saturating_sub(1);
                    result.errors += 1;
                }
            }
        }

        FileClassification::Ingestable(file_type @ (FileType::Markdown | FileType::Text)) => {
            let id = queue
                .enqueue(path, file_type, parent_id)
                .context("failed to enqueue markdown/text file")?;
            result.queued += 1;

            let dest = config.paths.processed_dir.join(format!("{id}.txt"));
            if let Err(e) = copy_to_processed(path, &dest) {
                eprintln!("Failed to copy {} to processed dir: {e:#}", path.display());
                queue
                    .mark_error(id, &format!("{e:#}"))
                    .context("failed to mark copy error")?;
                result.errors += 1;
            }
        }

        FileClassification::Ingestable(file_type @ (FileType::Audio | FileType::Video)) => {
            let id = queue
                .enqueue(path, file_type.clone(), parent_id)
                .context("failed to enqueue audio/video file")?;
            result.queued += 1;

            if let Err(e) = transcribe_source(path, id, &file_type, config) {
                eprintln!("Failed to transcribe {}: {e:#}", path.display());
                queue
                    .mark_error(id, &format!("{e:#}"))
                    .context("failed to mark transcription error")?;
                result.errors += 1;
            }
        }

        FileClassification::Ingestable(FileType::Unknown) => {
            let id = queue
                .enqueue(path, FileType::Unknown, parent_id)
                .context("failed to enqueue unknown file")?;
            queue
                .mark_rejected(id, "unknown file type")
                .context("failed to mark unknown file rejected")?;
            result.rejected += 1;
        }
    }

    Ok(result)
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn extract_and_store_text(path: &Path, item_id: i64, config: &Config) -> anyhow::Result<()> {
    let text = extract_pdf_text(path, config)
        .with_context(|| format!("extract_pdf_text failed for {}", path.display()))?;

    let dest = config.paths.processed_dir.join(format!("{item_id}.txt"));
    fs::create_dir_all(&config.paths.processed_dir).with_context(|| {
        format!(
            "failed to create processed dir: {}",
            config.paths.processed_dir.display()
        )
    })?;
    fs::write(&dest, text)
        .with_context(|| format!("failed to write extracted text to {}", dest.display()))?;

    Ok(())
}

fn transcribe_source(
    path: &Path,
    item_id: i64,
    file_type: &FileType,
    config: &Config,
) -> anyhow::Result<()> {
    let audio_path: PathBuf;
    let audio_ref: &Path;

    if *file_type == FileType::Video {
        let audio_dir = config.paths.raw_dir.join(format!("audio_{item_id}"));
        let extracted = extract_audio(path, &audio_dir, config)
            .with_context(|| format!("failed to extract audio from video {}", path.display()))?;
        audio_path = extracted;
        audio_ref = &audio_path;
    } else {
        audio_ref = path;
    }

    let transcript = transcribe_audio(audio_ref, config)
        .with_context(|| format!("failed to transcribe {}", audio_ref.display()))?;

    let dest = config.paths.processed_dir.join(format!("{item_id}.txt"));
    fs::create_dir_all(&config.paths.processed_dir).with_context(|| {
        format!(
            "failed to create processed dir: {}",
            config.paths.processed_dir.display()
        )
    })?;
    fs::write(&dest, transcript)
        .with_context(|| format!("failed to write transcript to {}", dest.display()))?;

    Ok(())
}

fn copy_to_processed(src: &Path, dest: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create processed dir: {}", parent.display()))?;
    }
    fs::copy(src, dest)
        .with_context(|| format!("failed to copy {} to {}", src.display(), dest.display()))?;
    Ok(())
}

// ─── File resolution ──────────────────────────────────────────────────────────

fn resolve_files(path_str: &str) -> anyhow::Result<Vec<PathBuf>> {
    // @filename — read file list, one path per line
    if let Some(list_file) = path_str.strip_prefix('@') {
        return read_file_list(list_file);
    }

    let path = Path::new(path_str);

    if path.is_dir() {
        let mut files = Vec::new();
        walk_dir(path, &mut files, 0)?;
        if files.is_empty() {
            anyhow::bail!("directory is empty: {}", path.display());
        }
        return Ok(files);
    }

    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    // Try glob expansion
    let entries: Vec<PathBuf> = glob::glob(path_str)
        .with_context(|| format!("invalid glob pattern: {path_str}"))?
        .filter_map(|r| match r {
            Ok(p) if p.is_file() => Some(p),
            Ok(_) => None, // skip directories matched by glob
            Err(e) => {
                eprintln!("glob error: {e}");
                None
            }
        })
        .collect();

    if entries.is_empty() {
        anyhow::bail!("no files matched: {path_str}");
    }

    Ok(entries)
}

const MAX_WALK_DEPTH: usize = 50;

fn walk_dir(dir: &Path, files: &mut Vec<PathBuf>, depth: usize) -> anyhow::Result<()> {
    if depth > MAX_WALK_DEPTH {
        anyhow::bail!(
            "directory nesting exceeds {} levels: {}",
            MAX_WALK_DEPTH,
            dir.display()
        );
    }

    let entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read directory: {}", dir.display()))?;

    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to get file type for {}", path.display()))?;

        if file_type.is_dir() {
            walk_dir(&path, files, depth + 1)?;
        } else if file_type.is_file() {
            files.push(path);
        }
        // symlinks are silently skipped
    }

    Ok(())
}

/// Strip matching leading and trailing single or double quotes from a string.
fn strip_quotes(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Read a file list (one path per line). Blank lines and lines starting with `#` are skipped.
/// Leading/trailing quotes on each line are stripped.
/// Relative paths are resolved from the current working directory, not the list file's location.
fn read_file_list(list_path: &str) -> anyhow::Result<Vec<PathBuf>> {
    let content = fs::read_to_string(list_path)
        .with_context(|| format!("failed to read file list: {list_path}"))?;

    let files: Vec<PathBuf> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(strip_quotes)
        .map(PathBuf::from)
        .collect();

    if files.is_empty() {
        anyhow::bail!("file list is empty: {list_path}");
    }

    // Verify all listed files exist
    let mut missing = Vec::new();
    for f in &files {
        if !f.is_file() {
            missing.push(f.display().to_string());
        }
    }
    if !missing.is_empty() {
        anyhow::bail!(
            "{} file(s) from list not found: {}",
            missing.len(),
            missing.join(", ")
        );
    }

    Ok(files)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.md");
        fs::write(&file, "hello").unwrap();

        let resolved = resolve_files(file.to_str().unwrap()).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], file);
    }

    #[test]
    fn test_resolve_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();

        let mut resolved = resolve_files(dir.path().to_str().unwrap()).unwrap();
        resolved.sort();
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn test_resolve_glob() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("note.md"), "md content").unwrap();
        fs::write(dir.path().join("readme.txt"), "txt content").unwrap();

        let pattern = format!("{}/*.md", dir.path().to_str().unwrap());
        let resolved = resolve_files(&pattern).unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].extension().map_or(false, |e| e == "md"));
    }

    #[test]
    fn test_resolve_no_match_returns_error() {
        let result = resolve_files("/nonexistent/path/to/nothing.xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_file_list() {
        let dir = tempfile::tempdir().unwrap();
        let file_a = dir.path().join("a.md");
        let file_b = dir.path().join("b.txt");
        fs::write(&file_a, "a").unwrap();
        fs::write(&file_b, "b").unwrap();

        let list_file = dir.path().join("files.txt");
        fs::write(
            &list_file,
            format!(
                "{}\n# comment line\n{}\n\n",
                file_a.display(),
                file_b.display()
            ),
        )
        .unwrap();

        let input = format!("@{}", list_file.display());
        let resolved = resolve_files(&input).unwrap();
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn test_resolve_file_list_missing_entry_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let list_file = dir.path().join("files.txt");
        fs::write(&list_file, "/nonexistent/file.md\n").unwrap();

        let input = format!("@{}", list_file.display());
        let result = resolve_files(&input);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not found"));
    }

    #[test]
    fn test_resolve_file_list_quoted_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file_a = dir.path().join("a.md");
        let file_b = dir.path().join("b.txt");
        let file_c = dir.path().join("c.rs");
        fs::write(&file_a, "a").unwrap();
        fs::write(&file_b, "b").unwrap();
        fs::write(&file_c, "c").unwrap();

        let list_file = dir.path().join("files.txt");
        fs::write(
            &list_file,
            format!(
                "\"{}\"\n'{}'\n{}\n",
                file_a.display(),
                file_b.display(),
                file_c.display(),
            ),
        )
        .unwrap();

        let input = format!("@{}", list_file.display());
        let resolved = resolve_files(&input).unwrap();
        assert_eq!(resolved.len(), 3);
    }

    #[test]
    fn test_strip_quotes_fn() {
        assert_eq!(strip_quotes(r#""hello""#), "hello");
        assert_eq!(strip_quotes("'hello'"), "hello");
        assert_eq!(strip_quotes("hello"), "hello");
        assert_eq!(strip_quotes(r#""hello'"#), r#""hello'"#); // mismatched, no strip
        assert_eq!(strip_quotes(""), "");
        assert_eq!(strip_quotes(r#""""#), ""); // empty quoted string
        assert_eq!(strip_quotes("\""), "\""); // single char, no panic
        assert_eq!(strip_quotes("'"), "'"); // single char, no panic
    }

    #[test]
    fn test_resolve_file_list_empty_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let list_file = dir.path().join("empty.txt");
        fs::write(&list_file, "# only comments\n\n").unwrap();

        let input = format!("@{}", list_file.display());
        let result = resolve_files(&input);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("empty"));
    }
}
