//! Hotkey string parsing — thin wrapper around `global_hotkey` with
//! friendlier error messages.
//!
//! Format: modifiers first, then exactly one key, joined by `+`.
//! Modifiers: `shift`, `control`/`ctrl`, `alt`, `super`/`meta`/`cmd`.
//! Keys: anything in [`keyboard_types::Code`] — `Digit1`, `KeyA`, `F1`, `Space`, ...

use anyhow::{Context, Result};
use global_hotkey::hotkey::HotKey;

/// Parse a hotkey string like `"alt+Digit1"` or `"ctrl+shift+Space"`.
pub fn parse(s: &str) -> Result<HotKey> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        anyhow::bail!("hotkey is empty");
    }
    trimmed.parse::<HotKey>().with_context(|| {
        format!(
            "invalid hotkey '{s}' (expected e.g. 'alt+Digit1', 'ctrl+shift+Space', 'F1')"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers};

    #[test]
    fn parses_alt_digit1() {
        let hk = parse("alt+Digit1").unwrap();
        assert_eq!(hk.key, Code::Digit1);
        assert!(hk.mods.contains(Modifiers::ALT));
    }

    #[test]
    fn parses_ctrl_shift_space() {
        let hk = parse("ctrl+shift+Space").unwrap();
        assert_eq!(hk.key, Code::Space);
        assert!(hk.mods.contains(Modifiers::CONTROL));
        assert!(hk.mods.contains(Modifiers::SHIFT));
    }

    #[test]
    fn parses_bare_function_key() {
        let hk = parse("F1").unwrap();
        assert_eq!(hk.key, Code::F1);
        assert!(hk.mods.is_empty());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
    }

    #[test]
    fn rejects_bogus() {
        assert!(parse("not+a+real+thing").is_err());
        assert!(parse("ctrl+").is_err());
    }

    #[test]
    fn parses_with_leading_trailing_whitespace() {
        assert!(parse("  alt+Digit1  ").is_ok());
    }
}
