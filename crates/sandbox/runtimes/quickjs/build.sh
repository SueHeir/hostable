#!/usr/bin/env bash
# Build quickjs-rules-host.wasm: QuickJS (quickjs-ng) plus glue.c as a rules
# module whose only import is env.log.
#
# Needs: a clang with the wasm32 backend (Homebrew `llvm` works; Apple's
# clang doesn't), rustup's bundled rust-lld, curl and tar. It downloads the
# quickjs-ng sources and the wasi-sdk sysroot (headers and libc only; no
# downloaded program is run) into a cache directory.
#
# The result is committed as ../quickjs-rules-host.wasm so building Hostable
# doesn't need any of this. Re-run it to update QuickJS.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
QJS_VERSION="${QJS_VERSION:-0.18.0}"
WASI_SDK_VERSION="${WASI_SDK_VERSION:-34}"
cache="${CACHE_DIR:-$here/../../../../target/quickjs-build}"
mkdir -p "$cache"
cd "$cache"

CLANG="${CLANG:-}"
if [ -z "$CLANG" ]; then
  for c in /opt/homebrew/opt/llvm/bin/clang /usr/local/opt/llvm/bin/clang "$(command -v clang || true)"; do
    if [ -x "$c" ] && "$c" --print-targets 2>/dev/null | grep -q wasm32; then CLANG="$c"; break; fi
  done
fi
[ -n "$CLANG" ] || { echo "need a clang with the wasm32 target (brew install llvm)"; exit 1; }
LLD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/host: //p')/bin/rust-lld"
[ -x "$LLD" ] || { echo "rust-lld not found at $LLD (install Rust with rustup)"; exit 1; }

if [ ! -d "quickjs-$QJS_VERSION" ]; then
  curl -fsSL "https://github.com/quickjs-ng/quickjs/archive/refs/tags/v$QJS_VERSION.tar.gz" | tar xz
fi
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) sdk_arch=arm64-macos ;;
  Darwin-x86_64) sdk_arch=x86_64-macos ;;
  Linux-x86_64) sdk_arch=x86_64-linux ;;
  Linux-aarch64) sdk_arch=arm64-linux ;;
  *) echo "unsupported build platform"; exit 1 ;;
esac
sdk="wasi-sdk-$WASI_SDK_VERSION.0-$sdk_arch"
if [ ! -d "$sdk" ]; then
  curl -fsSL "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-$WASI_SDK_VERSION/$sdk.tar.gz" | tar xz
fi
sysroot="$cache/$sdk/share/wasi-sysroot"
builtins="$(ls "$cache/$sdk"/lib/clang/*/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a | head -1)"
Q="$cache/quickjs-$QJS_VERSION"

objs=()
for f in "$here/glue.c" "$Q/quickjs.c" "$Q/libregexp.c" "$Q/libunicode.c" "$Q/dtoa.c"; do
  o="$cache/$(basename "$f" .c).o"
  "$CLANG" --target=wasm32-wasip1 --sysroot="$sysroot" -O2 -D_GNU_SOURCE -I"$Q" -c "$f" -o "$o" -w
  objs+=("$o")
done
libdir="$sysroot/lib/wasm32-wasip1"
"$LLD" -flavor wasm --no-entry -O2 --strip-all -z stack-size=1048576 \
  "$libdir/crt1-reactor.o" "${objs[@]}" -L"$libdir" -lc "$builtins" \
  -o "$here/../quickjs-rules-host.wasm"
echo "built $(wc -c < "$here/../quickjs-rules-host.wasm") bytes"
