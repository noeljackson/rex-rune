#!/usr/bin/env bash
# Install a user launcher pointing to this checkout's release build (or a supplied binary).
#   install-desktop.sh [--copy] [path/to/rex-linux]
#   install-desktop.sh --uninstall
# --copy installs the binary into $XDG_BIN_HOME (default ~/.local/bin) and points the
# launcher there, so it keeps working if the checkout moves.
# PREFIX selects PREFIX/bin and PREFIX/share instead of XDG defaults.
# BINDIR/DATADIR override those directories; DESTDIR stages files, not launcher paths.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${PREFIX:-}" ]]; then
    bin_dir="${BINDIR:-$PREFIX/bin}"
    data_dir="${DATADIR:-$PREFIX/share}"
else
    bin_dir="${BINDIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
    data_dir="${DATADIR:-${XDG_DATA_HOME:-$HOME/.local/share}}"
fi
destdir="${DESTDIR:-}"
for path in "$bin_dir" "$data_dir" "${destdir:-/}"; do
    if [[ "$path" != /* || "$path" == *$'\n'* ]]; then
        echo "Install directories must be absolute paths without newlines: $path" >&2
        exit 2
    fi
done
bin_dir="$(realpath -ms -- "$bin_dir")"
data_dir="$(realpath -ms -- "$data_dir")"
if [[ -n "$destdir" ]]; then
    destdir="$(realpath -ms -- "$destdir")"
    destdir="${destdir%/}"
fi
install_bin_dir="$destdir$bin_dir"
applications="$destdir$data_dir/applications"
icons="$destdir$data_dir/icons/hicolor"
desktop="$applications/dev.shuv.rex-linux.desktop"
receipt="$destdir$data_dir/rex-linux/install.json"

usage() {
    echo "Usage: $0 [--copy] [path/to/rex-linux] | --uninstall" >&2
    exit 2
}

refresh_caches() {
    # A package stage must never refresh the host's caches.
    [[ -z "${DESTDIR:-}" ]] || return 0
    if command -v gtk-update-icon-cache >/dev/null && [[ -d "$icons" ]]; then
        gtk-update-icon-cache -f -t "$icons" >/dev/null 2>&1 || true
    fi
    if command -v update-desktop-database >/dev/null && [[ -d "$applications" ]]; then
        update-desktop-database "$applications"
    fi
}

# Quote a value as one argument of a desktop entry's Exec key.
desktop_exec_quote() {
    local value="$1"
    if [[ "$value" == *$'\n'* ]]; then
        echo "Desktop launcher paths cannot contain newlines" >&2
        return 1
    fi
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    value="${value//\`/\\\`}"
    value="${value//\$/\\\$}"
    printf '"%s"' "$value"
}

copy=false
case "${1:-}" in
    --uninstall)
        [[ $# -eq 1 ]] || usage
        rm -f -- "$install_bin_dir/rex-linux" "$desktop" "$receipt"
        for size in 32 128 256; do
            rm -f -- "$icons/${size}x${size}/apps/dev.shuv.rex-linux.png"
        done
        refresh_caches
        echo "Removed Rex Linux's launcher, icons and $install_bin_dir/rex-linux"
        exit 0
        ;;
    --copy)
        copy=true
        shift
        ;;
    -*) usage ;;
esac
receipt_version=""
if [[ "${1:-}" == --receipt-version ]]; then
    [[ $# -ge 2 && "$2" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
    $copy || usage
    receipt_version="$2"
    shift 2
fi
[[ $# -le 1 ]] || usage
if [[ -n "${DESTDIR:-}" ]] && ! $copy; then
    echo "DESTDIR staging requires --copy" >&2
    exit 2
fi
source_binary="$(realpath -m -- "${1:-$root/target/release/rex-linux}")"

if [[ ! -x "$source_binary" ]]; then
    echo "Build Rex Linux first: make (or cargo build --locked --release --bin rex-linux)" >&2
    exit 1
fi
for tool in install desktop-file-install desktop-file-validate update-desktop-database; do
    if ! command -v "$tool" >/dev/null; then
        echo "Install requirement: missing $tool; install coreutils and desktop-file-utils." >&2
        exit 1
    fi
done

# Source installs invalidate a previous official receipt, even at the same path.
rm -f -- "$receipt"
binary="$source_binary"
if $copy; then
    binary="$bin_dir/rex-linux"
    if [[ ! "$source_binary" -ef "$install_bin_dir/rex-linux" ]]; then
        install -Dm755 "$source_binary" "$install_bin_dir/rex-linux"
    fi
fi
exec_value="$(desktop_exec_quote "$binary")"

for size in 32 128 256; do
    install -Dm644 "$root/resources/icons/${size}x${size}/apps/dev.shuv.rex-linux.png" \
        "$icons/${size}x${size}/apps/dev.shuv.rex-linux.png"
done

desktop-file-install --dir="$applications" \
    --set-key=Exec --set-value="$exec_value" \
    --set-key=TryExec --set-value="$binary" \
    "$root/resources/dev.shuv.rex-linux.desktop"
desktop-file-validate "$desktop"
refresh_caches
if [[ -n "$receipt_version" ]]; then
    # JSON strings, including unusual XDG paths, without adding a Python dependency.
    json_string() {
        local value="$1" char escaped n
        value="${value//\\/\\\\}"
        value="${value//\"/\\\"}"
        for ((n=1; n<32; n++)); do
            printf -v char '%b' "\\$(printf '%03o' "$n")"
            printf -v escaped '\\u%04x' "$n"
            value="${value//"$char"/"$escaped"}"
        done
        printf '"%s"' "$value"
    }
    installed_path="$(realpath -m -- "$binary")"
    digest="$(sha256sum < "$install_bin_dir/rex-linux")"
    digest="${digest%% *}"
    mkdir -p -- "$(dirname -- "$receipt")"
    temp_receipt="$(mktemp "${receipt}.XXXXXX")"
    trap 'rm -f -- "$temp_receipt"' EXIT
    printf '{"schema":1,"path":%s,"version":"%s","sha256":"%s"}\n' \
        "$(json_string "$installed_path")" "$receipt_version" "$digest" > "$temp_receipt"
    mv -f -- "$temp_receipt" "$receipt"
fi
echo "Installed $desktop"
