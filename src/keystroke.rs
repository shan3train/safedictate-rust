//! Type text at the current cursor via `enigo` (XTest on X11).
//!
//! A small delay is inserted before typing so the hotkey modifiers have a
//! chance to fully release at the OS level — otherwise the first few keys
//! can be interpreted with Alt/Ctrl still held.

use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use enigo::{Enigo, Keyboard, Settings};

pub fn type_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    thread::sleep(Duration::from_millis(75));
    let mut enigo = Enigo::new(&Settings::default())
        .context("initializing enigo — is the display server reachable?")?;
    enigo
        .text(text)
        .with_context(|| format!("typing {} chars", text.len()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_noop_and_succeeds() {
        assert!(type_text("").is_ok());
    }
}
