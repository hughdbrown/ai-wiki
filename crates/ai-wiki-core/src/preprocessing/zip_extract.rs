use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context;

const MAX_DECOMPRESSED_SIZE: u64 = 1_073_741_824; // 1 GB
const MAX_ENTRIES: usize = 10_000;

pub fn extract_zip(zip_path: &Path, output_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let file = fs::File::open(zip_path)
        .with_context(|| format!("failed to open zip file: {}", zip_path.display()))?;

    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read zip archive: {}", zip_path.display()))?;

    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output dir: {}", output_dir.display()))?;

    let mut extracted_paths = Vec::new();
    let mut total_bytes: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("failed to access zip entry {i}"))?;

        // Skip directories
        if entry.is_dir() {
            continue;
        }

        // Zip-slip protection: enclosed_name() strips all `..` components.
        // Preserve the relative directory structure under output_dir.
        let safe_path = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue, // skip entries with unsafe paths
        };
        let dest_path = output_dir.join(&safe_path);

        // Ensure parent directories exist (for nested zip entries)
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create parent directories for: {}",
                    dest_path.display()
                )
            })?;
        }

        let mut dest_file = fs::File::create(&dest_path)
            .with_context(|| format!("failed to create file: {}", dest_path.display()))?;

        let bytes_written = io::copy(&mut entry, &mut dest_file)
            .with_context(|| format!("failed to extract entry to: {}", dest_path.display()))?;

        total_bytes += bytes_written;
        if total_bytes > MAX_DECOMPRESSED_SIZE {
            anyhow::bail!(
                "ZIP extraction aborted: decompressed size exceeds {} bytes",
                MAX_DECOMPRESSED_SIZE
            );
        }

        extracted_paths.push(dest_path);

        if extracted_paths.len() >= MAX_ENTRIES {
            anyhow::bail!("ZIP extraction aborted: {} or more entries", MAX_ENTRIES);
        }
    }

    Ok(extracted_paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn make_test_zip(dir: &Path) -> PathBuf {
        let zip_path = dir.join("test.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);

        let options = SimpleFileOptions::default();
        writer.start_file("hello.txt", options).unwrap();
        writer.write_all(b"hello world").unwrap();
        writer.start_file("subdir/data.txt", options).unwrap();
        writer.write_all(b"some data").unwrap();
        writer.finish().unwrap();

        zip_path
    }

    #[test]
    fn test_extract_zip() {
        let temp = tempfile::tempdir().unwrap();
        let zip_path = make_test_zip(temp.path());
        let output_dir = temp.path().join("extracted");

        let paths = extract_zip(&zip_path, &output_dir).unwrap();

        assert_eq!(paths.len(), 2);
        // All extracted files should exist
        for p in &paths {
            assert!(p.exists(), "expected file to exist: {}", p.display());
        }

        // Check content of hello.txt (at the root of output_dir)
        let hello = output_dir.join("hello.txt");
        let content = fs::read_to_string(&hello).unwrap();
        assert_eq!(content, "hello world");

        // subdir/data.txt should be preserved in its subdirectory
        let data = output_dir.join("subdir").join("data.txt");
        assert!(data.exists(), "expected subdir/data.txt to exist");
        let data_content = fs::read_to_string(&data).unwrap();
        assert_eq!(data_content, "some data");
    }

    #[test]
    fn test_extract_nonexistent_zip_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let result = extract_zip(Path::new("/nonexistent/path/to/missing.zip"), temp.path());
        assert!(result.is_err());
    }
}
