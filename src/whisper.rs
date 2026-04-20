//! Whisper model loading, download, and transcription.
//!
//! Models live under `~/.cache/safedictate/models/ggml-{size}.en.bin`.
//! Download URLs are the standard Hugging Face whisper.cpp mirror.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Static table of the whisper models we expose in the UI.
/// `size` is the key stored in config; `filename` matches the HF mirror.
#[derive(Debug, Clone, Copy)]
pub struct ModelInfo {
    pub size: &'static str,
    pub filename: &'static str,
    pub approx_mb: u32,
    pub english_only: bool,
    pub description: &'static str,
}

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        size: "tiny",
        filename: "ggml-tiny.en.bin",
        approx_mb: 78,
        english_only: true,
        description: "fastest, low accuracy",
    },
    ModelInfo {
        size: "base",
        filename: "ggml-base.en.bin",
        approx_mb: 148,
        english_only: true,
        description: "fast, decent accuracy",
    },
    ModelInfo {
        size: "small",
        filename: "ggml-small.en.bin",
        approx_mb: 488,
        english_only: true,
        description: "good",
    },
    ModelInfo {
        size: "medium",
        filename: "ggml-medium.en.bin",
        approx_mb: 1534,
        english_only: true,
        description: "better",
    },
    ModelInfo {
        size: "large-v3-turbo",
        filename: "ggml-large-v3-turbo.bin",
        approx_mb: 1625,
        english_only: false,
        description: "near-large accuracy, ~medium speed",
    },
    ModelInfo {
        size: "large-v3",
        filename: "ggml-large-v3.bin",
        approx_mb: 3095,
        english_only: false,
        description: "highest accuracy, multilingual",
    },
];

pub fn info(size: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.size == size)
}

/// Format bytes-ish-MB as "488 MB" or "1.5 GB".
pub fn human_mb(mb: u32) -> String {
    if mb >= 1000 {
        format!("{:.1} GB", mb as f32 / 1000.0)
    } else {
        format!("{mb} MB")
    }
}

pub fn model_path(size: &str) -> Result<PathBuf> {
    let info = info(size).with_context(|| format!("unknown model size '{size}'"))?;
    let dirs = crate::config::project_dirs()?;
    Ok(dirs.cache_dir().join("models").join(info.filename))
}

pub fn model_url(size: &str) -> String {
    let filename = info(size)
        .map(|i| i.filename.to_string())
        .unwrap_or_else(|| format!("ggml-{size}.en.bin"));
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}")
}

pub fn model_exists(size: &str) -> bool {
    model_path(size).map(|p| p.exists()).unwrap_or(false)
}

/// Download the GGML model for `size` to the cache directory.
///
/// `progress(downloaded_bytes, total_bytes_if_known)` is called periodically so
/// the UI can render a progress bar.
pub fn download_model<F>(size: &str, mut progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>),
{
    let path = model_path(size)?;
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let url = model_url(size);
    tracing::info!("downloading whisper model: {url}");
    let resp = ureq::get(&url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let total: Option<u64> = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());
    let tmp = path.with_extension("bin.tmp");
    let mut out = std::fs::File::create(&tmp)
        .with_context(|| format!("creating {}", tmp.display()))?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        let n = reader.read(&mut buf).context("read from network")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .with_context(|| format!("writing {}", tmp.display()))?;
        downloaded += n as u64;
        progress(downloaded, total);
    }
    out.sync_all().ok();
    drop(out);
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    tracing::info!("model ready at {}", path.display());
    Ok(path)
}

/// Holds a loaded WhisperContext so subsequent transcriptions are fast.
pub struct Transcriber {
    ctx: WhisperContext,
    size: String,
}

impl Transcriber {
    pub fn load(size: &str) -> Result<Self> {
        let path = model_path(size)?;
        anyhow::ensure!(
            path.exists(),
            "whisper model not found at {}. Download it from the Settings panel.",
            path.display()
        );
        let path_str = path
            .to_str()
            .context("model path is not valid UTF-8")?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .context("loading whisper context")?;
        Ok(Self {
            ctx,
            size: size.to_string(),
        })
    }

    pub fn size(&self) -> &str {
        &self.size
    }

    /// Run transcription on a mono F32 buffer at 16 kHz.
    pub fn transcribe(&self, samples: &[f32], lang: &str) -> Result<String> {
        anyhow::ensure!(!samples.is_empty(), "no samples to transcribe");
        let mut state = self.ctx.create_state().context("create_state")?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(lang));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, samples)
            .context("whisper full() failed")?;

        let n = state.full_n_segments();
        let mut text = String::new();
        for segment in state.as_iter() {
            let s = segment.to_str_lossy().unwrap_or_default();
            text.push_str(&s);
        }
        tracing::debug!("whisper produced {n} segments, {} chars", text.len());
        Ok(text.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_path_is_under_cache_models_dir() {
        let p = model_path("base").unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains("safedictate"));
        assert!(s.contains("models"));
        assert!(s.ends_with("ggml-base.en.bin"));
    }

    #[test]
    fn model_url_targets_hf_whisper_cpp() {
        let u = model_url("tiny");
        assert!(u.starts_with("https://huggingface.co/ggerganov/whisper.cpp"));
        assert!(u.ends_with("ggml-tiny.en.bin"));
    }

    #[test]
    fn models_contain_expected_sizes() {
        let sizes: Vec<&str> = MODELS.iter().map(|m| m.size).collect();
        for expected in ["tiny", "base", "small", "medium", "large-v3-turbo", "large-v3"] {
            assert!(sizes.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn large_v3_uses_multilingual_filename() {
        assert_eq!(info("large-v3").unwrap().filename, "ggml-large-v3.bin");
        assert!(!info("large-v3").unwrap().english_only);
    }

    #[test]
    fn human_mb_formats_correctly() {
        assert_eq!(human_mb(78), "78 MB");
        assert_eq!(human_mb(488), "488 MB");
        assert_eq!(human_mb(1534), "1.5 GB");
        assert_eq!(human_mb(3095), "3.1 GB");
    }

    #[test]
    fn load_fails_with_useful_message_when_size_unknown() {
        let err = match Transcriber::load("definitely-not-a-real-size-xyz") {
            Ok(_) => panic!("expected load to fail for bogus size"),
            Err(e) => e,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.to_lowercase().contains("unknown"),
            "expected 'unknown model size' hint, got: {msg}"
        );
    }

    /// End-to-end: load the tiny model (downloaded once by `--download-model
    /// tiny`) and transcribe a short buffer. Gated behind `--ignored` because
    /// it needs the model file on disk and is slow.
    #[test]
    #[ignore = "needs ggml-tiny.en.bin at the cache path (run --download-model tiny)"]
    fn tiny_model_transcribes_silence_without_crashing() {
        if !model_exists("tiny") {
            panic!(
                "tiny model not installed at {}",
                model_path("tiny").unwrap().display()
            );
        }
        let t = Transcriber::load("tiny").expect("load tiny");
        // 1 second of silence at 16 kHz.
        let samples = vec![0.0f32; 16_000];
        let text = t.transcribe(&samples, "en").expect("transcribe");
        // We can't assert on the exact string — whisper hallucinates on
        // silence. We can assert it returns *something* without panicking.
        assert!(text.len() < 4096, "unexpectedly large transcript: {text}");
    }
}
