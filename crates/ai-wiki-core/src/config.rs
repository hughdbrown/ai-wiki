use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Global application config, read from ~/.ai-wiki/config.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub tools: ToolsConfig,
    #[serde(default)]
    pub process: ProcessConfig,
    pub wikis: HashMap<String, WikiEntry>,
}

/// Settings for the `process` command (Claude invocation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfig {
    /// "pro" for Claude Code Pro subscription, "api" for ANTHROPIC_API_KEY.
    #[serde(default = "ProcessConfig::default_auth")]
    pub auth: String,
    /// Optional model override (e.g., "sonnet", "opus", "claude-sonnet-4-6").
    pub model: Option<String>,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            auth: "pro".to_string(),
            model: None,
        }
    }
}

impl ProcessConfig {
    fn default_auth() -> String {
        "pro".to_string()
    }
}

/// Per-wiki config entry -- just a root directory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiEntry {
    pub root: PathBuf,
}

/// Resolved wiki with derived paths -- not serialized
#[derive(Debug, Clone)]
pub struct WikiConfig {
    pub name: String,
    pub root: PathBuf,
}

impl WikiConfig {
    pub fn wiki_dir(&self) -> PathBuf {
        self.root.join("wiki")
    }

    pub fn processed_dir(&self) -> PathBuf {
        self.root.join("processed")
    }

    pub fn raw_dir(&self) -> PathBuf {
        self.root.join("raw")
    }

    pub fn database_path(&self) -> PathBuf {
        self.root.join("ai-wiki.db")
    }

    pub fn processed_text_path(&self, id: i64) -> PathBuf {
        self.processed_dir().join(format!("{id}.txt"))
    }
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

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            qpdf_path: "qpdf".to_string(),
            pdftotext_path: "pdftotext".to_string(),
            pdftoppm_path: "pdftoppm".to_string(),
            tesseract_path: "tesseract".to_string(),
            ffmpeg_path: "ffmpeg".to_string(),
            whisper_cpp_path: "whisper-cpp".to_string(),
            whisper_model_path: PathBuf::from("models/ggml-large-v3.bin"),
        }
    }
}

impl AppConfig {
    /// Returns the path to the global config file: ~/.ai-wiki/config.toml
    /// Can be overridden with the `AI_WIKI_CONFIG` environment variable (useful for tests).
    pub fn config_path() -> anyhow::Result<PathBuf> {
        if let Ok(path) = std::env::var("AI_WIKI_CONFIG") {
            return Ok(PathBuf::from(path));
        }
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
        Ok(home.join(".ai-wiki").join("config.toml"))
    }

