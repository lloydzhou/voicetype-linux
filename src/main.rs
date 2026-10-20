//! voicetype — hold-to-talk voice input for Linux (X11 + Wayland).
//!
//! Wiring: hotkey (evdev thread) / audio (cpal callback) / ASR (whisper
//! thread) / tray (muda thread) all talk to the GTK main thread through
//! glib channels.

mod asr;
mod audio;
mod config;
mod hotkey;
mod inject;
mod llm;
mod tray;
mod ui;

use asr::AsrUpdate;
use config::{shared, Config, SharedConfig};
use gtk::glib;
use hotkey::HotkeyEvent;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::JoinHandle;
use tray::TrayAction;

enum Session {
    None,
    Recording {
        rec: Arc<audio::AudioRecorder>,
        asr_handle: JoinHandle<()>,
    },
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // ---- debug CLI: `voicetype --transcribe <audio.wav>` ----
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--transcribe" {
        run_transcribe_cli(&args[2]);
        return;
    }
    if args.len() == 3 && args[1] == "--transcribe-sv" {
        run_transcribe_sv_cli(&args[2]);
        return;
    }
    if args.iter().any(|a| a == "--transcribe" || a == "--transcribe-sv")
        || args.iter().any(|a| a == "--help" || a == "-h")
    {
        print_usage_and_exit();
    }

    log::info!("voicetype starting");

    let config = shared(Config::load_or_create());

    gtk::init().expect("gtk init");

    // ---- glib channels ----
    let (hk_tx, hk_rx) = glib::MainContext::channel::<HotkeyEvent>(glib::Priority::DEFAULT);
    let (level_tx, level_rx) = glib::MainContext::channel::<f32>(glib::Priority::DEFAULT);
    let (asr_tx, asr_rx) = glib::MainContext::channel::<AsrUpdate>(glib::Priority::DEFAULT);
    let (tray_tx, tray_rx) = glib::MainContext::channel::<TrayAction>(glib::Priority::DEFAULT);
    let (hide_tx, hide_rx) = glib::MainContext::channel::<()>(glib::Priority::DEFAULT);

    // ---- background threads ----
    let hk_handle = hotkey::spawn(hk_tx);

    // ---- tray ----
    let tray = match tray::setup(config.clone(), tray_tx) {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("{e}; continuing without tray icon");
            None
        }
    };

    // ---- UI ----
    let capsule = Rc::new(ui::Capsule::new());
    let settings_storage = ui::SettingsWindow::new(config.clone());

    // Shared application state, manipulated only on the GTK main thread.
    let app = Rc::new(RefCell::new(AppState {
        session: Session::None,
    }));

    // ---- audio levels -> capsule ----
    {
        let capsule = Rc::clone(&capsule);
        level_rx.attach(None, move |rms| {
            capsule.set_level(rms);
            glib::ControlFlow::Continue
        });
    }

    // ---- hotkey events ----
    {
        let app = Rc::clone(&app);
        let capsule = Rc::clone(&capsule);
        let config = config.clone();
        let asr_tx = asr_tx.clone();
        let level_tx = level_tx.clone();
        hk_rx.attach(None, move |ev| {
            handle_hotkey(ev, &mut app.borrow_mut(), &capsule, &config, &asr_tx, &level_tx);
            glib::ControlFlow::Continue
        });
    }

    // ---- ASR updates ----
    {
        let capsule = Rc::clone(&capsule);
        let config = config.clone();
        let app = Rc::clone(&app);
        asr_rx.attach(None, move |upd| {
            handle_asr(upd, &mut app.borrow_mut(), &capsule, &config, &hide_tx);
            glib::ControlFlow::Continue
        });
    }

    // ---- worker threads ask the UI thread to hide the capsule ----
    {
        let capsule = Rc::clone(&capsule);
        hide_rx.attach(None, move |()| {
            log::info!("hide requested (worker → ui)");
            capsule.hide();
            glib::ControlFlow::Continue
        });
    }

    // ---- tray actions ----
    {
        let settings = settings_storage.clone();
        tray_rx.attach(None, move |action| {
            match action {
                TrayAction::Language(idx) => {
                    if let Ok(mut c) = config.lock() {
                        if let Some((key, _, _)) = config::LANGUAGES.get(idx) {
                            c.language = key.to_string();
                            log::info!("language -> {key}");
                        }
                    }
                }
                TrayAction::LlmToggled(on) => log::info!("LLM refinement -> {on}"),
                TrayAction::OpenSettings => ui::SettingsWindow::show(&settings),
                TrayAction::Quit => {
                    log::info!("quit requested");
                    gtk::main_quit();
                }
            }
            glib::ControlFlow::Continue
        });
    }

    log::info!("ready — hold RightCtrl+RightShift to talk; release either key to submit");
    gtk::main();

    drop(tray);
    drop(hk_handle);
}

fn print_usage_and_exit() -> ! {
    eprintln!("usage: voicetype                 # normal hold-to-talk mode");
    eprintln!("       voicetype --transcribe <audio.wav>   # transcribe a WAV file (16-bit PCM) and print text");
    eprintln!("       voicetype --transcribe-sv <audio.wav> # same, through SenseVoice");
    std::process::exit(2);
}

