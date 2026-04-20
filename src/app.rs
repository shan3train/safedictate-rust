//! eframe application — mini feather icon window that expands to full panel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Result;
use eframe::egui;
use egui_phosphor::regular as icons;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::audio::InputDevice;
use crate::config::Settings;
use crate::diagnostics::{run_all, Check, Severity};
use crate::recorder::{RecordingHandle, RecordingStats};
use crate::theme;
use crate::whisper::Transcriber;

const MINI_SIZE: [f32; 2] = [72.0, 72.0];
const FULL_SIZE: [f32; 2] = [460.0, 620.0];

#[derive(Debug, Clone)]
struct DownloadProgress {
    size: String,
    downloaded: u64,
    total: Option<u64>,
    done: Option<Result<(), String>>,
}

#[derive(Debug, Clone, Default)]
pub struct RunStats {
    pub audio_duration_ms: u32,
    pub audio_channels: u16,
    pub audio_sample_rate: u32,
    pub audio_frames: u64,
    pub load_resample_ms: u32,
    pub model_load_ms: Option<u32>,
    pub inference_ms: u32,
    pub type_ms: u32,
    pub char_count: usize,
    pub word_count: usize,
    pub model_size: String,
}

impl RunStats {
    pub fn speed_factor(&self) -> f32 {
        if self.inference_ms == 0 { 0.0 } else {
            self.audio_duration_ms as f32 / self.inference_ms as f32
        }
    }
    pub fn words_per_minute(&self) -> f32 {
        if self.audio_duration_ms == 0 { 0.0 } else {
            self.word_count as f32 / (self.audio_duration_ms as f32 / 60_000.0)
        }
    }
    pub fn end_to_end_ms(&self) -> u32 {
        self.load_resample_ms + self.inference_ms + self.type_ms + self.model_load_ms.unwrap_or(0)
    }
}

pub fn run() -> Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(MINI_SIZE)
            .with_title("SafeDictate")
            .with_decorations(false)
            .with_always_on_top(),
        ..Default::default()
    };

    eframe::run_native(
        "SafeDictate",
        native_options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(DictateApp::new()))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

struct DictateApp {
    settings: Settings,
    prev_settings: Settings,
    expanded: bool,
    checks: Arc<Mutex<Vec<Check>>>,
    devices: Arc<Mutex<Option<Result<Vec<InputDevice>, String>>>>,
    hotkey_manager: Option<GlobalHotKeyManager>,
    active_hotkey: Option<HotKey>,
    recorder: Option<RecordingHandle>,
    last_recording: Option<Result<RecordingStats, String>>,
    transcriber: Arc<Mutex<Option<Transcriber>>>,
    transcribing: Arc<AtomicBool>,
    last_transcript: Arc<Mutex<Option<Result<String, String>>>>,
    last_stats: Arc<Mutex<Option<RunStats>>>,
    download: Arc<Mutex<Option<DownloadProgress>>>,
    status: String,
}

impl DictateApp {
    fn new() -> Self {
        let settings = crate::config::load().unwrap_or_else(|e| {
            tracing::warn!("falling back to default config: {e:#}");
            Settings::default()
        });
        let prev_settings = settings.clone();
        let hotkey_manager = match GlobalHotKeyManager::new() {
            Ok(m) => Some(m),
            Err(e) => { tracing::error!("hotkey manager init failed: {e}"); None }
        };
        let mut app = Self {
            settings,
            prev_settings,
            expanded: false,
            checks: Arc::new(Mutex::new(Vec::new())),
            devices: Arc::new(Mutex::new(None)),
            hotkey_manager,
            active_hotkey: None,
            recorder: None,
            last_recording: None,
            transcriber: Arc::new(Mutex::new(None)),
            transcribing: Arc::new(AtomicBool::new(false)),
            last_transcript: Arc::new(Mutex::new(None)),
            last_stats: Arc::new(Mutex::new(None)),
            download: Arc::new(Mutex::new(None)),
            status: "Ready".into(),
        };
        app.spawn_diagnostics();
        app.spawn_device_refresh();
        app.apply_hotkey();
        app
    }

    fn spawn_diagnostics(&self) {
        let out = Arc::clone(&self.checks);
        thread::spawn(move || {
            let result = run_all();
            if let Ok(mut g) = out.lock() { *g = result; }
        });
    }

