#!/usr/bin/env sh
set -eu

WINDOWS_TARGET="x86_64-pc-windows-msvc"
LINUX_TARGET="x86_64-unknown-linux-musl"

printf '==> Installing Rust targets if needed\n'
rustup target add "$WINDOWS_TARGET"
rustup target add "$LINUX_TARGET"

printf '\n==> Building Windows release: %s\n' "$WINDOWS_TARGET"
cargo build --release --target "$WINDOWS_TARGET"

printf '\n==> Building Linux release: %s\n' "$LINUX_TARGET"
if command -v cargo-zigbuild >/dev/null 2>&1; then
  cargo zigbuild --release --target "$LINUX_TARGET"
else
  printf 'cargo-zigbuild not found. Install it with:\n'
  printf '  cargo install cargo-zigbuild\n'
  exit 1
fi

printf '\n==> Done\n'
printf 'Windows: target/%s/release/markdown-preview-service.exe\n' "$WINDOWS_TARGET"
printf 'Linux:   target/%s/release/markdown-preview-service\n' "$LINUX_TARGET"
