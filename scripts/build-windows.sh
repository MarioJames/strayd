#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

for tool in bun cargo-xwin makensis llvm-rc lld; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf '缺少 Windows 打包依赖：%s\n' "$tool" >&2
    exit 1
  fi
done

export XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/cargo-xwin}"
export XWIN_ARCH="${XWIN_ARCH:-x86_64}"
export XWIN_HTTP_RETRIES="${XWIN_HTTP_RETRIES:-8}"
rustup target add x86_64-pc-windows-msvc
bun install --frozen-lockfile
bun tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc

printf '\n安装包：%s\n' "$project_root/target/x86_64-pc-windows-msvc/release/bundle/nsis/"
