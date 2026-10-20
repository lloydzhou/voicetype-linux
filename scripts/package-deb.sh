#!/usr/bin/env bash
set -euo pipefail
# Build from the current Linux release artifact; no model or user config is bundled.
version=${1:?usage: package-deb.sh VERSION [OUTPUT_DIR]}
version=${version#v}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.+~-][A-Za-z0-9.+~-]+)?$ ]] || { echo 'Invalid Debian version' >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
out=${2:-"$root/dist"}
arch=$(dpkg --print-architecture)
[[ "$arch" == amd64 ]] || { echo 'Only amd64 is currently supported' >&2; exit 1; }
binary="$root/target/release/voicetype"
[[ -f "$binary" ]] || { echo 'Run cargo build --release --locked first' >&2; exit 1; }
stage=$(mktemp -d)
# Only remove the directory created by this invocation.
trap '[[ -n "$stage" && -d "$stage" && "$stage" == /tmp/tmp.* ]] && rm -rf -- "$stage"' EXIT
install -Dm755 "$binary" "$stage/usr/bin/voicetype"
mkdir -p "$stage/usr/lib/voicetype" "$stage/DEBIAN" "$out"
# ldd resolves the exact libraries selected by this build, including sherpa's cache.
python3 - "$binary" "$stage/usr/lib/voicetype" <<'PY'
import pathlib, shutil, subprocess, sys
binary, destination = sys.argv[1:]
result = subprocess.run(['ldd', binary], capture_output=True, text=True, check=True)
if 'not found' in result.stdout:
    raise SystemExit(result.stdout)
libs = {}
for line in result.stdout.splitlines():
    parts = line.split()
    if len(parts) >= 3 and parts[1] == '=>' and parts[2].startswith('/'):
        if parts[0].startswith(('libsherpa-', 'libonnxruntime')):
            libs[parts[0]] = parts[2]
if not any(n.startswith('libsherpa-onnx-c-api') for n in libs):
    raise SystemExit('Expected shared sherpa-onnx library missing from ldd output')
for name, source in libs.items():
    shutil.copy2(pathlib.Path(source).resolve(), pathlib.Path(destination)/name)
PY
patchelf --set-rpath '$ORIGIN/../lib/voicetype' "$stage/usr/bin/voicetype"
for lib in "$stage"/usr/lib/voicetype/*.so*; do
    patchelf --set-rpath '$ORIGIN' "$lib"
done
install -Dm644 "$root/packaging/voicetype.desktop" "$stage/usr/share/applications/voicetype.desktop"
install -Dm644 "$root/config.example.toml" "$stage/usr/share/doc/voicetype/config.example.toml"
install -Dm644 "$root/LICENSE" "$stage/usr/share/doc/voicetype/copyright"
install -Dm644 "$root/THIRD_PARTY_NOTICES.md" "$stage/usr/share/doc/voicetype/THIRD_PARTY_NOTICES.md"
# Resolve distro-specific GTK/ALSA dependency names instead of hardcoding t64 names.
mkdir -p "$stage/debian"
printf 'Source: voicetype\nSection: utils\nPriority: optional\nMaintainer: lloydzhou <lloydzhou@users.noreply.github.com>\n\nPackage: voicetype\nArchitecture: amd64\nDescription: Local hold-to-talk dictation\n' > "$stage/debian/control"
deps=$(cd "$stage" && dpkg-shlibdeps --ignore-missing-info -l"$stage/usr/lib/voicetype" -O -eusr/bin/voicetype | sed -n 's/^shlibs:Depends=//p')
[[ -n "$deps" ]] || { echo 'Could not determine system library dependencies' >&2; exit 1; }
cat > "$stage/DEBIAN/control" <<EOF
Package: voicetype
Version: $version
Section: sound
Priority: optional
Architecture: $arch
Maintainer: lloydzhou <lloydzhou@users.noreply.github.com>
Homepage: https://github.com/lloydzhou/voicetype-linux
Depends: $deps, xdotool, xclip, x11-utils
Recommends: libayatana-appindicator3-1
Suggests: wl-clipboard, wtype
Description: Local hold-to-talk voice input for Linux
 Hold right Ctrl and right Shift to dictate using local SenseVoice inference.
 Release either key to paste into the focused input. Models are downloaded
 separately. Reading global hotkeys requires user-configured input permissions.
EOF
# Exclude build-only metadata without deleting other paths.
python3 - "$stage/debian/control" <<'PY'
import pathlib,sys
p=pathlib.Path(sys.argv[1]); p.unlink(); p.parent.rmdir()
PY
dpkg-deb --root-owner-group -Zxz --build "$stage" "$out/voicetype_${version}_${arch}.deb"
dpkg-deb --info "$out/voicetype_${version}_${arch}.deb"
