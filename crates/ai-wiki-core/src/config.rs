use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub paths: PathsConfig,
    pub pdf: PdfConfig,
    pub rejection: RejectionConfig,
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    pub raw_dir: PathBuf,
    pub wiki_dir: PathBuf,
    pub database_path: PathBuf,
    pub processed_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfConfig {
    pub book_min_pages: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionConfig {
    pub non_operative_extensions: Vec<String>,
    pub sensitive_filename_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub qpdf_path: String,
    pub pdftotext_path: String,
    pub pdftoppm_path: String,
    pub tesseract_path: String,
    pub ffmpeg_path: String,
    pub whisper_cpp_path: String,
    pub whisper_model_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            paths: PathsConfig {
                raw_dir: PathBuf::from("raw"),
                wiki_dir: PathBuf::from("wiki"),
                database_path: PathBuf::from("ai-wiki.db"),
                processed_dir: PathBuf::from("processed"),
            },
            pdf: PdfConfig { book_min_pages: 50 },
            rejection: RejectionConfig {
                non_operative_extensions: vec![".dmg".to_string()],
                sensitive_filename_patterns: vec![
                    "divorce".to_string(),
                    "court".to_string(),
                    "bank.statement".to_string(),
                    "tax.return".to_string(),
                    "report.card".to_string(),
                    "financial".to_string(),
                    "lease".to_string(),
                ],
            },
            tools: ToolsConfig {
                qpdf_path: "qpdf".to_string(),
                pdftotext_path: "pdftotext".to_string(),
                pdftoppm_path: "pdftoppm".to_string(),
                tesseract_path: "tesseract".to_string(),
                ffmpeg_path: "ffmpeg".to_string(),
                whisper_cpp_path: "whisper-cpp".to_string(),
                whisper_model_path: PathBuf::from("models/ggml-large-v3.bin"),
            },
        }
    }
}

impl PathsConfig {
    /// Return the path where processed text for a given queue item ID is stored.
    ///
    /// This is the single source of truth for the `{id}.txt` naming convention.
    pub fn processed_text_path(&self, id: i64) -> PathBuf {
        self.processed_dir.join(format!("{id}.txt"))
    }
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.pdf.book_min_pages == 0 {
            anyhow::bail!("pdf.book_min_pages must be greater than 0");
        }

        let tool_paths = [
            ("qpdf_path", &self.tools.qpdf_path),
            ("pdftotext_path", &self.tools.pdftotext_path),
            ("pdftoppm_path", &self.tools.pdftoppm_path),
            ("tesseract_path", &self.tools.tesseract_path),
            ("ffmpeg_path", &self.tools.ffmpeg_path),
            ("whisper_cpp_path", &self.tools.whisper_cpp_path),
        ];
        for (name, path) in tool_paths {
            if path.is_empty() {
                anyhow::bail!("tools.{name} must not be empty");
            }
        }

        if self.tools.whisper_model_path.as_os_str().is_empty() {
            anyhow::bail!("tools.whisper_model_path must not be empty");
        }

        Ok(())
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config file {}: {}", path.display(), e))?;
        let config: Config = toml::from_str(&content).map_err(|e| {
            anyhow::anyhow!("failed to parse config file {}: {}", path.display(), e)
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("failed to serialize config: {}", e))?;
        std::fs::write(path, content).map_err(|e| {
            anyhow::anyhow!("failed to write config file {}: {}", path.display(), e)
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_default_config_round_trips() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.pdf.book_min_pages, 50);
        assert_eq!(deserialized.paths.raw_dir, PathBuf::from("raw"));
        assert_eq!(
            deserialized.rejection.non_operative_extensions,
            vec![".dmg"]
        );
    }

    #[test]
    fn test_load_config_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config::default();
        config.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.pdf.book_min_pages, config.pdf.book_min_pages);
        assert_eq!(loaded.paths.wiki_dir, config.paths.wiki_dir);
    }

    #[test]
    fn test_load_missing_file_returns_error() {
        let result = Config::load(std::path::Path::new("/nonexistent/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"this is not valid toml [[[").unwrap();

        let result = Config::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_config_validates() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn test_validate_rejects_zero_book_min_pages() {
        let mut config = Config::default();
        config.pdf.book_min_pages = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_empty_tool_path() {
        let mut config = Config::default();
        config.tools.qpdf_path = String::new();
        assert!(config.validate().is_err());
    }
}