    /// Load the global config from ~/.ai-wiki/config.toml.
    /// Returns an error if the config file does not exist.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path()?;
        Self::load_from(&path)
    }

    /// Load the global config, creating it with defaults if it does not exist.
    /// Use this only in commands that should bootstrap the config (e.g., `init`).
    pub fn load_or_create() -> anyhow::Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }
        Self::load_from(&path)
    }

    /// Load from a specific path (useful for tests).
    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let config: AppConfig = toml::from_str(&content).map_err(|e| {
            anyhow::anyhow!("failed to parse config file {}: {}", path.display(), e)
        })?;
        config.validate_tools()?;
        Ok(config)
    }

    /// Write the config to ~/.ai-wiki/config.toml.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path()?;
        self.save_to(&path)
    }

    /// Write the config to a specific path (useful for tests).
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!(
                    "failed to create config directory {}: {}",
                    parent.display(),
                    e
                )
            })?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("failed to serialize config: {}", e))?;
        std::fs::write(path, content).map_err(|e| {
            anyhow::anyhow!("failed to write config file {}: {}", path.display(), e)
        })?;
        Ok(())
    }

    /// Look up a wiki by name and return a resolved WikiConfig.
    pub fn resolve_wiki(&self, name: &str) -> anyhow::Result<WikiConfig> {
        let entry = self.wikis.get(name).ok_or_else(|| {
            let available: Vec<&str> = self.wikis.keys().map(|s| s.as_str()).collect();
            if available.is_empty() {
                anyhow::anyhow!("wiki '{}' not found. No wikis registered.", name)
            } else {
                anyhow::anyhow!(
                    "wiki '{}' not found. Available wikis: {}",
                    name,
                    available.join(", ")
                )
            }
        })?;
        Ok(WikiConfig {
            name: name.to_string(),
            root: entry.root.clone(),
        })
    }

    /// Check if the current working directory is at or under any wiki root.
    /// When multiple wiki roots overlap, returns the most specific (longest) match.
    pub fn find_wiki_by_cwd(&self) -> Option<WikiConfig> {
        let raw_cwd = std::env::current_dir().ok()?;
        let canon_cwd = std::fs::canonicalize(&raw_cwd).ok();
        let mut best: Option<(usize, &str, &WikiEntry)> = None;
        for (name, entry) in &self.wikis {
            match std::fs::canonicalize(&entry.root) {
                Ok(canon_root) => {
                    if let Some(ref cc) = canon_cwd {
                        if cc.starts_with(&canon_root) {
                            let depth = canon_root.components().count();
                            if best.as_ref().is_none_or(|(d, _, _)| depth > *d) {
                                best = Some((depth, name.as_str(), entry));
                            }
                        }
                    } else if raw_cwd.starts_with(&entry.root) {
                        // CWD can't be canonicalized — fall back to raw vs raw
                        let depth = entry.root.components().count();
                        if best.as_ref().is_none_or(|(d, _, _)| depth > *d) {
                            best = Some((depth, name.as_str(), entry));
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "warning: could not resolve wiki '{}' root {}: {}",
                        name,
                        entry.root.display(),
                        e
                    );
                    // Fall back to non-canonicalized path comparison
                    if raw_cwd.starts_with(&entry.root) {
                        let depth = entry.root.components().count();
                        if best.as_ref().is_none_or(|(d, _, _)| depth > *d) {
                            best = Some((depth, name.as_str(), entry));
                        }
                    }
                }
            }
        }
        best.map(|(_, name, entry)| WikiConfig {
            name: name.to_string(),
            root: entry.root.clone(),
        })
    }

    /// Resolve a wiki: explicit name wins, then CWD match, then error.
    pub fn resolve_wiki_auto(&self, explicit_name: Option<&str>) -> anyhow::Result<WikiConfig> {
        if let Some(name) = explicit_name {
            return self.resolve_wiki(name);
        }

        if let Some(wiki) = self.find_wiki_by_cwd() {
            return Ok(wiki);
        }

        let available: Vec<&str> = self.wikis.keys().map(|s| s.as_str()).collect();
        if available.is_empty() {
            anyhow::bail!(
                "no wiki specified and CWD is not under any wiki root. No wikis registered. Run `ai-wiki init` to create one."
            );
        } else {
            anyhow::bail!(
                "no wiki specified and CWD is not under any wiki root. Available wikis: {}",
                available.join(", ")
            );
        }
    }

    /// Register a wiki entry. Overwrites if name already exists.
    pub fn register_wiki(&mut self, name: String, root: PathBuf) {
        self.wikis.insert(name, WikiEntry { root });
    }

    /// Validate that tool paths are non-empty.
    pub fn validate_tools(&self) -> anyhow::Result<()> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_default_config_round_trips() {
        let mut config = AppConfig::default();
        config.register_wiki("test".to_string(), PathBuf::from("/tmp/test-wiki"));
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: AppConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.tools.qpdf_path, "qpdf");
        assert!(deserialized.wikis.contains_key("test"));
        assert_eq!(
            deserialized.wikis["test"].root,
            PathBuf::from("/tmp/test-wiki")
        );
    }

    #[test]
    fn test_load_config_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = AppConfig::default();
        config.register_wiki("mywiki".to_string(), PathBuf::from("/tmp/mywiki"));
        config.save_to(&path).unwrap();

        let loaded = AppConfig::load_from(&path).unwrap();
        assert_eq!(loaded.tools.qpdf_path, config.tools.qpdf_path);
        assert!(loaded.wikis.contains_key("mywiki"));
    }

    #[test]
    fn test_load_missing_file_returns_error() {
        let result = AppConfig::load_from(Path::new("/nonexistent/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"this is not valid toml [[[").unwrap();

        let result = AppConfig::load_from(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_config_validates() {
        AppConfig::default().validate_tools().unwrap();
    }

    #[test]
    fn test_validate_rejects_empty_tool_path() {
        let mut config = AppConfig::default();
        config.tools.qpdf_path = String::new();
        assert!(config.validate_tools().is_err());
    }

    #[test]
    fn test_resolve_wiki() {
        let mut config = AppConfig::default();
        config.register_wiki("rust".to_string(), PathBuf::from("/wikis/rust"));

        let wiki = config.resolve_wiki("rust").unwrap();
        assert_eq!(wiki.name, "rust");
        assert_eq!(wiki.root, PathBuf::from("/wikis/rust"));
    }

    #[test]
    fn test_resolve_wiki_not_found() {
        let config = AppConfig::default();
        let result = config.resolve_wiki("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_wiki_config_derived_paths() {
        let wiki = WikiConfig {
            name: "test".to_string(),
            root: PathBuf::from("/wikis/test"),
        };
        assert_eq!(wiki.wiki_dir(), PathBuf::from("/wikis/test/wiki"));
        assert_eq!(wiki.processed_dir(), PathBuf::from("/wikis/test/processed"));
        assert_eq!(wiki.raw_dir(), PathBuf::from("/wikis/test/raw"));
        assert_eq!(
            wiki.database_path(),
            PathBuf::from("/wikis/test/ai-wiki.db")
        );
        assert_eq!(
            wiki.processed_text_path(42),
            PathBuf::from("/wikis/test/processed/42.txt")
        );
    }

    #[test]
    fn test_register_wiki_overwrites() {
        let mut config = AppConfig::default();
        config.register_wiki("rust".to_string(), PathBuf::from("/old/path"));
        config.register_wiki("rust".to_string(), PathBuf::from("/new/path"));
        assert_eq!(config.wikis["rust"].root, PathBuf::from("/new/path"));
    }
}
