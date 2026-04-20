use std::process::ExitCode;

mod app;
mod audio;
mod audio_pipeline;
mod config;
mod diagnostics;
mod hotkey;
mod icon;
mod keystroke;
mod recorder;
mod theme;
mod whisper;

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("safedictate=info,warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

fn main() -> ExitCode {
    init_tracing();
    audio::init_once();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--doctor" || a == "-d") {
        return run_doctor();
    }
    if args.iter().any(|a| a == "--record-test") {
        return run_record_test();
    }
    if let Some(idx) = args.iter().position(|a| a == "--transcribe-test") {
        let path_arg = args.get(idx + 1).cloned();
        return run_transcribe_test(path_arg);
    }
    if let Some(idx) = args.iter().position(|a| a == "--download-model") {
        let size = args
            .get(idx + 1)
            .cloned()
            .unwrap_or_else(|| "base".to_string());
        return run_download_model(&size);
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }

    // System tray icon — lives for the duration of the process.
    let _tray = {
        use tray_icon::{TrayIconBuilder, menu::{Menu, MenuItem}};
        let (rgba, w, h) = icon::feather_rgba();
        let tray_img = tray_icon::Icon::from_rgba(rgba, w, h)
            .expect("tray icon");
        let menu = Menu::new();
        let quit_item = MenuItem::new("Quit SafeDictate", true, None);
        let quit_id = quit_item.id().clone();
        menu.append(&quit_item).ok();
        let tray = TrayIconBuilder::new()
            .with_icon(tray_img)
            .with_tooltip("SafeDictate")
            .with_menu(Box::new(menu))
            .build()
            .expect("tray icon build");

        // Spawn a thread to handle tray menu events (Quit).
        std::thread::spawn(move || {
            loop {
                if let Ok(event) = tray_icon::menu::MenuEvent::receiver().recv() {
                    if event.id == quit_id {
                        std::process::exit(0);
                    }
                }
            }
        });

        tray
    };

    match app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            eprintln!("SafeDictate failed to start: {e:#}");
            eprintln!();
            eprintln!("Run `safedictate --doctor` for a system diagnostic report.");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("SafeDictate v2 — local voice dictation for Windows");
    println!();
    println!("USAGE:");
    println!("  safedictate              launch the app");
    println!("  safedictate --doctor     run system diagnostics and exit");
    println!("  safedictate --record-test          record 2s from default mic → latest.wav");
    println!("  safedictate --download-model [size]  fetch ggml model (tiny/base/small/medium)");
    println!("  safedictate --transcribe-test [wav]  run whisper on a WAV (default: latest.wav)");
    println!("  safedictate --help                 show this message");
    println!();
    println!("ENV:");
    println!("  RUST_LOG=safedictate=debug  verbose logging");
}

fn run_transcribe_test(path_arg: Option<String>) -> ExitCode {
    let path = match path_arg {
        Some(s) => std::path::PathBuf::from(s),
        None => match recorder::default_wav_path() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("default wav path: {e:#}");
                return ExitCode::FAILURE;
            }
        },
    };
    if !path.exists() {
        eprintln!("no WAV at {}. Run --record-test first.", path.display());
        return ExitCode::FAILURE;
    }
    let settings = config::load().unwrap_or_default();
    let size = &settings.model_size;
    if !whisper::model_exists(size) {
        eprintln!(
            "model '{size}' not installed. Download with:\n  safedictate --download-model {size}"
        );
        return ExitCode::FAILURE;
    }
    let samples = match audio_pipeline::load_wav_for_whisper(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("audio pipeline: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "loaded {} samples = {:.2}s at 16 kHz",
        samples.len(),
        samples.len() as f32 / audio_pipeline::WHISPER_RATE as f32
    );
    let transcriber = match whisper::Transcriber::load(size) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("load model: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    match transcriber.transcribe(&samples, "en") {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("transcribe: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_download_model(size: &str) -> ExitCode {
    if whisper::model_exists(size) {
        if let Ok(p) = whisper::model_path(size) {
            println!("already have: {}", p.display());
        }
        return ExitCode::SUCCESS;
    }
    eprintln!("downloading ggml-{size}.en.bin from {}", whisper::model_url(size));
    let mut last_pct = -1i32;
    match whisper::download_model(size, |downloaded, total| {
        if let Some(t) = total {
            let pct = (downloaded * 100 / t.max(1)) as i32;
            if pct != last_pct && pct % 5 == 0 {
                eprintln!("  {pct:3}%  ({:.1} / {:.1} MB)", downloaded as f64 / 1e6, t as f64 / 1e6);
                last_pct = pct;
            }
        }
    }) {
        Ok(p) => {
            println!("✓ {}", p.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("download: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_record_test() -> ExitCode {
    let device = match audio::default_input() {
        Ok(Some(d)) => d,
        Ok(None) => {
            eprintln!("no default audio source — plug in a mic");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("enumerating devices: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let path = match recorder::default_wav_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cache path: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "recording 2s from {} ({})\n         → {}",
        device.display,
        device.pw_name,
        path.display(),
    );
    let handle = match recorder::RecordingHandle::start(device.pw_name.clone(), path) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("start: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    std::thread::sleep(std::time::Duration::from_secs(2));
    match handle.stop() {
        Ok(s) => {
            println!(
                "✓ {:.2}s, {} frames, {} ch, {} Hz → {}",
                s.duration.as_secs_f32(),
                s.frame_count,
                s.channels,
                s.sample_rate,
                s.path.display(),
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("stop: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_doctor() -> ExitCode {
    use diagnostics::{run_all, Severity};
    println!("SafeDictate — system diagnostics");
    println!("─────────────────────────────────");
    let checks = run_all();
    let mut failed = 0;
    let mut warned = 0;
    for c in &checks {
        let tag = match c.severity {
            Severity::Ok => "  OK  ",
            Severity::Warn => " WARN ",
            Severity::Fail => " FAIL ",
        };
        println!("[{tag}] {}", c.name);
        if !c.detail.is_empty() {
            for line in c.detail.lines() {
                println!("         {line}");
            }
        }
        match c.severity {
            Severity::Fail => failed += 1,
            Severity::Warn => warned += 1,
            Severity::Ok => {}
        }
    }
    println!("─────────────────────────────────");
    println!(
        "{} ok, {} warn, {} fail",
        checks.len() - failed - warned,
        warned,
        failed
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
