pub mod detect;
pub mod media;
pub mod pdf;
pub mod zip_extract;

pub use detect::{FileClassification, detect_file_type};
pub use media::{extract_audio, transcribe_audio};
pub use pdf::{classify_pdf, extract_pdf_text, split_pdf_chapters};
pub use zip_extract::extract_zip;

use std::process::Command;

/// Run an external tool, returning a clear error if the binary is not found.
pub(crate) fn run_tool(
    cmd: &mut Command,
    tool_name: &str,
) -> anyhow::Result<std::process::ExitStatus> {
    cmd.status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!(
                "external tool '{}' not found. Install it (e.g., `brew install {}`)",
                tool_name,
                tool_name
            )
        } else {
            anyhow::anyhow!("failed to run '{}': {}", tool_name, e)
        }
    })
}

/// Run an external tool and capture output, returning a clear error if the binary is not found.
pub(crate) fn run_tool_output(
    cmd: &mut Command,
    tool_name: &str,
) -> anyhow::Result<std::process::Output> {
    cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!(
                "external tool '{}' not found. Install it (e.g., `brew install {}`)",
                tool_name,
                tool_name
            )
        } else {
            anyhow::anyhow!("failed to run '{}': {}", tool_name, e)
        }
    })
}
