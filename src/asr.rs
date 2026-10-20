//! ASR engines: whisper-rs (legacy chunked path) and SenseVoiceSmall INT8
//! via sherpa-rs (live path). Hallucination filtering and
//! Simplified->Traditional conversion are shared.

use crate::audio::RecorderHandle;
use crate::config::{language_index, s2t_char_map, t2s_char_map, LANGUAGES};
use log::{error, info, warn};
use std::sync::{Arc, Mutex, OnceLock};

/// Messages from the ASR session thread to the UI.
#[derive(Debug, Clone)]
pub enum AsrUpdate {
    Partial(String),
    Final(String),
    Failed(String),
}

const PARTIAL_INTERVAL_MS: u64 = 1200;
const MIN_SAMPLES: usize = 16_000 / 2; // 0.5 s
const HALLUCINATION_SHORT_SECS: f32 = 1.5;

const HALLUCINATION_PHRASES: &[&str] = &[
    "谢谢观看",
    "谢谢收看",
    "请订阅",
    "請訂閱",
    "Thank you for watching",
    "Thank you.",
    "Thanks for watching!",
    "Bye.",
];

pub fn whisper_language(lang_key: &str) -> &'static str {
    LANGUAGES[language_index(lang_key)].2
}

pub fn is_traditional(lang_key: &str) -> bool {
    lang_key == "zh-Hant"
}

/// Simplified -> Traditional conversion (common characters).
pub fn s2t(s: &str) -> String {
    let map = s2t_char_map();
    s.chars().map(|c| *map.get(&c).unwrap_or(&c)).collect()
}

/// Traditional -> Simplified conversion. Whisper tends to emit Traditional
/// Chinese even for zh, so zh-Hans output needs this direction.
pub fn t2s(s: &str) -> String {
    let map = t2s_char_map();
    s.chars().map(|c| *map.get(&c).unwrap_or(&c)).collect()
}

/// Convert raw ASR output for the configured UI language. Simplified output
/// is passed through untouched: SenseVoice's zh output is simplified already
/// and the old t2s pass had bogus pairs that corrupted correct text. Only an
/// explicit 繁體中文 setting triggers s2t.
pub fn convert_for(lang_key: &str, t: &str) -> String {
    match lang_key {
        "zh-Hant" => s2t(t),
        _ => t.to_string(),
    }
}

fn filter_hallucination(text: &str, duration_secs: f32) -> String {
    let t = text.trim();
    if t.is_empty() {
        return String::new();
    }
    // Whisper hallucinates subtitle-style artifacts on quiet audio regardless
    // of duration: "(字幕:J Chong)", "（字幕）", "谢谢观看" etc. Reject
    // anything carrying subtitle markers, and the known phrase list on any
    // audio length (they never occur as real dictation).
    let parenthesized = (t.starts_with('(') && t.ends_with(')'))
        || (t.starts_with('（') && t.ends_with('）'));
    if t.contains("字幕") || (parenthesized && t.chars().count() <= 24) {
        warn!("filtered subtitle-style hallucination: {t:?}");
        return String::new();
    }
    for p in HALLUCINATION_PHRASES {
        if t.eq_ignore_ascii_case(p) {
            warn!("filtered hallucination: {t:?}");
            return String::new();
        }
    }
    if duration_secs < HALLUCINATION_SHORT_SECS {
        for p in HALLUCINATION_PHRASES {
            if t.contains(p) {
                warn!("filtered hallucination on short audio: {t:?}");
                return String::new();
            }
        }
    }
    t.to_string()
}

/// Process-wide whisper context cache: the model is expensive to load
/// (~1-2 s), so it is loaded once and reused across recording sessions.
/// whisper-rs 0.16 `WhisperContext` is `Send + Sync` (Arc'd inner), and each
/// inference creates its own `WhisperState`, so sharing the context is safe.
/// The load itself still happens inside an ASR worker thread — only the
/// model path crosses the thread boundary, preserving the original
/// no-!Send-capture property.
static CTX_CACHE: OnceLock<Mutex<Option<CachedCtx>>> = OnceLock::new();

