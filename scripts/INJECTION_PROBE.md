# X11 injection tests

Both integration tests import `src/inject.rs` and call the production `inject::inject()` function. Tests require explicit opt-in and an isolated X server; they do not exercise microphone capture or the evdev hotkey.

`tests/injection_probe.rs` checks exact Unicode GTK Entry readback and exactly one paste chord for normal, Sakura-class, and normal windows in sequence. The Sakura-class Entry has a Ctrl+Shift+V handler and is not a terminal emulator.

`tests/sakura_probe.rs` launches real Sakura with a raw Python byte receiver instead of a shell, focuses its unnamed input child, and compares received PTY bytes to the expected Unicode text. No Enter key is sent. It requires Sakura and Python 3, leaves its small diagnostic directory under `/tmp/voicetype-sakura-probe-*`, and refuses DISPLAY=:0.

Dependencies: Linux GTK3, Xvfb, xauth, xdotool, xclip, xwininfo, timeout, and the project's build dependencies. Supply LD_LIBRARY_PATH if the local sherpa shared libraries require it.

```sh
export GDK_BACKEND=x11 XDG_SESSION_TYPE=x11 VOICETYPE_INJECTION_PROBE=1
unset WAYLAND_DISPLAY
xvfb-run -a cargo test --test injection_probe x11_entry_paste_readback -- --ignored --nocapture --test-threads=1
xvfb-run -a cargo test --test sakura_probe real_sakura_readback -- --ignored --nocapture --test-threads=1
cargo test --bin voicetype inject::tests -- --test-threads=1
```

Command completion is not proof of text delivery. These tests pass only after receiver readback. They do not validate microphone-to-text accuracy, the user's window manager, Wayland, or preservation of non-text clipboard formats.
