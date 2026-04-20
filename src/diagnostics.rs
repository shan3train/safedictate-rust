//! System diagnostics — runs on startup and via `--doctor`.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub severity: Severity,
    pub detail: String,
}

impl Check {
    fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { name: name.into(), severity: Severity::Ok, detail: detail.into() }
    }
    fn warn(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { name: name.into(), severity: Severity::Warn, detail: detail.into() }
    }
    fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { name: name.into(), severity: Severity::Fail, detail: detail.into() }
    }
}

pub fn run_all() -> Vec<Check> {
    let mut out = Vec::new();
    out.extend(check_audio());
    out.push(check_hotkey_manager());
    out.push(check_gpu_build());
    out.push(check_config_dir());
    out
}

fn check_gpu_build() -> Check {
    #[cfg(feature = "cuda")]
    {
        Check::ok(
            "GPU backend: CUDA",
            "compiled in — whisper.cpp uses CUDA if a GPU is present, else falls back to CPU",
        )
    }
    #[cfg(all(feature = "vulkan", not(feature = "cuda")))]
    {
        Check::ok(
            "GPU backend: Vulkan",
            "compiled in — whisper.cpp uses Vulkan compute if a GPU is present, else falls back to CPU",
        )
    }
    #[cfg(not(any(feature = "cuda", feature = "vulkan")))]
    {
        Check::warn(
            "GPU backend",
            "binary is CPU-only. Rebuild with `--features vulkan` or `--features cuda` for GPU acceleration.",
        )
    }
}

fn check_audio() -> Vec<Check> {
    let devices = match crate::audio::list_input_devices() {
        Ok(d) => d,
        Err(e) => {
            return vec![Check::fail(
                "Audio (WASAPI)",
                format!("Failed to enumerate input devices: {e}"),
            )];
        }
    };

    let mut out = vec![Check::ok("Audio (WASAPI)", "Windows audio initialized")];

    match devices.first() {
        Some(d) => out.push(Check::ok(
            "Default input device",
            format!("{}", d.display),
        )),
        None => out.push(Check::warn(
            "Default input device",
            "No input devices found. Plug in a microphone.",
        )),
    }

    if !devices.is_empty() {
        let lines: Vec<String> = devices.iter().map(|d| d.display.clone()).collect();
        out.push(Check::ok(
            format!("Input devices ({})", devices.len()),
            lines.join("\n"),
        ));
    }

    out
}

fn check_hotkey_manager() -> Check {
    match global_hotkey::GlobalHotKeyManager::new() {
        Ok(_) => Check::ok("Global hotkey manager", "Initialized."),
        Err(e) => Check::fail(
            "Global hotkey manager",
            format!("Failed to initialize: {e}"),
        ),
    }
}

fn check_config_dir() -> Check {
    match crate::config::config_path() {
        Ok(path) => {
            let parent = path.parent().map(PathBuf::from).unwrap_or_default();
            if parent.as_os_str().is_empty() {
                return Check::fail("Config directory", "config path has no parent");
            }
            if parent.exists() {
                Check::ok("Config directory", path.display().to_string())
            } else {
                match std::fs::create_dir_all(&parent) {
                    Ok(()) => Check::ok("Config directory (created)", path.display().to_string()),
                    Err(e) => Check::fail(
                        "Config directory",
                        format!("Cannot create {}: {e}", parent.display()),
                    ),
                }
            }
        }
        Err(e) => Check::fail("Config directory", format!("{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_constructors_set_severity() {
        assert_eq!(Check::ok("a", "b").severity, Severity::Ok);
        assert_eq!(Check::warn("a", "b").severity, Severity::Warn);
        assert_eq!(Check::fail("a", "b").severity, Severity::Fail);
    }

    #[test]
    fn run_all_includes_audio_and_config() {
        let checks = run_all();
        assert!(!checks.is_empty());
        assert!(checks.iter().any(|c| c.name.contains("Config")));
    }
}
