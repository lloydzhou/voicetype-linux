//! Microphone capture via cpal: 16 kHz mono f32 into a shared buffer,
//! RMS level pushed to the UI through a glib channel.

use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TARGET_RATE: usize = 16_000;

/// Send+Sync handle shared with the ASR thread (cpal::Stream is !Sync, so
/// the recorder itself must stay on the UI thread; only the handle crosses).
#[derive(Clone)]
pub struct RecorderHandle {
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) buffer: Arc<Mutex<Vec<f32>>>,
    /// Native capture rate (e.g. 44100); the buffer holds mono samples at
    /// this rate and everything leaving the handle MUST be resampled.
    pub(crate) in_rate: usize,
}

pub struct AudioRecorder {
    handle: RecorderHandle,
    /// Keep the stream alive; dropping stops capture.
    _stream: cpal::Stream,
}

impl AudioRecorder {
    /// Shared handle for the ASR session thread.
    pub fn handle(&self) -> RecorderHandle {
        self.handle.clone()
    }

    /// Open the default input device and start streaming.
    pub fn start(level_tx: glib::Sender<f32>) -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "没有可用的录音设备 (default input device)".to_string())?;
        let cfg = device
            .default_input_config()
            .map_err(|e| format!("查询输入配置失败: {e}"))?;
        let sample_rate = cfg.sample_rate().0 as usize;
        let channels = cfg.channels() as usize;
        info!(
            "input: {} Hz, {} ch, {:?}",
            sample_rate,
            channels,
            cfg.sample_format()
        );

        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let buffer = Arc::new(Mutex::new(Vec::<f32>::with_capacity(TARGET_RATE * 60)));

        let stop_cb = Arc::clone(&stop);
        let buf_cb = Arc::clone(&buffer);

        let err_fn = |e| warn!("audio stream error: {e}");
        // cpal 0.15: callback is FnMut(&[f32], &InputCallbackInfo) -> (),
        // and must not use `?`.
        let cb = move |data: &[f32], _info: &cpal::InputCallbackInfo| {
            if stop_cb.load(Ordering::Relaxed) {
                return;
            }
            let mono = mixdown(data, channels);
            let mut rms_acc = 0.0f32;
            let mut pushed = 0usize;
            if let Ok(mut b) = buf_cb.lock() {
                for s in mono {
                    rms_acc += s * s;
                    b.push(s);
                    pushed += 1;
                }
            }
            let rms = (rms_acc / pushed.max(1) as f32).sqrt();
            // Non-blocking: glib channel is unbounded.
            let _ = level_tx.send(rms);
        };

        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(&cfg.into(), cb, err_fn, None),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &cfg.into(),
                move |d: &[i16], info: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = d.iter().map(|&s| s as f32 / 32768.0).collect();
                    cb(&f, info);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &cfg.into(),
                move |d: &[u16], info: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = d
                        .iter()
                        .map(|&s| (s as f32 - 32768.0) / 32768.0)
                        .collect();
                    cb(&f, info);
                },
                err_fn,
                None,
            ),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| format!("打开录音流失败: {e}"))?;

        stream.play().map_err(|e| format!("启动录音失败: {e}"))?;
        Ok(Self {
            handle: RecorderHandle {
                stop,
                cancel,
                buffer,
                in_rate: sample_rate,
            },
            _stream: stream,
        })
    }

    pub fn snapshot(&self) -> Vec<f32> {
        self.handle.buffer.lock().map(|b| b.clone()).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.handle.buffer.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// Signal end of recording; capture halts, buffer stays readable.
    pub fn finish(&self) {
        self.handle.stop.store(true, Ordering::Relaxed);
    }

    pub fn cancel(&self) {
        self.handle.stop.store(true, Ordering::Relaxed);
        self.handle.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.handle.cancel.load(Ordering::Relaxed)
    }
}

impl RecorderHandle {
    /// Whether the recording session has been ended.
    pub fn finish_flag(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Snapshot of the accumulated samples, resampled to 16 kHz.
    pub fn snapshot_16k(&self) -> Vec<f32> {
        let raw = self.buffer.lock().map(|b| b.clone()).unwrap_or_default();
        resample_to_16k(&raw, self.in_rate)
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Drain the accumulated samples (final inference), resampled to 16 kHz.
    pub fn take_all(&self) -> Vec<f32> {
        let raw = self
            .buffer
            .lock()
            .map(|mut b| std::mem::take(&mut *b))
            .unwrap_or_default();
        resample_to_16k(&raw, self.in_rate)
    }
}

/// Mix N channels down to mono.
fn mixdown(data: &[f32], channels: usize) -> impl Iterator<Item = f32> + '_ {
    if channels <= 1 {
        data.iter().copied().collect::<Vec<f32>>().into_iter()
    } else {
        data.chunks(channels)
            .map(|ch| ch.iter().sum::<f32>() / channels as f32)
            .collect::<Vec<f32>>()
            .into_iter()
    }
}

/// Naive linear-interpolation resample to 16 kHz mono. Good enough for ASR.
pub fn resample_to_16k(input: &[f32], in_rate: usize) -> Vec<f32> {
    if in_rate == TARGET_RATE || input.is_empty() {
        return input.to_vec();
    }
    let ratio = in_rate as f64 / TARGET_RATE as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let i0 = pos as usize;
        let i1 = (i0 + 1).min(input.len() - 1);
        let frac = (pos - i0 as f64) as f32;
        out.push(input[i0] * (1.0 - frac) + input[i1] * frac);
    }
    out
}

/// Tiny helper used by the ASR session loop.
pub fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

// Silence unused warning when error path compiled out.
#[allow(dead_code)]
fn _log_guard() {
    error!("unused");
}