    fn spawn_device_refresh(&self) {
        let out = Arc::clone(&self.devices);
        if let Ok(mut g) = out.lock() { *g = None; }
        thread::spawn(move || {
            let result = crate::audio::list_input_devices().map_err(|e| format!("{e:#}"));
            if let Ok(mut g) = out.lock() { *g = Some(result); }
        });
    }

    fn apply_hotkey(&mut self) {
        let Some(manager) = &self.hotkey_manager else {
            self.status = "Hotkey manager unavailable".into();
            return;
        };
        if let Some(prev) = self.active_hotkey.take() {
            let _ = manager.unregister(prev);
        }
        match crate::hotkey::parse(&self.settings.hotkey) {
            Ok(hk) => match manager.register(hk) {
                Ok(()) => {
                    self.active_hotkey = Some(hk);
                    self.status = format!("Hold {} to dictate", self.settings.hotkey);
                }
                Err(e) => self.status = format!("register '{}' failed: {e}", self.settings.hotkey),
            },
            Err(e) => self.status = format!("invalid hotkey: {e}"),
        }
    }

    /// Called every frame — auto-saves and re-applies if settings changed.
    fn check_settings_changed(&mut self) {
        if self.settings == self.prev_settings {
            return;
        }
        let hotkey_changed = self.settings.hotkey != self.prev_settings.hotkey;
        if hotkey_changed {
            self.apply_hotkey();
        }
        match crate::config::save(&self.settings) {
            Ok(_) => {}
            Err(e) => tracing::warn!("auto-save failed: {e:#}"),
        }
        self.prev_settings = self.settings.clone();
    }

    fn set_expanded(&mut self, ctx: &egui::Context, expanded: bool) {
        self.expanded = expanded;
        let size = if expanded { FULL_SIZE } else { MINI_SIZE };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
        if expanded {
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
        }
    }

    fn pump_hotkey_events(&mut self) {
        let Some(active) = self.active_hotkey else { return };
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.id != active.id { continue; }
            match event.state {
                HotKeyState::Pressed => self.start_recording(),
                HotKeyState::Released => self.finish_recording(),
            }
        }
    }

    fn start_recording(&mut self) {
        if self.recorder.is_some() { return; }
        let target = self.settings.mic_name.clone();
        if target.is_empty() || target == "default" {
            self.status = "pick a microphone in settings".into();
            return;
        }
        let path = match crate::recorder::default_wav_path() {
            Ok(p) => p,
            Err(e) => { self.status = format!("cache path failed: {e:#}"); return; }
        };
        if let Ok(mut g) = self.last_transcript.lock() { *g = None; }
        match RecordingHandle::start(target.clone(), path) {
            Ok(h) => {
                self.status = format!("{} recording…", icons::RECORD);
                self.recorder = Some(h);
            }
            Err(e) => self.status = format!("start failed: {e:#}"),
        }
    }

    fn finish_recording(&mut self) {
        let Some(h) = self.recorder.take() else { return };
        match h.stop() {
            Ok(stats) => {
                self.status = format!("saved {:.2}s — transcribing…", stats.duration.as_secs_f32());
                self.spawn_transcription(stats.clone());
                self.last_recording = Some(Ok(stats));
            }
            Err(e) => {
                let msg = format!("{e:#}");
                self.status = format!("stop failed: {msg}");
                self.last_recording = Some(Err(msg));
            }
        }
    }

    fn spawn_transcription(&self, rec: RecordingStats) {
        let size = self.settings.model_size.clone();
        let ctx = Arc::clone(&self.transcriber);
        let out = Arc::clone(&self.last_transcript);
        let stats_out = Arc::clone(&self.last_stats);
        let flag = Arc::clone(&self.transcribing);
        flag.store(true, Ordering::SeqCst);
        thread::spawn(move || {
            let (result, stats) = transcription_pipeline(ctx, size, rec);
            if let Ok(mut g) = out.lock() { *g = Some(result.clone()); }
            if let Ok(mut g) = stats_out.lock() { *g = Some(stats); }
            flag.store(false, Ordering::SeqCst);
            match &result {
                Ok(text) => tracing::info!("transcript: {}", text.chars().take(120).collect::<String>()),
                Err(e) => tracing::error!("transcription failed: {e}"),
            }
        });
    }

    fn spawn_model_download(&self, size: String) {
        let state = Arc::clone(&self.download);
        if let Ok(mut g) = state.lock() {
            *g = Some(DownloadProgress { size: size.clone(), downloaded: 0, total: None, done: None });
        }
        thread::spawn(move || {
            let state_cb = Arc::clone(&state);
            let size_inner = size.clone();
            let result = crate::whisper::download_model(&size, |downloaded, total| {
                if let Ok(mut g) = state_cb.lock() {
                    if let Some(p) = g.as_mut() { p.downloaded = downloaded; p.total = total; }
                }
            });
            if let Ok(mut g) = state.lock() {
                if let Some(p) = g.as_mut() {
                    p.done = Some(match result { Ok(_) => Ok(()), Err(e) => Err(format!("{e:#}")) });
                }
            }
            tracing::info!("model download '{size_inner}' finished");
        });
    }
}

