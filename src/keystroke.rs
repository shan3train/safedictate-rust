use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

pub fn type_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    thread::sleep(Duration::from_millis(150));
    let mut enigo = Enigo::new(&Settings::default())
        .context("initializing enigo")?;
    // Explicitly release modifier keys so they don't corrupt the typed text.
    // Windows can still have Ctrl/Shift/Alt logically held after the hotkey
    // release event, which causes SendInput to produce wrong characters.
    let _ = enigo.key(Key::Control, Direction::Release);
    let _ = enigo.key(Key::Shift, Direction::Release);
    let _ = enigo.key(Key::Alt, Direction::Release);
    thread::sleep(Duration::from_millis(50));
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
