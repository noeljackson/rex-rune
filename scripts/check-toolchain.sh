#!/usr/bin/env bash
# Check build tools without installing anything or contacting a Rex server.
set -euo pipefail

fail() { echo "Build requirement: $*" >&2; exit 1; }

cargo="${CARGO:-cargo}"
rustc="${RUSTC:-rustc}"
zig="${ZIG:-zig}"
for tool in "$cargo" "$rustc" "$zig"; do
    command -v "$tool" >/dev/null || fail "missing $tool; install Rust 1.96+ and Zig 0.17.x, or set CARGO/RUSTC/ZIG."
done

version="$("$rustc" --version)"
[[ "$version" =~ ^rustc\ ([0-9]+)\.([0-9]+)\. ]] || fail "cannot read Rust version: $version"
(( BASH_REMATCH[1] > 1 || (BASH_REMATCH[1] == 1 && BASH_REMATCH[2] >= 96) )) ||
    fail "Rust 1.96+ required; found $version."

version="$("$zig" version)"
[[ "$version" =~ ^0\.17\.[0-9]+$ ]] || fail "Zig 0.17.x required; found $version. Set ZIG to a supported executable."
