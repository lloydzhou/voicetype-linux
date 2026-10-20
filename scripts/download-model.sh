#!/usr/bin/env bash
# Install as voicetype-download-model; downloads are data, never shell input.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: voicetype-download-model [--hf-base URL] [--proxy URL] [--output-dir DIR]

Download SenseVoice model.int8.onnx and tokens.txt from:
  csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17

Options:
  --hf-base URL    Hugging Face base URL (HF_ENDPOINT or https://huggingface.co)
  --proxy URL      Explicit curl proxy; otherwise standard proxy environment applies
  --output-dir DIR
                  Destination (XDG_DATA_HOME/voicetype/models/sensevoice or
                  HOME/.local/share/voicetype/models/sensevoice)
  --help          Show this help

Both downloads are staged and validated before replacing either existing file.
Replacement uses two renames, not an atomic pair: interruption or a filesystem
error during installation may leave mixed versions. Other files are untouched.
Each request has a 15-second connection timeout and 600-second transfer timeout,
with at most two retries and a 1200-second retry window.
EOF
}

fail() {
    printf 'voicetype-download-model: %s\n' "$1" >&2
    exit 1
}

hf_base=${HF_ENDPOINT:-https://huggingface.co}
proxy=''
output_dir=''
while (($#)); do
    case "$1" in
        --help) usage; exit 0 ;;
        --hf-base|--proxy|--output-dir)
            option=$1
            [[ $# -ge 2 && -n $2 && $2 != --* ]] || fail "${option} requires a value"
            case "$option" in
                --hf-base) hf_base=$2 ;;
                --proxy) proxy=$2 ;;
                --output-dir) output_dir=$2 ;;
            esac
            shift 2
            ;;
        *) fail 'unknown argument (use --help)' ;;
    esac
done

case "$hf_base" in
    http://?*|https://?*) ;;
    *) fail '--hf-base must be an HTTP or HTTPS URL' ;;
esac
[[ $hf_base != *[[:space:]]* && $hf_base != *\?* && $hf_base != *\#* ]] || fail 'invalid --hf-base URL'
if [[ -n $proxy ]]; then
    case "$proxy" in
        http://?*|https://?*|socks4://?*|socks4a://?*|socks5://?*|socks5h://?*) ;;
        *) fail '--proxy must be an HTTP, HTTPS, or SOCKS URL' ;;
    esac
    [[ $proxy != *[[:space:]]* ]] || fail 'invalid --proxy URL'
fi
if [[ -z $output_dir ]]; then
    if [[ -n ${XDG_DATA_HOME:-} ]]; then
        output_dir=$XDG_DATA_HOME/voicetype/models/sensevoice
    else
        [[ -n ${HOME:-} ]] || fail 'HOME or XDG_DATA_HOME must be set, or supply --output-dir'
        output_dir=$HOME/.local/share/voicetype/models/sensevoice
    fi
fi
command -v curl >/dev/null 2>&1 || fail 'curl is required'
mkdir -p -- "$output_dir"
output_dir=$(cd -- "$output_dir" && pwd -P)
[[ $output_dir == /* && $output_dir != / ]] || fail 'output directory must not be the filesystem root'
for name in model.int8.onnx tokens.txt; do
    [[ ! -d $output_dir/$name ]] || fail 'a destination filename is a directory'
done

stage=$(mktemp -d "$output_dir/.voicetype-download.XXXXXXXX")
# Only remove known files in our verified, immediate staging child. Never recurse
# through the output directory, whose other contents belong to the user.
cleanup() {
    local resolved
    [[ -n ${stage:-} && $stage == "$output_dir"/.voicetype-download.* && ${stage%/*} == "$output_dir" && ! -L $stage ]] || return 0
    resolved=$(cd -- "$stage" 2>/dev/null && pwd -P) || return 0
    [[ $resolved == "$stage" && $resolved == /* && $resolved != / ]] || return 0
    rm -f -- "$resolved/model.int8.onnx" "$resolved/tokens.txt"
    rmdir -- "$resolved" 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

curl_args=(--silent --fail --location --proto '=http,https' --proto-redir '=http,https'
    --connect-timeout 15 --max-time 600 --retry 2 --retry-max-time 1200)
if [[ -n $proxy ]]; then
    curl_args+=(--proxy "$proxy")
fi
repo=csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17
for name in model.int8.onnx tokens.txt; do
    printf 'Downloading %s...\n' "$name"
    # -q ignores curlrc: configuration must not enable tracing credentials or
    # execute unrelated transfers. Suppress curl diagnostics containing URLs.
    if ! curl -q "${curl_args[@]}" --output "$stage/$name" \
        "${hf_base%/}/$repo/resolve/main/$name" 2>/dev/null; then
        fail "download failed for $name (check endpoint, proxy, and connectivity)"
    fi
    [[ -s $stage/$name ]] || fail "empty download for $name"
    if LC_ALL=C head -c 4096 "$stage/$name" | LC_ALL=C grep -aEi '<!doctype[[:space:]]+html|<html([[:space:]>])' >/dev/null; then
        fail "HTML response received for $name"
    fi
done

# Each rename is atomic on this filesystem; the two-file update is not.
mv -f -- "$stage/model.int8.onnx" "$output_dir/model.int8.onnx"
mv -f -- "$stage/tokens.txt" "$output_dir/tokens.txt"
printf 'SenseVoice model and tokens installed in %s\n' "$output_dir"
