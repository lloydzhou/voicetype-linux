//! Paste into the current input focus without activating another window.
use log::{info, warn};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

const CLIPBOARD_TIMEOUT_MS: u64 = 800;
const PASTE_TIMEOUT_MS: u64 = 500;
static INJECTION: Mutex<()> = Mutex::new(());

fn is_wayland() -> bool {
    !std::env::var("WAYLAND_DISPLAY").unwrap_or_default().is_empty()
        && std::env::var("XDG_SESSION_TYPE").unwrap_or_default() != "x11"
}

/// Execute a bounded command. Clipboard owners inherit no output pipe: their
/// daemon descendants must not delay completion of a clipboard write.
fn run(program: &str, args: &[&str], input: Option<String>, timeout_ms: u64) -> Result<Option<String>, String> {
    let capture = input.is_none();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut child = Command::new(program).args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(if capture { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::null()).spawn().map_err(|e| format!("{program}: {e}"))?;
    let writer = input.map(|text| {
        let mut stdin = child.stdin.take().expect("piped stdin");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || { let _ = tx.send(stdin.write_all(text.as_bytes()).map_err(|e| e.to_string())); });
        rx
    });
    let reader = child.stdout.take().map(|mut out| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            let result = out.read_to_string(&mut text).map(|_| text).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        rx
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Err(e) => return Err(format!("{program}: {e}")),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill(); let _ = child.wait();
                return Err(format!("{program}: timeout ({timeout_ms}ms)"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    if !status.success() { return Err(format!("{program}: {status}")); }
    if let Some(rx) = writer {
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| format!("{program} stdin: {e}"))??;
    }
    reader.map(|rx| rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|e| format!("{program} stdout: {e}"))
        .and_then(|r| r)).transpose()
}

#[derive(Debug, PartialEq)]
struct Focus { leaf: String, client: String, class: String }

fn parent_from_tree(tree: &str) -> Option<String> {
    tree.lines().find_map(|line| line.trim().strip_prefix("Parent window id:")
        .and_then(|rest| rest.split_whitespace().next()).map(str::to_owned))
}

/// Class from the last quoted WM_CLASS field, absent when the property is
/// missing. xprop instead of `xdotool getwindowclassname`, which older
/// releases (Ubuntu 22.04) do not provide.
fn window_class(window: &str) -> Option<String> {
    let out = run("xprop", &["-id", window, "WM_CLASS"], None, 300).ok()??;
    let line = out.lines().find(|l| l.trim_start().starts_with("WM_CLASS"))?;
    line.split('"').nth_back(1).filter(|c| !c.is_empty()).map(str::to_owned)
}

/// WM_CLASS can belong to an ancestor of the focused input child. Resolve
/// that ancestor for shortcut selection, but preserve the actual input focus.
fn current_focus() -> Result<Focus, String> {
    let leaf = run("xdotool", &["getwindowfocus", "-f"], None, 300)?
        .ok_or("missing focus")?.trim().to_string();
    let mut client = leaf.clone();
    for _ in 0..32 {
        if let Some(class) = window_class(&client) {
            if class != "(null)" {
                return Ok(Focus { leaf, client, class: class.to_lowercase() });
            }
        }
        let tree = run("xwininfo", &["-id", &client, "-tree"], None, 300)?
            .ok_or("missing window tree")?;
        let parent = parent_from_tree(&tree).ok_or("focused window has no class-bearing ancestor")?;
        if parent == "0x0" || parent == client { break; }
        client = parent;
    }
    Err("cannot resolve current input focus; refusing paste".into())
}

fn paste_key(class: &str) -> &'static str {
    let terminal = class == "st" || ["terminal", "konsole", "alacritty", "kitty", "xterm",
        "urxvt", "st-256color", "foot", "wezterm", "tilix", "termite", "rxvt", "sakura", "vte"]
        .iter().any(|name| class.contains(name));
    if terminal { "ctrl+shift+v" } else { "ctrl+v" }
}

fn read_clipboard(wayland: bool) -> Option<String> {
    if wayland { run("wl-paste", &["--no-newline"], None, CLIPBOARD_TIMEOUT_MS) }
    else { run("xclip", &["-selection", "clipboard", "-o"], None, CLIPBOARD_TIMEOUT_MS) }.ok().flatten()
}

fn set_clipboard(wayland: bool, text: &str) -> Result<(), String> {
    if wayland { run("wl-copy", &[], Some(text.into()), CLIPBOARD_TIMEOUT_MS) }
    else { run("xclip", &["-selection", "clipboard", "-i"], Some(text.into()), CLIPBOARD_TIMEOUT_MS) }.map(|_| ())
}

fn inject_inner(text: &str, ime_switch: bool) -> Result<(), String> {
    let _guard = INJECTION.lock().map_err(|_| "injection lock poisoned")?;
    let started = Instant::now();
    let wayland = is_wayland();
    let focus = if wayland { None } else { Some(current_focus()?) };
    let saved = read_clipboard(wayland);
    set_clipboard(wayland, text)?;
    if read_clipboard(wayland).as_deref() != Some(text) {
        return Err("clipboard readback differs; refusing paste".into());
    }
    // Clipboard delivery needs no IME switch. Keep the caller's preference
    // accepted without changing a global input method during paste.
    if ime_switch { info!("clipboard paste preserves the active input method"); }
    let result = if let Some(target) = focus {
        let now = current_focus()?;
        if now != target { return Err("input focus changed during clipboard preparation; refusing paste".into()); }
        let key = paste_key(&target.class);
        info!("paste target: leaf={} client={} class={:?} key={key}", target.leaf, target.client, target.class);
        run("xdotool", &["key", "--clearmodifiers", key], None, PASTE_TIMEOUT_MS)
    } else {
        run("wtype", &["-M", "ctrl", "-k", "v", "-m", "ctrl"], None, PASTE_TIMEOUT_MS)
    };
    // Leave enough time for the receiver's asynchronous selection request.
    // Restore only if no other application replaced our clipboard value.
    std::thread::sleep(Duration::from_millis(500));
    if read_clipboard(wayland).as_deref() == Some(text) {
        if let Some(saved) = saved { set_clipboard(wayland, &saved)?; }
    }
    result?;
    info!("paste command completed in {}ms (receiver readback required for proof)", started.elapsed().as_millis());
    Ok(())
}

/// Serialize clipboard transactions and paste into the current focused input.
pub fn inject(text: &str, ime_switch: bool) {
    if text.is_empty() { return; }
    if let Err(error) = inject_inner(text, ime_switch) { warn!("injection failed: {error}"); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_parent_line() {
        assert_eq!(parent_from_tree("  Parent window id: 0x400003 \"terminal\"\n"), Some("0x400003".into()));
    }
    #[test]
    fn terminal_shortcuts_do_not_match_arbitrary_st_substrings() {
        assert_eq!(paste_key("sakura"), "ctrl+shift+v");
        assert_eq!(paste_key("st"), "ctrl+shift+v");
        assert_eq!(paste_key("test-browser"), "ctrl+v");
    }
    #[test]
    fn daemon_output_does_not_block_input_command() {
        let start = Instant::now();
        run("sh", &["-c", "cat >/dev/null; sleep 2 &"], Some("text".into()), 400).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn inherited_output_pipe_is_bounded() {
        let start = Instant::now();
        assert!(run("sh", &["-c", "sleep 2 &"], None, 100).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