fn transcription_pipeline(
    ctx: Arc<Mutex<Option<Transcriber>>>,
    size: String,
    rec: RecordingStats,
) -> (Result<String, String>, RunStats) {
    use std::time::Instant;
    let mut stats = RunStats {
        audio_duration_ms: rec.duration.as_millis() as u32,
        audio_channels: rec.channels,
        audio_sample_rate: rec.sample_rate,
        audio_frames: rec.frame_count,
        model_size: size.clone(),
        ..Default::default()
    };
    let result: Result<String, String> = (|| -> anyhow::Result<String> {
        {
            let mut g = ctx.lock().map_err(|_| anyhow::anyhow!("transcriber mutex poisoned"))?;
            let needs_load = g.as_ref().map(|t| t.size() != size).unwrap_or(true);
            if needs_load {
                *g = None;
                tracing::info!("loading whisper model '{size}'");
                let t0 = Instant::now();
                *g = Some(Transcriber::load(&size)?);
                stats.model_load_ms = Some(t0.elapsed().as_millis() as u32);
            }
        }
        let t0 = Instant::now();
        let samples = crate::audio_pipeline::load_wav_for_whisper(&rec.path)?;
        stats.load_resample_ms = t0.elapsed().as_millis() as u32;
        tracing::info!("whisper input: {} samples = {:.2}s", samples.len(),
            samples.len() as f32 / crate::audio_pipeline::WHISPER_RATE as f32);
        let text = {
            let g = ctx.lock().map_err(|_| anyhow::anyhow!("transcriber mutex poisoned"))?;
            let t = g.as_ref().ok_or_else(|| anyhow::anyhow!("transcriber unexpectedly empty"))?;
            let t0 = Instant::now();
            let text = t.transcribe(&samples, "en")?;
            stats.inference_ms = t0.elapsed().as_millis() as u32;
            text
        };
        stats.char_count = text.chars().count();
        stats.word_count = text.split_whitespace().filter(|w| !w.is_empty()).count();
        if !text.is_empty() {
            let t0 = Instant::now();
            crate::keystroke::type_text(&text)?;
            stats.type_ms = t0.elapsed().as_millis() as u32;
        }
        Ok(text)
    })().map_err(|e| format!("{e:#}"));
    (result, stats)
}

impl eframe::App for DictateApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_hotkey_events();
        self.check_settings_changed();

        let download_in_flight = self.download.lock().ok()
            .and_then(|g| g.as_ref().map(|p| p.done.is_none())).unwrap_or(false);
        let active = self.recorder.is_some()
            || self.transcribing.load(Ordering::SeqCst)
            || download_in_flight;
        if active {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        if self.expanded {
            self.ui_full(ctx);
        } else {
            self.ui_mini(ctx);
        }
    }
}