struct CachedCtx {
    model_path: String,
    ctx: Arc<whisper_rs::WhisperContext>,
}

/// Fetch the whisper context for `model_path`, loading it on first use (or
/// when the configured model path changed). Call from a worker thread.
fn get_context(model_path: &str) -> Result<Arc<whisper_rs::WhisperContext>, String> {
    let cache = CTX_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().map_err(|_| "whisper context cache poisoned".to_string())?;
    if let Some(cached) = guard.as_ref() {
        if cached.model_path == model_path {
            return Ok(Arc::clone(&cached.ctx));
        }
        info!("whisper model path changed; reloading");
    }
    if !std::path::Path::new(model_path).exists() {
        return Err(format!(
            "Whisper 模型不存在: {model_path}\n请运行 `make model` 或在配置中设置 model_path"
        ));
    }
    info!("loading whisper model: {model_path}");
    let ctx = Arc::new(
        whisper_rs::WhisperContext::new_with_params(
            model_path,
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|e| format!("加载 Whisper 模型失败: {e}"))?,
    );
    info!("whisper model loaded (cached for reuse)");
    *guard = Some(CachedCtx {
        model_path: model_path.to_string(),
        ctx: Arc::clone(&ctx),
    });
    Ok(ctx)
}

/// Run whisper over `samples` (16 kHz mono f32), returning trimmed text.
pub fn transcribe_samples(
    ctx: &whisper_rs::WhisperContext,
    samples: &[f32],
    lang: &str,
    duration_secs: f32,
) -> String {
    if samples.len() < MIN_SAMPLES {
        return String::new();
    }

    let mut params = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy {
        best_of: 1, // single beam
    });
    params.set_language(Some(lang));
    params.set_n_threads(1);
    params.set_translate(false);
    params.set_no_context(true); // no context reuse between partials
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);

    let mut state = match ctx.create_state() {
        Ok(s) => s,
        Err(e) => {
            error!("whisper create_state: {e}");
            return String::new();
        }
    };
    if let Err(e) = state.full(params, samples) {
        error!("whisper full: {e}");
        return String::new();
    }
    let n = state.full_n_segments(); // plain i32 in whisper-rs 0.16
    let mut text = String::new();
    for i in 0..n {
        match state.get_segment(i) {
            Some(seg) => match seg.to_str_lossy() {
                Ok(t) => text.push_str(&t),
                Err(e) => {
                    warn!("segment text {i}: {e}");
                    break;
                }
            },
            None => break,
        }
    }
    // Convert "▁token" underscores to spaces like whisper.cpp output.
    text = text.replace('\u{2581}', " ");
    filter_hallucination(&text, duration_secs)
        .trim()
        .to_string()
}

/// Spawn the per-recording session thread:
/// partial inference every ~1.2 s while recording, final on release.
///
/// The Whisper context comes from the process-wide cache (`get_context`);
/// the (possible) model load happens inside this thread so no !Send whisper
/// types are captured across the thread boundary. Subsequent sessions reuse
/// the cached context, avoiding the ~1-2 s reload per recording.
pub fn spawn_session(
    rec: Arc<RecorderHandle>,
    model_path: String,
    lang_key: String,
    tx: glib::Sender<AsrUpdate>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("asr".into())
        .spawn(move || {
            let lang = whisper_language(&lang_key).to_string();

            let ctx = match get_context(&model_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(AsrUpdate::Failed(e));
                    return;
                }
            };
            let ctx = &*ctx;

            // ---- partial loop ----
            while !rec.finish_flag() {
                crate::audio::sleep_ms(PARTIAL_INTERVAL_MS);
                if rec.finish_flag() {
                    break;
                }
                let snap = rec.snapshot_16k();
                if snap.len() >= MIN_SAMPLES {
                    let t = transcribe_samples(ctx, &snap, &lang, snap.len() as f32 / 16_000.0);
                    if !t.is_empty() {
                        let shown = convert_for(&lang_key, &t);
                        if tx.send(AsrUpdate::Partial(shown)).is_err() {
                            return;
                        }
                    }
                }
            }

            // ---- final ----
            if rec.cancelled() {
                info!("asr session cancelled");
                return;
            }
            let all = rec.take_all();
            let dur = all.len() as f32 / 16_000.0;
            if dur < 0.3 {
                let _ = tx.send(AsrUpdate::Final(String::new()));
                return;
            }
            info!("final inference over {:.2}s of audio", dur);
            let t = transcribe_samples(ctx, &all, &lang, dur);
            let final_text = convert_for(&lang_key, &t);
            let _ = tx.send(AsrUpdate::Final(final_text));
        })
        .expect("spawn asr thread")
}

