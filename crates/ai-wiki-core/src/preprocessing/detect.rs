use std::path::Path;

use crate::config::Config;
use crate::queue::FileType;

#[derive(Debug, PartialEq)]
pub enum FileClassification {
    Ingestable(FileType),
    Rejected(String),
}

pub fn detect_file_type(path: &Path, config: &Config) -> FileClassification {
    // 1. Get extension as lowercase ".ext" format
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();

    // 2. Check non_operative_extensions in config -> Rejected
    if config.rejection.non_operative_extensions.contains(&ext) {
        return FileClassification::Rejected(format!("non-operative extension: {ext}"));
    }

    // 3. Check sensitive_filename_patterns in config (case-insensitive filename contains) -> Rejected
    // Note: pattern matching is case-insensitive ASCII only. Unicode homoglyphs
    // (e.g., Cyrillic characters resembling Latin) are not normalized. This is
    // a convenience filter, not a security boundary — users should verify the
    // rejection list in the wiki's TUI after ingestion.
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    for pattern in &config.rejection.sensitive_filename_patterns {
        if filename.contains(&pattern.to_lowercase()) {
            return FileClassification::Rejected(format!("sensitive filename pattern: {pattern}"));
        }
    }

    // 4. Match extension to FileType
    let file_type = match ext.as_str() {
        ".md" | ".markdown" => FileType::Markdown,
        ".txt" | ".text" => FileType::Text,
        ".pdf" => FileType::Pdf,
        ".zip" => FileType::Zip,
        ".mp3" | ".wav" | ".flac" | ".ogg" | ".m4a" => FileType::Audio,
        ".mp4" | ".mkv" | ".avi" | ".mov" | ".webm" => FileType::Video,
        _ => FileType::Unknown,
    };

    // 5. Return Ingestable(file_type)
    FileClassification::Ingestable(file_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn default_config() -> Config {
        Config::default()
    }

    #[test]
    fn test_detect_markdown() {
        let config = default_config();
        assert_eq!(
            detect_file_type(Path::new("notes.md"), &config),
            FileClassification::Ingestable(FileType::Markdown)
        );
        assert_eq!(
            detect_file_type(Path::new("readme.markdown"), &config),
            FileClassification::Ingestable(FileType::Markdown)
        );
    }

    #[test]
    fn test_detect_pdf() {
        let config = default_config();
        assert_eq!(
            detect_file_type(Path::new("document.pdf"), &config),
            FileClassification::Ingestable(FileType::Pdf)
        );
    }

    #[test]
    fn test_detect_zip() {
        let config = default_config();
        assert_eq!(
            detect_file_type(Path::new("archive.zip"), &config),
            FileClassification::Ingestable(FileType::Zip)
        );
    }

    #[test]
    fn test_detect_video() {
        let config = default_config();
        assert_eq!(
            detect_file_type(Path::new("movie.mp4"), &config),
            FileClassification::Ingestable(FileType::Video)
        );
        assert_eq!(
            detect_file_type(Path::new("clip.mkv"), &config),
            FileClassification::Ingestable(FileType::Video)
        );
    }

    #[test]
    fn test_reject_dmg() {
        let config = default_config();
        let result = detect_file_type(Path::new("installer.dmg"), &config);
        assert!(matches!(result, FileClassification::Rejected(_)));
    }

    #[test]
    fn test_reject_sensitive_filename() {
        let config = default_config();
        let result = detect_file_type(Path::new("divorce_papers.pdf"), &config);
        assert!(matches!(result, FileClassification::Rejected(_)));
    }

    #[test]
    fn test_reject_financial() {
        let config = default_config();
        let result = detect_file_type(Path::new("financial_report_2023.pdf"), &config);
        assert!(matches!(result, FileClassification::Rejected(_)));
    }

    #[test]
    fn test_unknown_extension() {
        let config = default_config();
        assert_eq!(
            detect_file_type(Path::new("data.xyz"), &config),
            FileClassification::Ingestable(FileType::Unknown)
        );
    }
}
