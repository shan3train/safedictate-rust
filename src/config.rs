//! User settings stored as TOML under the XDG config directory.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

const QUALIFIER: &str = "dev";
const ORG: &str = "safedictate";
const APP: &str = "SafeDictate";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub model_size: String,
    pub mic_name: String,
    pub hotkey: String,
    pub max_record_seconds: u32,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model_size: "base".into(),
            mic_name: "default".into(),
            hotkey: "ctrl+shift+Space".into(),
            max_record_seconds: 30,
            sample_rate: 48_000,
            channels: 1,
        }
    }
}

pub fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORG, APP)
        .context("could not determine project directories (HOME unset?)")
}

pub fn config_path() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn load() -> Result<Settings> {
    let path = config_path()?;
    load_from(&path)
}

pub fn load_from(path: &Path) -> Result<Settings> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let s: Settings =
        toml::from_str(&text).with_context(|| format!("parsing TOML at {}", path.display()))?;
    Ok(s)
}

pub fn save(s: &Settings) -> Result<PathBuf> {
    let path = config_path()?;
    save_to(s, &path)?;
    Ok(path)
}

pub fn save_to(s: &Settings, path: &Path) -> Result<()> {
    let parent = path.parent().context("config path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let text = toml::to_string_pretty(s).context("serializing settings")?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert!(s.max_record_seconds > 0);
        assert!(s.sample_rate >= 8000);
        assert!(!s.hotkey.is_empty());
    }

    #[test]
    fn toml_round_trip_preserves_all_fields() {
        let s = Settings {
            model_size: "small".into(),
            mic_name: "Some Mic".into(),
            hotkey: "Alt+Digit2".into(),
            max_record_seconds: 45,
            sample_rate: 44_100,
            channels: 2,
        };
        let text = toml::to_string_pretty(&s).unwrap();
        let parsed: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, parsed);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let partial = "model_size = \"tiny\"\n";
        let s: Settings = toml::from_str(partial).unwrap();
        assert_eq!(s.model_size, "tiny");
        assert_eq!(s.mic_name, Settings::default().mic_name);
        assert_eq!(s.max_record_seconds, Settings::default().max_record_seconds);
    }

    #[test]
    fn save_and_load_round_trip_via_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/config.toml");
        let s = Settings {
            model_size: "medium".into(),
            ..Settings::default()
        };
        save_to(&s, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    fn load_from_missing_path_returns_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.toml");
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, Settings::default());
    }
}