/// Strip SenseVoice tag tokens like `<|zh|>`, `<|en|>`, `<|NEUTRAL|>`,
/// `<|HAPPY|>` etc. Regex-free: drop any `<|...|>` substring.
pub fn sanitize_sensevoice(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' && chars.peek() == Some(&'|') {
            // Scan for the closing `|>`; drop everything including it.
            let mut lookahead = Vec::new();
            let mut closed = false;
            while let Some(&n) = chars.peek() {
                chars.next();
                lookahead.push(n);
                if n == '|' && chars.peek() == Some(&'>') {
                    chars.next();
                    closed = true;
                    break;
                }
                if lookahead.len() > 32 {
                    break;
                }
            }
            if !closed {
                // Not a well-formed tag; keep the raw characters.
                out.push(c);
                out.push('|');
                out.extend(lookahead);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// SenseVoice language code for a UI language key ("auto" fallback).
pub fn sense_voice_language(lang_key: &str) -> &'static str {
    match lang_key {
        "en" => "en",
        "zh-Hans" | "zh-Hant" => "zh",
        "ja" => "ja",
        "ko" => "ko",
        _ => "auto",
    }
}

/// Run SenseVoice over `samples` (16 kHz mono f32), returning sanitized,
/// trimmed text. No hallucination filter here — partials are shown raw;
/// the filter is applied to Final results only.
fn transcribe_samples_sv(
    rec: &mut sherpa_rs::sense_voice::SenseVoiceRecognizer,
    samples: &[f32],
) -> String {
    if samples.len() < MIN_SAMPLES {
        return String::new();
    }
    let result = rec.transcribe(16_000, samples);
    sanitize_sensevoice(&result.text).trim().to_string()
}

/// Process-wide SenseVoice recognizer cache, mirroring the whisper
/// CTX_CACHE pattern: the model is loaded once and reused across recording
/// sessions. The recognizer is Send+Sync but `transcribe()` takes `&mut`,
/// so calls serialize on the cache mutex. The load happens inside an ASR
/// worker thread — only the model dir crosses the thread boundary,
/// preserving the no-!Send-capture property.
static SV_CACHE: OnceLock<Mutex<Option<CachedSv>>> = OnceLock::new();

struct CachedSv {
    model_path: String,
    language: String,
    rec: sherpa_rs::sense_voice::SenseVoiceRecognizer,
}

/// Run `f` with the cached SenseVoice recognizer for `model_path`, loading
/// it on first use (or when the model path / language changed). Call from a
/// worker thread.
fn with_sensevoice_recognizer<T>(
    model_path: &str,
    language: &str,
    f: impl FnOnce(&mut sherpa_rs::sense_voice::SenseVoiceRecognizer) -> T,
) -> Result<T, String> {
    let cache = SV_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache
        .lock()
        .map_err(|_| "sensevoice recognizer cache poisoned".to_string())?;
    let matches = guard
        .as_ref()
        .map(|c| c.model_path == model_path && c.language == language)
        .unwrap_or(false);
    if !matches {
        if !std::path::Path::new(model_path).exists() {
            return Err(format!(
                "SenseVoice 模型不存在: {model_path}\n请运行 `make model-sensevoice` 或在配置中设置 sensevoice_dir"
            ));
        }
        let tokens_path = std::path::Path::new(model_path)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("tokens.txt");
        if !tokens_path.exists() {
            return Err(format!("missing tokens.txt next to {model_path}"));
        }
        info!("loading SenseVoice model: {model_path}");
        let rec = sherpa_rs::sense_voice::SenseVoiceRecognizer::new(
            sherpa_rs::sense_voice::SenseVoiceConfig {
                model: model_path.to_string(),
                tokens: tokens_path.to_string_lossy().into_owned(),
                language: language.to_string(),
                use_itn: true,
                num_threads: Some(1),
                ..Default::default()
            },
        )
        .map_err(|e| format!("加载 SenseVoice 模型失败: {e}"))?;
        info!("SenseVoice model loaded (cached for reuse)");
        *guard = Some(CachedSv {
            model_path: model_path.to_string(),
            language: language.to_string(),
            rec,
        });
    }
    let cached = guard.as_mut().expect("just populated");
    Ok(f(&mut cached.rec))
}

/// Interval between partial re-runs while recording: 0.4 s for short audio,
/// 0.8 s once the accumulated audio exceeds 5 s (offline re-runs get slower
/// as the clip grows).
const SV_INTERVAL_SHORT_MS: u64 = 400;
const SV_INTERVAL_LONG_MS: u64 = 800;
const SV_LONG_AUDIO_SAMPLES: usize = 5 * 16_000;
/// Recording hard cap: the ASR session auto-finalizes at 30 s of audio,
/// taking the same path as a normal release.
const SV_MAX_SAMPLES: usize = 30 * 16_000;

/// Spawn the per-recording session thread for SenseVoice (offline, fast):
/// re-runs inference over the accumulated 16 kHz buffer every 0.4/0.8 s and
/// emits Partial updates; on release (or at the 30 s hard cap) it emits a
/// Final result (with the hallucination filter applied). Mirrors
/// `spawn_session`'s contract. The recognizer is loaded inside this thread.
pub fn spawn_sensevoice_session(
    rec: Arc<RecorderHandle>,
    sv_dir: String,
    lang_key: String,
    tx: glib::Sender<AsrUpdate>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("asr-sv".into())
        .spawn(move || {
            let model_path = format!("{}/model.int8.onnx", sv_dir.trim_end_matches('/'));
            let lang = sense_voice_language(&lang_key);

            // ---- partial loop: full re-run over accumulated audio ----
            while !rec.finish_flag() {
                crate::audio::sleep_ms(100);
                if rec.finish_flag() || rec.cancelled() {
                    break;
                }
                let snap = rec.snapshot_16k();
                if snap.len() >= SV_MAX_SAMPLES {
                    info!("sensevoice: 30 s hard cap reached; auto-finalizing");
                    break;
                }
                if snap.len() < MIN_SAMPLES {
                    continue;
                }
                let interval = if snap.len() < SV_LONG_AUDIO_SAMPLES {
                    SV_INTERVAL_SHORT_MS
                } else {
                    SV_INTERVAL_LONG_MS
                };
                // Coarse pacing: the 100 ms poll sleeps are summed by
                // tracking the last run against the interval budget.
                // (Sleep a bit extra so consecutive polls don't re-run.)
                crate::audio::sleep_ms(interval.saturating_sub(100));
                let snap = rec.snapshot_16k();
                if snap.len() < MIN_SAMPLES {
                    continue;
                }
                let text = with_sensevoice_recognizer(&model_path, lang, |r| {
                    transcribe_samples_sv(r, &snap)
                });
                match text {
                    Ok(t) if !t.is_empty() => {
                        let shown = convert_for(&lang_key, &t);
                        info!("sensevoice partial raw: {t:?} -> shown: {shown:?}");
                        if tx.send(AsrUpdate::Partial(shown)).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = tx.send(AsrUpdate::Failed(e));
                        return;
                    }
                }
            }

            // ---- final ----
            if rec.cancelled() {
                info!("sensevoice session cancelled");
                return;
            }
            let all = rec.take_all();
            let dur = all.len() as f32 / 16_000.0;
            if dur < 0.3 {
                let _ = tx.send(AsrUpdate::Final(String::new()));
                return;
            }
            info!("sensevoice final inference over {:.2}s of audio", dur);
            let result =
                with_sensevoice_recognizer(&model_path, lang, |r| {
                    transcribe_samples_sv(r, &all)
                });
            match result {
                Ok(raw) => {
                    let dur = all.len() as f32 / 16_000.0;
                    let filtered = filter_hallucination(&raw, dur);
                    let converted = convert_for(&lang_key, &filtered);
                    info!("sensevoice final raw: {raw:?}");
                    info!("sensevoice final converted: {converted:?}");
                    let _ = tx.send(AsrUpdate::Final(converted));
                }
                Err(e) => {
                    let _ = tx.send(AsrUpdate::Failed(e));
                }
            }
        })
        .expect("spawn asr-sv thread")
}

/// Standalone transcription of a WAV file through SenseVoice
/// (`voicetype --transcribe-sv <wav>`). Debug/smoke-test path. Applies the
/// hallucination filter like the live Final path. Uses the configured UI
/// language (not "auto") — SenseVoice's auto-detect is weak on noisy mic
/// recordings.
pub fn transcribe_file_sensevoice(
    wav_path: &str,
    sv_dir: &str,
    lang_key: &str,
) -> Result<String, String> {
    let samples = load_wav_16k_mono(wav_path)?;
    let model_path = format!("{}/model.int8.onnx", sv_dir.trim_end_matches('/'));
    let lang = sense_voice_language(lang_key);
    let text = with_sensevoice_recognizer(&model_path, lang, |r| {
        transcribe_samples_sv(r, &samples)
    })?;
    let dur = samples.len() as f32 / 16_000.0;
    Ok(filter_hallucination(&text, dur).trim().to_string())
}

/// Load a WAV file (16-bit PCM, mono or stereo, any sample rate), down-mix
/// to mono and resample to 16 kHz f32.
pub fn load_wav_16k_mono(wav_path: &str) -> Result<Vec<f32>, String> {
    let mut reader = hound::WavReader::open(wav_path)
        .map_err(|e| format!("failed to open WAV {wav_path}: {e}"))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(format!(
            "unsupported WAV format: expected 16-bit PCM, got {}-bit {:?}",
            spec.bits_per_sample, spec.sample_format
        ));
    }
    let channels = spec.channels as usize;
    let src_rate = spec.sample_rate as usize;
    if channels == 0 || channels > 8 {
        return Err(format!("unsupported channel count: {channels}"));
    }

    // Read interleaved i16 samples, average down-mix to mono.
    let mono: Vec<f32> = reader
        .samples::<i16>()
        .collect::<Result<Vec<i16>, _>>()
        .map_err(|e| format!("failed to read WAV samples: {e}"))?
        .chunks(channels)
        .map(|ch| ch.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32)
        .collect();

    let dst_rate = 16_000usize;
    let samples = if src_rate == dst_rate {
        mono
    } else {
        let out_len = (mono.len() as f64 * dst_rate as f64 / src_rate as f64) as usize;
        let mut out = Vec::with_capacity(out_len);
        let step = src_rate as f64 / dst_rate as f64;
        for i in 0..out_len {
            let pos = i as f64 * step;
            let i0 = pos as usize;
            let frac = (pos - i0 as f64) as f32;
            let s0 = mono[i0];
            let s1 = mono.get(i0 + 1).copied().unwrap_or(s0);
            out.push(s0 + (s1 - s0) * frac);
        }
        out
    };
    Ok(samples)
}

/// Standalone transcription of a WAV file via the legacy whisper chunked
/// pipeline (final-result quality). Debug path and ja/ko fallback.
pub fn transcribe_file(
    wav_path: &str,
    model_path: &str,
    lang_key: &str,
) -> Result<String, String> {
    let samples = load_wav_16k_mono(wav_path)?;
    let ctx = get_context(model_path)?;
    let lang = whisper_language(lang_key).to_string();
    let dur = samples.len() as f32 / 16_000.0;
    let text = transcribe_samples(&ctx, &samples, &lang, dur);
    Ok(convert_for(lang_key, &text))
}
