//! Hold right Ctrl+right Shift to record; releasing either modifier submits the recording.

use log::{error, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Key codes from linux/input-event-codes.h
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTSHIFT: u16 = 54;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Start,
    Stop,
}

#[derive(Default, Clone, Copy)]
struct DevKeys {
    right_ctrl: bool,
    right_shift: bool,
}

#[derive(Default)]
struct GlobalState {
    /// per-device key state, keyed by device path
    devices: HashMap<String, DevKeys>,
    /// whether a recording session is active
    active: bool,
}

impl GlobalState {
    fn combined(&self) -> DevKeys {
        let mut c = DevKeys::default();
        for d in self.devices.values() {
            c.right_ctrl |= d.right_ctrl;
            c.right_shift |= d.right_shift;
        }
        c
    }

    fn update(&mut self, path: &str, code: u16, pressed: bool) -> Option<HotkeyEvent> {
        let d = self.devices.entry(path.to_string()).or_default();
        match code {
            KEY_RIGHTCTRL => d.right_ctrl = pressed,
            KEY_RIGHTSHIFT => d.right_shift = pressed,
            _ => return None,
        }
        let c = self.combined();
        let held = c.right_ctrl && c.right_shift;
        match (self.active, held) {
            (false, true) => { self.active = true; Some(HotkeyEvent::Start) }
            (true, false) => { self.active = false; Some(HotkeyEvent::Stop) }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_press_and_release_order_submits_exactly_once() {
        for keys in [[KEY_RIGHTCTRL, KEY_RIGHTSHIFT], [KEY_RIGHTSHIFT, KEY_RIGHTCTRL]] {
            for release in [keys, [keys[1], keys[0]]] {
                let mut state = GlobalState::default();
                assert_eq!(state.update("keyboard", keys[0], true), None);
                assert_eq!(state.update("keyboard", keys[1], true), Some(HotkeyEvent::Start));
                assert_eq!(state.update("keyboard", keys[1], true), None);
                assert_eq!(state.update("keyboard", release[0], false), Some(HotkeyEvent::Stop));
                assert_eq!(state.update("keyboard", release[1], false), None);
            }
        }
    }

    #[test]
    fn old_chord_does_not_start_recording() {
        let mut state = GlobalState::default();
        for key in [29, 42, 56, 57, 125, 126, 100] {
            assert_eq!(state.update("keyboard", key, true), None);
        }
    }

    #[test]
    fn left_modifiers_never_substitute_for_right_modifiers() {
        for keys in [[29, KEY_RIGHTSHIFT], [KEY_RIGHTCTRL, 42], [29, 42]] {
            let mut state = GlobalState::default();
            for key in keys { assert_eq!(state.update("keyboard", key, true), None); }
        }
    }
}

fn open_device(path: &str) -> Option<evdev::Device> {
    // evdev 0.12: open by path. fetch_events() is non-blocking; the device
    // loop sleeps briefly when no events are pending (WouldBlock).
    evdev::Device::open(path).ok()
}

/// Spawn the hotkey listener thread. Returns a handle (the thread runs forever).
pub fn spawn(tx: glib::Sender<HotkeyEvent>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("hotkey".into())
        .spawn(move || run(tx))
        .expect("spawn hotkey thread")
}

fn run(tx: glib::Sender<HotkeyEvent>) {
    loop {
        let paths = list_event_devices();
        if paths.is_empty() {
            error!(
                "无法读取任何 /dev/input/event* 设备。请执行: \
                 sudo usermod -aG input $USER 然后重新登录。5 秒后重试。"
            );
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }

        let state = Arc::new(Mutex::new(GlobalState::default()));
        let mut handles = Vec::new();

        for path in paths {
            let Some(dev) = open_device(&path) else {
                warn!("cannot open {path} (permission denied?)");
                continue;
            };
            let name = dev.name().unwrap_or("?").to_string();
            info!("listening on {path} ({name})");
            let state = Arc::clone(&state);
            let tx = tx.clone();
            let p = path.clone();
            let h = std::thread::Builder::new()
                .name(format!("hotkey-{p}"))
                .spawn(move || device_loop(&p, dev, &state, &tx))
                .expect("spawn device thread");
            handles.push(h);
        }

        if handles.is_empty() {
            error!(
                "没有任何键盘设备可监听 (/dev/input 不可读?)。请执行: \
                 sudo usermod -aG input $USER 然后重新登录。5 秒后重试。"
            );
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }

        // If all device threads died (unplugged / permissions changed), rescan.
        for h in handles {
            let _ = h.join();
        }
        warn!("all input device threads exited; rescanning in 2s");
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

fn list_event_devices() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/dev/input") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("event") {
                out.push(e.path().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

fn device_loop(
    path: &str,
    mut dev: evdev::Device,
    state: &Arc<Mutex<GlobalState>>,
    tx: &glib::Sender<HotkeyEvent>,
) {
    use evdev::InputEventKind;
    loop {
        match dev.fetch_events() {
            Ok(events) => {
                for ev in events {
                    if !matches!(ev.kind(), InputEventKind::Key(_)) {
                        continue; // ignore SYN / other
                    }
                    if ev.value() == 2 {
                        continue; // auto-repeat
                    }
                    let pressed = ev.value() == 1;
                    let mut g = match state.lock() {
                        Ok(g) => g,
                        Err(_) => continue,
                    };
                    let event = g.update(path, ev.code(), pressed);
                    drop(g);
                    if let Some(e) = event {
                        if tx.send(e).is_err() {
                            return; // UI gone
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Non-blocking fd: no events pending; brief sleep instead of spinning.
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                // Read failure: device gone.
                warn!("{}: {e}; exiting device thread", path);
                return;
            }
        }
    }
}