fn run_transcribe_cli(wav_path: &str) {
    let config = Config::load_or_create();
    let (lang, model_path) = (config.language.clone(), config.model_path.clone());
    match asr::transcribe_file(wav_path, &model_path, &lang) {
        Ok(text) => {
            println!("{text}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("transcribe failed: {e}");
            std::process::exit(1);
        }
    }
}

fn run_transcribe_sv_cli(wav_path: &str) {
    let config = Config::load_or_create();
    match asr::transcribe_file_sensevoice(wav_path, &config.sensevoice_dir, &config.language) {
        Ok(text) => {
            println!("{text}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("transcribe failed: {e}");
            std::process::exit(1);
        }
    }
}

struct AppState {
    session: Session,
}

fn handle_hotkey(
    ev: HotkeyEvent,
    app: &mut AppState,
    capsule: &Rc<ui::Capsule>,
    config: &SharedConfig,
    asr_tx: &glib::Sender<AsrUpdate>,
    level_tx: &glib::Sender<f32>,
) {
    match ev {
        HotkeyEvent::Start => {
            log::info!("hotkey start");
            if matches!(app.session, Session::None) {
                match audio::AudioRecorder::start(level_tx.clone()) {
                    Ok(rec) => {
                        let rec = Arc::new(rec);
                        capsule.show();
                        capsule.set_text("listening...");
                        let (lang, engine, model_path, sv_dir) = {
                            let c = config.lock().expect("config lock");
                            (
                                c.language.clone(),
                                c.engine.clone(),
                                c.model_path.clone(),
                                c.sensevoice_dir.clone(),
                            )
                        };
                        let rec_handle = Arc::new(rec.handle());
                        // Engine selection: "whisper" keeps the legacy
                        // chunked whisper.cpp path; anything else (default
                        // "sensevoice") uses SenseVoiceSmall.
                        let handle = if engine == "whisper" {
                            asr::spawn_session(rec_handle, model_path, lang, asr_tx.clone())
                        } else {
                            asr::spawn_sensevoice_session(rec_handle, sv_dir, lang, asr_tx.clone())
                        };
                        app.session = Session::Recording {
                            rec,
                            asr_handle: handle,
                        };
                    }
                    Err(e) => log::error!("{e}"),
                }
            }
        }
        HotkeyEvent::Stop => {
            log::info!("hotkey stop");
            if let Session::Recording { rec, .. } = &app.session {
                rec.finish(); // ASR thread performs final inference
            }
            // Capsule stays visible showing partial text; Final update closes it.
        }
    }
}

fn handle_asr(
    upd: AsrUpdate,
    app: &mut AppState,
    capsule: &Rc<ui::Capsule>,
    config: &SharedConfig,
    hide_tx: &glib::Sender<()>,
) {
    match upd {
        AsrUpdate::Partial(text) => {
            if !text.is_empty() {
                capsule.set_text(&text);
            }
        }
        AsrUpdate::Failed(e) => {
            log::error!("ASR: {e}");
            capsule.hide();
            take_session(app);
        }
        AsrUpdate::Final(text) => {
            let session = take_session(app);
            let raw = text.trim().to_string();
            if raw.is_empty() {
                log::info!("empty transcription; nothing to inject");
                capsule.set_text("no speech detected");
                // Show the hint briefly before hiding, so the user sees why
                // nothing was typed.
                let capsule = Rc::clone(capsule);
                glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                    capsule.hide();
                });
                return;
            }
            log::info!("asr final: {raw:?}");
            capsule.set_text(&raw);

            let (llm_enabled, llm_cfg, ime_switch) = {
                let c = config.lock().expect("config lock");
                let cfg = c.llm.clone();
                (
                    cfg.enabled && !cfg.api_key.trim().is_empty(),
                    cfg,
                    c.ime_switch,
                )
            };

            if llm_enabled {
                capsule.set_refining();
                // Refine + inject off the UI thread; failures fall back to raw.
                // The capsule stays up until this thread asks for the hide.
                let hide_tx = hide_tx.clone();
                std::thread::spawn(move || {
                    let refined = llm::refine(&llm_cfg, &raw);
                    log::info!("llm refined: {refined:?}");
                    // Never inject nothing: fall back to the raw text if the
                    // refinement came back empty/whitespace.
                    if refined.trim().is_empty() {
                        log::warn!("LLM refinement empty; falling back to raw text");
                        inject::inject(&raw, ime_switch);
                    } else {
                        inject::inject(&refined, ime_switch);
                    }
                    let _ = hide_tx.send(());
                });
            } else {
                let hide_tx = hide_tx.clone();
                std::thread::spawn(move || {
                    inject::inject(&raw, ime_switch);
                    let _ = hide_tx.send(());
                });
            }
            drop(session);
        }
    }
}

fn take_session(app: &mut AppState) -> Session {
    std::mem::replace(&mut app.session, Session::None)
}
