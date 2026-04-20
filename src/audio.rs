//! Audio device discovery via cpal (WASAPI on Windows).

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};

pub fn init_once() {
    // Nothing to initialize on Windows — cpal handles this lazily.
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDevice {
    pub display: String,
    /// Device name as returned by cpal — used to re-open the device in the recorder.
    pub pw_name: String,
}

pub fn list_input_devices() -> Result<Vec<InputDevice>> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .context("enumerating input devices")?
        .filter_map(|d| {
            let name = d.name().ok()?;
            Some(InputDevice {
                display: name.clone(),
                pw_name: name,
            })
        })
        .collect();
    Ok(devices)
}

pub fn default_input() -> Result<Option<InputDevice>> {
    let host = cpal::default_host();
    Ok(host.default_input_device().and_then(|d| {
        let name = d.name().ok()?;
        Some(InputDevice {
            display: name.clone(),
            pw_name: name,
        })
    }))
}
