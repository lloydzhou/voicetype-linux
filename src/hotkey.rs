//! Hold-to-talk chords from config.toml: start when a configured chord is
//! fully held, submit when any of its keys is released.

use log::{error, info, warn};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Key codes from linux/input-event-codes.h
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_RIGHTSHIFT: u16 = 54;

/// Key names accepted in `hotkeys` chords (case-insensitive, joined with `+`).
fn key_code(name: &str) -> Option<u16> {
    Some(match name.to_ascii_lowercase().as_str() {
        "leftctrl" => 29,
        "rightctrl" => KEY_RIGHTCTRL,
        "leftshift" => 42,
        "rightshift" => KEY_RIGHTSHIFT,
        "leftalt" => 56,
        "rightalt" => 100,
        "leftsuper" | "leftwin" => 125,
        "rightsuper" | "rightwin" => 126,
        "space" => 57,
        "capslock" => 58,
        code => return code.parse::<u16>().ok(),
    })
}

/// Parse configured chord strings into evdev key codes. Unknown names,
/// empty chords, or duplicate keys inside one chord fail loudly: hotkeys
/// are load-bearing, so a typo must stop startup instead of disabling it.
pub fn parse_combos(configured: &[String]) -> Result<Vec<Vec<u16>>, String> {
    let names: Vec<String> = if configured.is_empty() {
        vec!["RightCtrl+RightShift".to_string()]
    } else {
        configured.to_vec()
    };
    let mut combos = Vec::new();
    for name in &names {
        let mut combo = Vec::new();
        for part in name.split('+') {
            let part = part.trim();
            let Some(code) = key_code(part) else {
                return Err(format!("hotkeys: unknown key name {part:?} in {name:?}"));
            };
            if combo.contains(&code) {
                return Err(format!("hotkeys: duplicate key {part:?} in {name:?}"));
            }
            combo.push(code);
        }
        if combo.is_empty() {
            return Err(format!("hotkeys: empty chord {name:?}"));
        }
        combos.push(combo);
    }
    Ok(combos)
}

