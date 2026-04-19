#!/usr/bin/env bash
# Vercel build wrapper. The web app's `prebuild` hook calls wasm-pack,
# which Vercel's Node-only build image doesn't ship. Install a prebuilt
# wasm-pack binary into a PATH-visible bin dir before `next build` runs.
# A Rust toolchain is NOT required: the wasm artifact ships in a
# precompiled tarball, so no `cargo` invocation happens.
set -euo pipefail

BIN_DIR="$HOME/.local/bin"
mkdir -p "$BIN_DIR"
export PATH="$BIN_DIR:$PATH"

if ! command -v wasm-pack >/dev/null 2>&1; then
  WP=v0.14.0
  TRIPLE=x86_64-unknown-linux-musl
  echo "Installing wasm-pack ${WP} into ${BIN_DIR}"
  curl -fsSL "https://github.com/rustwasm/wasm-pack/releases/download/${WP}/wasm-pack-${WP}-${TRIPLE}.tar.gz" \
    | tar xz -C "$BIN_DIR" --strip-components=1 \
        "wasm-pack-${WP}-${TRIPLE}/wasm-pack"
  chmod +x "$BIN_DIR/wasm-pack"
fi

# wasm-pack drives `cargo build --target wasm32-unknown-unknown`
# internally, so we also need a Rust toolchain with that target. Install
# rustup minimal if cargo isn't already on PATH.
if ! command -v cargo >/dev/null 2>&1; then
  echo "Installing Rust toolchain via rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal \
                --target wasm32-unknown-unknown
  # rustup writes an env shim at ~/.cargo/env.
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
fi

npm run build
