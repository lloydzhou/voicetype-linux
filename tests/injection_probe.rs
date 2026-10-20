//! Opt-in, real-X11 clipboard integration probe. See scripts/INJECTION_PROBE.md.
//! The Sakura case is a GTK Entry with a terminal paste binding, not a VTE test.

#[path = "../src/inject.rs"]
mod inject;

use gtk::gdk;
use gtk::prelude::*;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn xdotool(args: &[&str]) -> String {
    let output = Command::new("timeout")
        .args(["--kill-after=1s", "3s", "xdotool"])
        .args(args)
        .output()
        .expect("spawn timeout/xdotool (GNU coreutils and xdotool required)");
    assert!(
        output.status.success(),
        "xdotool {args:?}: {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("xdotool UTF-8 output")
        .trim()
        .to_owned()
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        // Bound each drain too: an always-ready source must not hide the deadline.
        for _ in 0..64 {
            if !gtk::events_pending() {
                break;
            }
            gtk::main_iteration_do(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn run_case(class: &str, shift: bool, _tracked: bool) {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let title = format!("voicetype-injection-probe-{}-{nonce}", std::process::id());
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(&title);
    window.set_default_size(640, 90);
    let entry = gtk::Entry::new();
    window.add(&entry);

    let chords = Rc::new(RefCell::new(Vec::new()));
    let observed = chords.clone();
    entry.connect_key_press_event(move |entry, event| {
        let state = event.state();
        if (event.keyval() == gdk::keys::constants::v
            || event.keyval() == gdk::keys::constants::V)
            && state.contains(gdk::ModifierType::CONTROL_MASK)
        {
            let shifted = state.contains(gdk::ModifierType::SHIFT_MASK);
            observed.borrow_mut().push(shifted);
            if shift {
                // Only the terminal chord pastes in this controlled target. A wrong
                // Ctrl+V cannot pass merely because GtkEntry normally accepts it.
                if shifted {
                    entry.paste_clipboard();
                }
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });

    window.show_all();
    entry.grab_focus();
    pump_for(Duration::from_millis(250));
    let pid = std::process::id().to_string();
    let pattern = format!("^{title}$");
    let ids = xdotool(&["search", "--all", "--pid", &pid, "--name", &pattern]);
    let found: Vec<_> = ids.lines().collect();
    assert_eq!(found.len(), 1, "must discover exactly one owned window: {ids:?}");
    let xid = found[0].to_owned();
    assert_eq!(xdotool(&["getwindowpid", &xid]), pid);
    assert_eq!(xdotool(&["getwindowname", &xid]), title);
    xdotool(&["set_window", "--classname", class, "--class", class, &xid]);
    // Read back through xprop, matching the production class resolver that
    // avoids `xdotool getwindowclassname` (missing from older xdotool).
    let wm_class = Command::new("xprop")
        .args(["-id", &xid, "WM_CLASS"])
        .output()
        .expect("spawn xprop");
    assert!(wm_class.status.success());
    let wm_class = String::from_utf8(wm_class.stdout).expect("xprop UTF-8");
    assert_eq!(wm_class.split('"').nth_back(1), Some(class), "WM_CLASS: {wm_class}");
    xdotool(&["windowfocus", "--sync", &xid]);
    pump_for(Duration::from_millis(100));
    assert_eq!(xdotool(&["getwindowfocus"]), xid, "focus moved before injection");

    // Non-shell payload, no newline/Enter, with Unicode and punctuation.
    let expected = format!("VoiceType {class} {nonce} — 中文 café 🦀 [ok]");
    let payload = expected.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        assert_eq!(xdotool(&["getwindowfocus"]), xid, "focus moved before injection");
        inject::inject(&payload, false);
        tx.send(()).expect("probe receiver remains alive");
    });

    // inject() blocks on subprocesses and clipboard timing. Keep GTK dispatching
    // on this thread so the asynchronous selection request can actually finish.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        pump_for(Duration::from_millis(10));
        match rx.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if Instant::now() >= deadline {
            // Never destroy the target while a live injector may still send keys.
            eprintln!("injection worker timed out; aborting probe process");
            std::process::abort();
        }
    }
    worker.join().expect("injection worker panicked");
    let readback_deadline = Instant::now() + Duration::from_secs(2);
    while entry.text().as_str() != expected && Instant::now() < readback_deadline {
        pump_for(Duration::from_millis(10));
    }
    // Continue dispatching to catch duplicate paste or late changes.
    pump_for(Duration::from_millis(200));
    let actual = entry.text().to_string();
    let actual_chords = chords.borrow().clone();
    window.close();
    pump_for(Duration::from_millis(50));
    assert_eq!(actual_chords, vec![shift], "{class}: expected exactly one paste chord");
    assert_eq!(actual, expected, "{class}: actual GTK Entry readback");
    eprintln!("PASS class={class} chord={} readback={actual:?}",
        if shift { "Ctrl+Shift+V" } else { "Ctrl+V" });
}

#[test]
#[ignore = "real X11 focus/clipboard side effects; requires VOICETYPE_INJECTION_PROBE=1"]
fn x11_entry_paste_readback() {
    assert!(cfg!(target_os = "linux"), "probe requires Linux/X11");
    assert_eq!(std::env::var("VOICETYPE_INJECTION_PROBE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("GDK_BACKEND").as_deref(), Ok("x11"));
    assert_eq!(std::env::var("XDG_SESSION_TYPE").as_deref(), Ok("x11"));
    assert!(std::env::var("DISPLAY").is_ok(), "DISPLAY must be explicit");
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .is_test(true)
        .try_init();
    gtk::init().expect("connect GTK to the explicitly selected X11 display");
    // One thread owns GTK; switch target classes to detect stale decisions.
    run_case("VoiceTypeProbe", false, false);
    run_case("Sakura", true, true);
    run_case("VoiceTypeProbe", false, true);
}