/// Human-readable chord names for logs and the tray tooltip.
pub fn display_combos(configured: &[String]) -> String {
    if configured.is_empty() {
        "RightCtrl+RightShift".to_string()
    } else {
        configured.join(" / ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Start,
    Stop,
}

#[derive(Default)]
struct GlobalState {
    /// per-device held key codes, keyed by device path
    devices: HashMap<String, HashSet<u16>>,
    /// whether a recording session is active
    active: bool,
}

impl GlobalState {
    fn update(
        &mut self,
        path: &str,
        code: u16,
        pressed: bool,
        combos: &[Vec<u16>],
    ) -> Option<HotkeyEvent> {
        let held = self.devices.entry(path.to_string()).or_default();
        if pressed { held.insert(code); } else { held.remove(&code); }
        let all: HashSet<u16> = self.devices.values().flatten().copied().collect();
        let chord_held = combos.iter().any(|c| c.iter().all(|k| all.contains(k)));
        match (self.active, chord_held) {
            (false, true) => { self.active = true; Some(HotkeyEvent::Start) }
            (true, false) => { self.active = false; Some(HotkeyEvent::Stop) }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_combos() -> Vec<Vec<u16>> {
        parse_combos(&[]).unwrap()
    }

    #[test]
    fn either_press_and_release_order_submits_exactly_once() {
        let combos = default_combos();
        for keys in [[KEY_RIGHTCTRL, KEY_RIGHTSHIFT], [KEY_RIGHTSHIFT, KEY_RIGHTCTRL]] {
            for release in [keys, [keys[1], keys[0]]] {
                let mut state = GlobalState::default();
                assert_eq!(state.update("keyboard", keys[0], true, &combos), None);
                assert_eq!(state.update("keyboard", keys[1], true, &combos), Some(HotkeyEvent::Start));
                assert_eq!(state.update("keyboard", keys[1], true, &combos), None);
                assert_eq!(state.update("keyboard", release[0], false, &combos), Some(HotkeyEvent::Stop));
                assert_eq!(state.update("keyboard", release[1], false, &combos), None);
            }
        }
    }

    #[test]
    fn unrelated_keys_do_not_start_recording() {
        let combos = default_combos();
        let mut state = GlobalState::default();
        for key in [29, 42, 56, 57, 125, 126, 100] {
            assert_eq!(state.update("keyboard", key, true, &combos), None);
        }
    }

    #[test]
    fn left_modifiers_never_substitute_for_right_modifiers() {
        let combos = default_combos();
        for keys in [[29, KEY_RIGHTSHIFT], [KEY_RIGHTCTRL, 42], [29, 42]] {
            let mut state = GlobalState::default();
            for key in keys {
                assert_eq!(state.update("keyboard", key, true, &combos), None);
            }
        }
    }

    #[test]
    fn parse_accepts_names_numeric_codes_and_defaults() {
        assert_eq!(
            parse_combos(&["RightCtrl+RightShift".into()]).unwrap(),
            vec![vec![KEY_RIGHTCTRL, KEY_RIGHTSHIFT]]
        );
        // The documented default (no hotkeys configured) is exactly this chord.
        assert_eq!(parse_combos(&[]).unwrap(), vec![vec![97, 54]]);
        assert_eq!(parse_combos(&["rightalt".into()]).unwrap(), vec![vec![100]]);
        assert_eq!(parse_combos(&[" 97 + 54 ".into()]).unwrap(), vec![vec![97, 54]]);
        assert_eq!(display_combos(&["A+B".into(), "C".into()]), "A+B / C");
    }

    #[test]
    fn parse_rejects_unknown_empty_and_duplicate_keys() {
        assert!(parse_combos(&["Fn+Space".into()]).is_err());
        assert!(parse_combos(&["".into()]).is_err());
        assert!(parse_combos(&["RightCtrl+RightCtrl".into()]).is_err());
    }

    #[test]
    fn single_key_chord_starts_and_stops() {
        let combos = parse_combos(&["RightAlt".into()]).unwrap();
        let mut state = GlobalState::default();
        assert_eq!(state.update("keyboard", 100, true, &combos), Some(HotkeyEvent::Start));
        assert_eq!(state.update("keyboard", 100, false, &combos), Some(HotkeyEvent::Stop));
    }

    #[test]
    fn any_configured_chord_holds_recording_open() {
        let combos = parse_combos(&["RightCtrl+RightShift".into(), "RightCtrl+Space".into()]).unwrap();
        let mut state = GlobalState::default();
        state.update("keyboard", KEY_RIGHTCTRL, true, &combos);
        assert_eq!(state.update("keyboard", KEY_RIGHTSHIFT, true, &combos), Some(HotkeyEvent::Start));
        // Shift released, but the other chord (RightCtrl+Space) is still held.
        assert_eq!(state.update("keyboard", 57, true, &combos), None);
        assert_eq!(state.update("keyboard", KEY_RIGHTSHIFT, false, &combos), None);
        assert_eq!(state.update("keyboard", 57, false, &combos), Some(HotkeyEvent::Stop));
    }

    #[test]
    fn same_key_on_two_keyboards_stays_held_until_both_released() {
        let combos = parse_combos(&["RightAlt".into()]).unwrap();
        let mut state = GlobalState::default();
        state.update("builtin", 100, true, &combos);
        state.update("external", 100, true, &combos);
        assert_eq!(state.update("builtin", 100, false, &combos), None);
        assert_eq!(state.update("external", 100, false, &combos), Some(HotkeyEvent::Stop));
    }
}

fn open_device(path: &str) -> Option<evdev::Device> {
    // evdev 0.12: open by path. fetch_events() is non-blocking; the device
    // loop sleeps briefly when no events are pending (WouldBlock).
    evdev::Device::open(path).ok()
}

/// Spawn the hotkey listener thread. Returns a handle (the thread runs forever).
pub fn spawn(tx: glib::Sender<HotkeyEvent>, combos: Vec<Vec<u16>>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("hotkey".into())
        .spawn(move || run(tx, Arc::new(combos)))
        .expect("spawn hotkey thread")
}

fn run(tx: glib::Sender<HotkeyEvent>, combos: Arc<Vec<Vec<u16>>>) {
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
            let combos = Arc::clone(&combos);
            let h = std::thread::Builder::new()
                .name(format!("hotkey-{p}"))
                .spawn(move || device_loop(&p, dev, &state, &tx, &combos))
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
    combos: &Arc<Vec<Vec<u16>>>,
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
                    let event = g.update(path, ev.code(), pressed, combos);
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
