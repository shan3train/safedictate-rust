//! Audio recorder — opens a WASAPI capture stream via cpal and writes
//! F32 PCM to a WAV file until told to stop via a channel.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use hound::{SampleFormat as HoundFmt, WavSpec, WavWriter};

#[derive(Debug, Clone)]
pub struct RecordingStats {
    pub path: PathBuf,
    pub duration: Duration,
    pub frame_count: u64,
    pub channels: u16,
    pub sample_rate: u32,
}

pub struct RecordingHandle {
    stop_tx: std::sync::mpsc::SyncSender<()>,
    thread: Option<JoinHandle<Result<RecordingStats>>>,
    started: Instant,
}

impl RecordingHandle {
    pub fn start(target_name: String, wav_path: PathBuf) -> Result<Self> {
        let (stop_tx, stop_rx) = std::sync::mpsc::sync_channel::<()>(1);
        let thread = thread::Builder::new()
            .name("safedictate-recorder".into())
            .spawn(move || record_thread(target_name, wav_path, stop_rx))
            .context("spawning recorder thread")?;
        Ok(Self {
            stop_tx,
            thread: Some(thread),
            started: Instant::now(),
        })
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn stop(mut self) -> Result<RecordingStats> {
        let _ = self.stop_tx.send(());
        let handle = self.thread.take().context("recorder thread already joined")?;
        match handle.join() {
            Ok(res) => res,
            Err(_) => anyhow::bail!("recorder thread panicked"),
        }
    }
}

impl Drop for RecordingHandle {
    fn drop(&mut self) {
        if let Some(h) = self.thread.take() {
            let _ = self.stop_tx.send(());
            let _ = h.join();
        }
    }
}

fn record_thread(
    target_name: String,
    wav_path: PathBuf,
    stop_rx: std::sync::mpsc::Receiver<()>,
) -> Result<RecordingStats> {
    let host = cpal::default_host();

    let device = if target_name == "default" {
        host.default_input_device()
            .context("no default input device")?
    } else {
        host.input_devices()
            .context("enumerating devices")?
            .find(|d| d.name().map(|n| n == target_name).unwrap_or(false))
            .or_else(|| host.default_input_device())
            .context("requested device not found and no default available")?
    };

    // Use the device's default config — whatever sample rate and channel count
    // it prefers. The audio_pipeline will resample + downmix to 16 kHz mono.
    let supported_config = device
        .default_input_config()
        .context("querying default input config")?;

    let channels = supported_config.channels();
    let sample_rate = supported_config.sample_rate().0;
    let sample_format = supported_config.sample_format();
    let stream_config = supported_config.into();

    if let Some(parent) = wav_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    // Always write F32 WAV — convert other formats in the callback.
    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: HoundFmt::Float,
    };
    let writer = Arc::new(Mutex::new(Some(
        WavWriter::create(&wav_path, spec)
            .with_context(|| format!("creating WAV at {}", wav_path.display()))?,
    )));
    let frame_count = Arc::new(std::sync::atomic::AtomicU64::new(0));

    let stream = build_stream(&device, &stream_config, sample_format, channels, writer.clone(), frame_count.clone())?;

    stream.play().context("starting capture stream")?;
    let started = Instant::now();

    let _ = stop_rx.recv();
    let duration = started.elapsed();

    drop(stream);

    let frames = frame_count.load(std::sync::atomic::Ordering::Relaxed);
    {
        let mut guard = writer.lock().unwrap();
        if let Some(w) = guard.take() {
            w.finalize().context("finalizing WAV")?;
        }
    }

    anyhow::ensure!(frames > 0, "recorded zero frames from '{target_name}'");

    Ok(RecordingStats {
        path: wav_path,
        duration,
        frame_count: frames,
        channels,
        sample_rate,
    })
}

type SharedWriter = Arc<Mutex<Option<WavWriter<std::io::BufWriter<std::fs::File>>>>>;

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: SampleFormat,
    channels: u16,
    writer: SharedWriter,
    frame_count: Arc<std::sync::atomic::AtomicU64>,
) -> Result<cpal::Stream> {
    let err_fn = |e| tracing::error!("cpal stream error: {e}");

    let stream = match format {
        SampleFormat::F32 => {
            device.build_input_stream(
                config,
                move |data: &[f32], _| write_f32(data, channels, &writer, &frame_count),
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            device.build_input_stream(
                config,
                move |data: &[i16], _| {
                    let converted: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    write_f32(&converted, channels, &writer, &frame_count);
                },
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            device.build_input_stream(
                config,
                move |data: &[u16], _| {
                    let converted: Vec<f32> = data.iter()
                        .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                        .collect();
                    write_f32(&converted, channels, &writer, &frame_count);
                },
                err_fn,
                None,
            )
        }
        other => anyhow::bail!("unsupported sample format: {other:?}"),
    };

    stream.context("building input stream")
}

fn write_f32(
    data: &[f32],
    channels: u16,
    writer: &SharedWriter,
    frame_count: &Arc<std::sync::atomic::AtomicU64>,
) {
    let mut guard = writer.lock().unwrap();
    if let Some(w) = guard.as_mut() {
        for &sample in data {
            let _ = w.write_sample(sample);
        }
        frame_count.fetch_add(
            (data.len() / channels as usize) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

pub fn default_wav_path() -> Result<PathBuf> {
    let dirs = crate::config::project_dirs()?;
    Ok(dirs.cache_dir().join("latest.wav"))
}