impl DictateApp {
    fn ui_mini(&mut self, ctx: &egui::Context) {
        let recording = self.recorder.is_some();
        let transcribing = self.transcribing.load(Ordering::SeqCst);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ctx.style().visuals.window_fill()))
            .show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    let icon = if recording {
                        egui::RichText::new(icons::RECORD.to_string())
                            .size(36.0)
                            .color(egui::Color32::from_rgb(243, 139, 168))
                    } else if transcribing {
                        egui::RichText::new(icons::SPINNER.to_string())
                            .size(36.0)
                            .color(egui::Color32::from_rgb(249, 226, 175))
                    } else {
                        egui::RichText::new(icons::FEATHER.to_string())
                            .size(36.0)
                    };

                    let btn = ui.add(
                        egui::Button::new(icon)
                            .frame(false)
                            .min_size(egui::vec2(60.0, 60.0)),
                    );

                    if btn.clicked() {
                        self.set_expanded(ctx, true);
                    }

                    btn.on_hover_text(format!("SafeDictate — {} — click to open", self.status));
                });
            });
    }

    fn ui_full(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("titlebar")
            .exact_height(44.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    ui.heading(format!("{} SafeDictate", icons::FEATHER));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        if ui.small_button(icons::MINUS.to_string())
                            .on_hover_text("Minimise to icon").clicked()
                        {
                            self.set_expanded(ctx, false);
                        }
                        ui.add_space(8.0);
                        if let Some(h) = &self.recorder {
                            let elapsed = h.elapsed().as_secs_f32();
                            ui.label(
                                egui::RichText::new(format!("{} REC {elapsed:>4.1}s", icons::RECORD))
                                    .color(egui::Color32::from_rgb(243, 139, 168))
                                    .strong(),
                            );
                        } else {
                            ui.label(egui::RichText::new(&self.status).weak());
                        }
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.ui_settings(ui);
                ui.add_space(12.0);
                self.ui_model(ui);
                ui.add_space(12.0);
                self.ui_last_transcript(ui);
                ui.add_space(12.0);
                self.ui_stats(ui);
                ui.add_space(12.0);
                self.ui_diagnostics(ui);
            });
        });
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.label(egui::RichText::new(format!("{} Settings", icons::GEAR)).strong());
            ui.add_space(4.0);
            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Model");
                    let selected_label = crate::whisper::info(&self.settings.model_size)
                        .map(|i| format!("{} ({})", i.size, crate::whisper::human_mb(i.approx_mb)))
                        .unwrap_or_else(|| self.settings.model_size.clone());
                    egui::ComboBox::from_id_salt("model_size")
                        .width(220.0)
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for m in crate::whisper::MODELS {
                                let lang = if m.english_only { "EN" } else { "multi" };
                                let label = format!(
                                    "{}  {} · {}  — {}",
                                    m.size, crate::whisper::human_mb(m.approx_mb), lang, m.description,
                                );
                                ui.selectable_value(&mut self.settings.model_size, m.size.to_string(), label);
                            }
                        });
                    ui.end_row();

                    ui.label("Microphone");
                    self.ui_mic_picker(ui);
                    ui.end_row();

                    ui.label("Hotkey");
                    egui::ComboBox::from_id_salt("hotkey_picker")
                        .selected_text(&self.settings.hotkey)
                        .show_ui(ui, |ui| {
                            for hk in &[
                                "alt+Digit1", "alt+Digit2", "alt+Digit3", "alt+Space",
                                "ctrl+Digit1", "ctrl+Digit2", "ctrl+Digit3", "ctrl+Space",
                                "ctrl+shift+Space", "ctrl+alt+Space",
                                "F1", "F2", "F3", "F4", "F5", "F6", "F9",
                            ] {
                                ui.selectable_value(&mut self.settings.hotkey, hk.to_string(), *hk);
                            }
                        });
                    ui.end_row();

                    ui.label("Max record (s)");
                    ui.add(egui::DragValue::new(&mut self.settings.max_record_seconds).range(5..=120));
                    ui.end_row();
                });
        });
    }

    fn ui_mic_picker(&mut self, ui: &mut egui::Ui) {
        let snapshot: Option<Result<Vec<InputDevice>, String>> =
            self.devices.lock().ok().and_then(|g| g.clone());
        let current_display = match &snapshot {
            Some(Ok(list)) => list.iter()
                .find(|d| d.pw_name == self.settings.mic_name)
                .map(|d| d.display.clone())
                .unwrap_or_else(|| format!("{} (not found)", self.settings.mic_name)),
            Some(Err(e)) => format!("error: {e}"),
            None => "loading…".to_string(),
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("mic_picker")
                .width(260.0)
                .selected_text(current_display)
                .show_ui(ui, |ui| match &snapshot {
                    Some(Ok(list)) if list.is_empty() => { ui.label("no audio sources"); }
                    Some(Ok(list)) => {
                        for d in list {
                            ui.selectable_value(&mut self.settings.mic_name, d.pw_name.clone(), &d.display);
                        }
                    }
                    Some(Err(e)) => { ui.label(format!("enum failed: {e}")); }
                    None => { ui.label("loading…"); }
                });
            if ui.small_button(icons::ARROW_CLOCKWISE.to_string())
                .on_hover_text("Re-enumerate input devices").clicked()
            {
                self.spawn_device_refresh();
            }
        });
    }

    fn ui_model(&mut self, ui: &mut egui::Ui) {
        let size = self.settings.model_size.clone();
        let have = crate::whisper::model_exists(&size);
        let download_snap: Option<DownloadProgress> =
            self.download.lock().ok().and_then(|g| g.clone());

        ui.group(|ui| {
            ui.label(egui::RichText::new(format!("{} Whisper model", icons::CPU)).strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if have {
                    ui.colored_label(egui::Color32::from_rgb(166, 227, 161), icons::CHECK_CIRCLE.to_string());
                    let fname = crate::whisper::info(&size).map(|i| i.filename).unwrap_or("");
                    ui.label(format!("{fname} ready"));
                } else {
                    match download_snap.as_ref() {
                        Some(p) if p.size == size && p.done.is_none() => {
                            let downloaded_mb = p.downloaded as f64 / 1.0e6;
                            let total_mb = p.total.map(|t| t as f64 / 1.0e6);
                            let pct = p.total.map(|t| p.downloaded as f32 / t as f32).unwrap_or(0.0);
                            ui.label(match total_mb {
                                Some(t) => format!("{downloaded_mb:.1} / {t:.1} MB"),
                                None => format!("{downloaded_mb:.1} MB"),
                            });
                            ui.add(egui::ProgressBar::new(pct).desired_width(140.0).show_percentage());
                        }
                        Some(p) if p.size == size && matches!(p.done, Some(Err(_))) => {
                            let err = match &p.done { Some(Err(e)) => e.clone(), _ => String::new() };
                            ui.colored_label(egui::Color32::from_rgb(243, 139, 168),
                                format!("{} {err}", icons::X_CIRCLE));
                            if ui.button("Retry").clicked() { self.spawn_model_download(size.clone()); }
                        }
                        _ => {
                            ui.label(format!("ggml-{size} not installed"));
                            let btn = ui.button(format!("{} Download", icons::DOWNLOAD_SIMPLE))
                                .on_hover_text(crate::whisper::model_url(&size));
                            if btn.clicked() { self.spawn_model_download(size.clone()); }
                        }
                    }
                }
            });
        });
    }

    fn ui_last_transcript(&mut self, ui: &mut egui::Ui) {
        let transcript = self.last_transcript.lock().ok().and_then(|g| g.clone());
        let transcribing = self.transcribing.load(Ordering::SeqCst);
        let recording = self.recorder.is_some();
        let copyable: Option<String> = match &transcript {
            Some(Ok(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{} Transcript", icons::TEXT_ALIGN_LEFT)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(text) = copyable.as_ref() {
                        if ui.small_button(format!("{} Copy", icons::COPY))
                            .on_hover_text("Copy to clipboard").clicked()
                        {
                            ui.ctx().copy_text(text.clone());
                            self.status = "copied to clipboard".into();
                        }
                    }
                });
            });
            ui.add_space(4.0);
            if recording {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(243, 139, 168), icons::MICROPHONE.to_string());
                    ui.label(egui::RichText::new("Listening…").italics().color(egui::Color32::from_rgb(243, 139, 168)));
                });
                return;
            }
            if transcribing {
                ui.horizontal(|ui| { ui.spinner(); ui.weak("transcribing…"); });
                return;
            }
            match &transcript {
                Some(Ok(s)) if s.is_empty() => { ui.weak("(empty — silence or unsupported speech?)"); }
                Some(Ok(s)) => { ui.label(s); }
                Some(Err(e)) => {
                    ui.colored_label(egui::Color32::from_rgb(243, 139, 168), format!("{} {e}", icons::X_CIRCLE));
                }
                None => { ui.weak("Hold your hotkey to dictate. Transcript appears here."); }
            }
        });
    }

    fn ui_stats(&mut self, ui: &mut egui::Ui) {
        let snapshot: Option<RunStats> = self.last_stats.lock().ok().and_then(|g| g.clone());
        ui.group(|ui| {
            ui.label(egui::RichText::new(format!("{} Stats", icons::CHART_BAR)).strong());
            ui.add_space(4.0);
            if let Some(Err(e)) = &self.last_recording {
                ui.colored_label(egui::Color32::from_rgb(243, 139, 168),
                    format!("{} recording failed: {e}", icons::X_CIRCLE));
                return;
            }
            let Some(s) = snapshot else {
                ui.weak("Run a dictation cycle to populate stats.");
                return;
            };
            let fmt_ms = |ms: u32| -> String {
                if ms >= 1000 { format!("{:.2} s", ms as f32 / 1000.0) } else { format!("{ms} ms") }
            };
            let wav_path = self.last_recording.as_ref()
                .and_then(|r| r.as_ref().ok()).map(|r| r.path.clone());
            egui::Grid::new("stats_grid").num_columns(2).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
                ui.weak("Audio"); ui.label(format!("{:.2}s · {}ch · {}Hz", s.audio_duration_ms as f32/1000.0, s.audio_channels, s.audio_sample_rate)); ui.end_row();
                if let Some(p) = wav_path { ui.weak("WAV"); ui.monospace(p.display().to_string()); ui.end_row(); }
                ui.weak("Load+resample"); ui.label(fmt_ms(s.load_resample_ms)); ui.end_row();
                if let Some(load) = s.model_load_ms { ui.weak("Model load"); ui.label(format!("{} (this run)", fmt_ms(load))); ui.end_row(); }
                ui.weak("GPU inference"); ui.label(format!("{}  ·  {:.1}× realtime", fmt_ms(s.inference_ms), s.speed_factor())); ui.end_row();
                ui.weak("End-to-end"); ui.label(egui::RichText::new(fmt_ms(s.end_to_end_ms())).strong()); ui.end_row();
                ui.weak("Words"); ui.label(format!("{} ({:.0} wpm)", s.word_count, s.words_per_minute())); ui.end_row();
                ui.weak("Model"); ui.label(&s.model_size); ui.end_row();
            });
        });
    }

    fn ui_diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{} Diagnostics", icons::STETHOSCOPE)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(format!("{} Re-run", icons::ARROW_CLOCKWISE)).clicked() {
                        self.spawn_diagnostics();
                    }
                });
            });
            ui.add_space(6.0);
            let checks: Option<Vec<Check>> = self.checks.lock().ok().map(|g| g.clone());
            match checks.as_deref() {
                Some(list) if list.is_empty() => { ui.weak("Running checks…"); }
                Some(list) => {
                    for (i, c) in list.iter().enumerate() {
                        let (icon, color) = match c.severity {
                            Severity::Ok => (icons::CHECK_CIRCLE, egui::Color32::from_rgb(166, 227, 161)),
                            Severity::Warn => (icons::WARNING, egui::Color32::from_rgb(249, 226, 175)),
                            Severity::Fail => (icons::X_CIRCLE, egui::Color32::from_rgb(243, 139, 168)),
                        };
                        ui.horizontal(|ui| {
                            ui.colored_label(color, icon);
                            ui.label(egui::RichText::new(&c.name).strong());
                        });
                        if !c.detail.is_empty() {
                            ui.indent(("detail", i), |ui| { ui.weak(&c.detail); });
                        }
                        ui.add_space(3.0);
                    }
                }
                None => { ui.label("diagnostics state unavailable"); }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stats() -> RunStats {
        RunStats {
            audio_duration_ms: 5_000, audio_channels: 2, audio_sample_rate: 48_000,
            audio_frames: 240_000, load_resample_ms: 40, model_load_ms: None,
            inference_ms: 500, type_ms: 20, char_count: 60, word_count: 12,
            model_size: "small".into(),
        }
    }

    #[test]
    fn speed_factor_above_one_means_faster_than_realtime() {
        let s = sample_stats();
        assert!((s.speed_factor() - 10.0).abs() < 1e-4);
    }

    #[test]
    fn speed_factor_zero_when_no_inference_recorded() {
        let mut s = sample_stats(); s.inference_ms = 0;
        assert_eq!(s.speed_factor(), 0.0);
    }

    #[test]
    fn words_per_minute_math() {
        let s = sample_stats();
        assert!((s.words_per_minute() - 144.0).abs() < 1e-3);
    }

    #[test]
    fn end_to_end_sums_all_components_including_model_load() {
        let mut s = sample_stats(); s.model_load_ms = Some(250);
        assert_eq!(s.end_to_end_ms(), 810);
    }

    #[test]
    fn end_to_end_skips_model_load_when_none() {
        let s = sample_stats();
        assert_eq!(s.end_to_end_ms(), 560);
    }
}
