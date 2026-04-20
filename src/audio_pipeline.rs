//! Convert a recorded WAV into the shape whisper wants: 16 kHz mono F32.

use std::path::Path;

use anyhow::{Context, Result};
use hound::WavReader;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

pub const WHISPER_RATE: u32 = 16_000;

/// Full pipeline: WAV on disk → mono f32 samples at 16 kHz.
pub fn load_wav_for_whisper(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.sample_format == hound::SampleFormat::Float && spec.bits_per_sample == 32,
        "expected F32 WAV, got {:?} @ {} bits (at {})",
        spec.sample_format,
        spec.bits_per_sample,
        path.display(),
    );

    let interleaved: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .context("reading samples")?;

    let mono = downmix_to_mono(&interleaved, spec.channels as usize);
    resample(&mono, spec.sample_rate, WHISPER_RATE)
}

/// Average interleaved channels down to a single-channel signal.
pub fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let mut out = Vec::with_capacity(interleaved.len() / channels);
    for frame in interleaved.chunks_exact(channels) {
        let sum: f32 = frame.iter().sum();
        out.push(sum / channels as f32);
    }
    out
}

/// Resample a mono signal between arbitrary rates using a sinc interpolator.
pub fn resample(input: &[f32], input_rate: u32, output_rate: u32) -> Result<Vec<f32>> {
    if input_rate == output_rate {
        return Ok(input.to_vec());
    }
    anyhow::ensure!(input_rate > 0, "input rate is zero");
    anyhow::ensure!(output_rate > 0, "output rate is zero");

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let chunk = 1024usize;
    let ratio = output_rate as f64 / input_rate as f64;
    let mut resampler =
        SincFixedIn::<f32>::new(ratio, 2.0, params, chunk, 1).context("SincFixedIn::new")?;

    let expected = (input.len() as f64 * ratio).ceil() as usize + 256;
    let mut out = Vec::with_capacity(expected);
    let mut pos = 0usize;
    while pos + chunk <= input.len() {
        let block = &input[pos..pos + chunk];
        let result = resampler
            .process(&[block], None)
            .context("resample chunk")?;
        out.extend_from_slice(&result[0]);
        pos += chunk;
    }
    if pos < input.len() {
        let tail = &input[pos..];
        let result = resampler
            .process_partial(Some(&[tail]), None)
            .context("resample partial")?;
        out.extend_from_slice(&result[0]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_passes_mono_unchanged() {
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(downmix_to_mono(&input, 1), input);
    }

    #[test]
    fn downmix_averages_stereo() {
        let input = vec![0.0, 1.0, 0.5, -0.5];
        let mono = downmix_to_mono(&input, 2);
        assert_eq!(mono, vec![0.5, 0.0]);
    }

    #[test]
    fn downmix_averages_5_1() {
        let input = vec![1.0; 12]; // 2 frames of 6 channels
        let mono = downmix_to_mono(&input, 6);
        assert_eq!(mono, vec![1.0, 1.0]);
    }

    #[test]
    fn resample_passthrough_when_rates_match() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = resample(&input, 16_000, 16_000).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn resample_48k_to_16k_shrinks_threefold() {
        // 48k → 16k is a 3:1 ratio, so output len ≈ input len / 3.
        let input = vec![0.0f32; 48_000]; // 1 second of silence
        let out = resample(&input, 48_000, 16_000).unwrap();
        // Allow some slack for resampler delay/padding.
        assert!(
            (out.len() as i64 - 16_000).abs() < 256,
            "expected ~16000, got {}",
            out.len()
        );
    }

    /// Integration: write a WAV that matches what our recorder produces,
    /// then run it through the full load pipeline and check the result.
    #[test]
    fn load_wav_for_whisper_end_to_end() {
        use hound::{SampleFormat, WavSpec, WavWriter};
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_owned();

        // 1 second of sine at 48 kHz stereo, F32 — matches recorder output.
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        {
            let mut w = WavWriter::create(&path, spec).unwrap();
            for n in 0..48_000u32 {
                let s = (2.0 * std::f32::consts::PI * 440.0 * n as f32 / 48_000.0).sin();
                w.write_sample(s).unwrap(); // L
                w.write_sample(s).unwrap(); // R (same content, so mono ≈ same sine)
            }
            w.finalize().unwrap();
        }

        let samples = load_wav_for_whisper(&path).unwrap();
        // Expect ~16_000 samples (1 sec at 16 kHz), allow filter-delay slack.
        assert!(
            (samples.len() as i64 - 16_000).abs() < 256,
            "expected ~16000, got {}",
            samples.len()
        );
        // Stereo of equal L/R downmixed to mono should still be non-trivial.
        let rms = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
        assert!(rms > 0.4, "rms {rms} — pipeline may have dropped signal");
    }

    #[test]
    fn load_wav_rejects_non_f32_files() {
        use hound::{SampleFormat, WavSpec, WavWriter};
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_owned();
        let spec = WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        {
            let mut w = WavWriter::create(&path, spec).unwrap();
            w.write_sample(0i16).unwrap();
            w.finalize().unwrap();
        }
        let err = load_wav_for_whisper(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("F32"), "error should mention F32: {msg}");
    }

    #[test]
    fn resample_preserves_signal_energy_ballpark() {
        // A simple 440 Hz sine at 48 kHz → 16 kHz should still have non-trivial RMS.
        let sr_in = 48_000.0f32;
        let freq = 440.0f32;
        let input: Vec<f32> = (0..sr_in as usize)
            .map(|n| (2.0 * std::f32::consts::PI * freq * n as f32 / sr_in).sin())
            .collect();
        let out = resample(&input, 48_000, 16_000).unwrap();
        let rms = (out.iter().map(|x| x * x).sum::<f32>() / out.len() as f32).sqrt();
        // A pure sine has RMS ≈ 1/√2 ≈ 0.707. Allow generous slack for filter transient.
        assert!(rms > 0.4, "rms was {rms}");
    }
}
