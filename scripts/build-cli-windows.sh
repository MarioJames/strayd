#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
export XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/cargo-xwin}"
export XWIN_ARCH="${XWIN_ARCH:-x86_64}"
export XWIN_VERSION="${XWIN_VERSION:-17}"
export XWIN_SDK_VERSION="${XWIN_SDK_VERSION:-10.0.26100}"
export XWIN_CRT_VERSION="${XWIN_CRT_VERSION:-14.44.17.14}"
export XWIN_HTTP_RETRIES="${XWIN_HTTP_RETRIES:-8}"

for tool in cargo cargo-xwin; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf '缺少 Windows CLI 构建依赖：%s\n' "$tool" >&2
    exit 1
  fi
done

cargo xwin build \
  --release \
  --target x86_64-pc-windows-msvc \
  --target-dir target/port-deck-cli-windows \
  -p port-deck-cli
