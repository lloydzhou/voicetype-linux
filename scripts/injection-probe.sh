#!/usr/bin/env bash
# Explicit local-to-the-X-server runner; never SSHs, deploys, or launches a shell target.
set -euo pipefail

if [[ ${VOICETYPE_INJECTION_PROBE:-} != 1 ]]; then
  printf '%s\n' 'Refusing live desktop input. Set VOICETYPE_INJECTION_PROBE=1 after reading scripts/INJECTION_PROBE.md.' >&2
  exit 2
fi
if [[ $(uname -s) != Linux ]]; then
  printf '%s\n' 'This probe requires Linux and an X11 server.' >&2
  exit 2
fi
: "${DISPLAY:?Set DISPLAY explicitly; prefer an isolated Xvfb or Xephyr server}"
for tool in cargo timeout xdotool xclip; do
  command -v "$tool" >/dev/null || { printf 'Missing prerequisite: %s\n' "$tool" >&2; exit 2; }
done
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
export GDK_BACKEND=x11 XDG_SESSION_TYPE=x11
unset WAYLAND_DISPLAY
# Cargo needs the project's ordinary native/audio/ASR build dependencies too.
# No blanket cargo timeout: dependency compilation may legitimately take minutes.
exec cargo test --manifest-path "$root/Cargo.toml" --test injection_probe \
  x11_entry_paste_readback -- --exact --ignored --nocapture --test-threads=1
