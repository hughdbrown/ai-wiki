use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use crate::config::ToolsConfig;

pub fn extract_audio(
    video_path: &Path,
    output_dir: &Path,
    tools: &ToolsConfig,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output dir: {}", output_dir.display()))?;

    let stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");

    let output_path = output_dir.join(format!("{stem}.wav"));

    let status = super::run_tool(
        Command::new(&tools.ffmpeg_path)
            .arg("-i")
            .arg(video_path)
            .arg("-vn")
            .arg("-acodec")
            .arg("pcm_s16le")
            .arg("-ar")
            .arg("16000")
            .arg("-ac")
            .arg("1")
            .arg("-y")
            .arg(&output_path),
        "ffmpeg",
    )?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "ffmpeg failed to extract audio from: {}",
            video_path.display()
        ));
    }

    Ok(output_path)
}

pub fn transcribe_audio(audio_path: &Path, tools: &ToolsConfig) -> anyhow::Result<String> {
    let output = super::run_tool_output(
        Command::new(&tools.whisper_cpp_path)
            .arg("-m")
            .arg(&tools.whisper_model_path)
            .arg("-f")
            .arg(audio_path)
            .arg("--output-txt")
            .arg("--no-timestamps"),
        "whisper-cpp",
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "whisper-cpp failed for {}: {}",
            audio_path.display(),
            stderr
        ));
    }

    // whisper-cpp with --output-txt typically writes to <input>.txt; check that first
    let txt_path = {
        let mut p = audio_path.to_path_buf();
        let new_name = format!(
            "{}.txt",
            p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("audio.wav")
        );
        p.set_file_name(new_name);
        p
    };

    if txt_path.exists() {
        let text = std::fs::read_to_string(&txt_path).with_context(|| {
            format!(
                "failed to read whisper-cpp output file: {}",
                txt_path.display()
            )
        })?;
        let _ = std::fs::remove_file(&txt_path); // cleanup sidecar file
        return Ok(text);
    }

    // Fallback: some versions of whisper-cpp write transcript to stdout
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !stdout.trim().is_empty() {
        return Ok(stdout);
    }

    Err(anyhow::anyhow!(
        "whisper-cpp produced no output for: {}",
        audio_path.display()
    ))
}
