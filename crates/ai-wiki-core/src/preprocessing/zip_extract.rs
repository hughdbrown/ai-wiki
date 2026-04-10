use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context;

pub fn extract_zip(zip_path: &Path, output_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let file = fs::File::open(zip_path)
        .with_context(|| format!("failed to open zip file: {}", zip_path.display()))?;

    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read zip archive: {}", zip_path.display()))?;

    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output dir: {}", output_dir.display()))?;

    let mut extracted_paths = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("failed to access zip entry {i}"))?;

        // Skip directories
        if entry.is_dir() {
            continue;
        }

        // Zip-slip protection: use only the file name, not the full path
        let file_name = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(PathBuf::from))
            .with_context(|| format!("zip entry {i} has an unsafe or empty path"))?;

        let dest_path = output_dir.join(&file_name);

        let mut dest_file = fs::File::create(&dest_path)
            .with_context(|| format!("failed to create file: {}", dest_path.display()))?;

        io::copy(&mut entry, &mut dest_file)
            .with_context(|| format!("failed to extract entry to: {}", dest_path.display()))?;

        extracted_paths.push(dest_path);
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
        // Both files should exist in output_dir (zip-slip: flat, no subdir)
        for p in &paths {
            assert!(p.exists(), "expected file to exist: {}", p.display());
        }

        // Check content of hello.txt
        let hello = output_dir.join("hello.txt");
        let content = fs::read_to_string(&hello).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_extract_nonexistent_zip_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let result = extract_zip(Path::new("/nonexistent/path/to/missing.zip"), temp.path());
        assert!(result.is_err());
    }
}
