use std::path::Path;

use crate::queue::FileType;

#[derive(Debug, PartialEq)]
pub enum FileClassification {
    Ingestable(FileType),
}

pub fn detect_file_type(path: &Path) -> FileClassification {
    // Get extension as lowercase ".ext" format
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();

    // Match extension to FileType
    let file_type = match ext.as_str() {
        ".md" | ".markdown" => FileType::Markdown,
        ".txt" | ".text" => FileType::Text,
        ".pdf" => FileType::Pdf,
        ".zip" => FileType::Zip,
        ".mp3" | ".wav" | ".flac" | ".ogg" | ".m4a" => FileType::Audio,
        ".mp4" | ".mkv" | ".avi" | ".mov" | ".webm" => FileType::Video,
        _ => FileType::Unknown,
    };

    FileClassification::Ingestable(file_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_detect_markdown() {
        assert_eq!(
            detect_file_type(Path::new("notes.md")),
            FileClassification::Ingestable(FileType::Markdown)
        );
        assert_eq!(
            detect_file_type(Path::new("readme.markdown")),
            FileClassification::Ingestable(FileType::Markdown)
        );
    }

    #[test]
    fn test_detect_pdf() {
        assert_eq!(
            detect_file_type(Path::new("document.pdf")),
            FileClassification::Ingestable(FileType::Pdf)
        );
    }

    #[test]
    fn test_detect_zip() {
        assert_eq!(
            detect_file_type(Path::new("archive.zip")),
            FileClassification::Ingestable(FileType::Zip)
        );
    }

    #[test]
    fn test_detect_video() {
        assert_eq!(
            detect_file_type(Path::new("movie.mp4")),
            FileClassification::Ingestable(FileType::Video)
        );
        assert_eq!(
            detect_file_type(Path::new("clip.mkv")),
            FileClassification::Ingestable(FileType::Video)
        );
    }

    #[test]
    fn test_detect_audio() {
        assert_eq!(
            detect_file_type(Path::new("song.mp3")),
            FileClassification::Ingestable(FileType::Audio)
        );
    }

    #[test]
    fn test_unknown_extension() {
        assert_eq!(
            detect_file_type(Path::new("data.xyz")),
            FileClassification::Ingestable(FileType::Unknown)
        );
    }

    #[test]
    fn test_dmg_is_unknown() {
        // DMG files are no longer rejected; they return Unknown
        assert_eq!(
            detect_file_type(Path::new("installer.dmg")),
            FileClassification::Ingestable(FileType::Unknown)
        );
    }
}
