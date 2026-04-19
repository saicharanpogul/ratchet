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

# wasm-pack drives `cargo build --target wasm32-unknown-unknown`, so we
# need a Rust toolchain with that target. Vercel's build image ships a
# system rustc at /rust/bin without rustup and without wasm32 support,
# so force-install rustup and prepend it to PATH. rustup is a no-op if
# already installed.
if ! command -v rustup >/dev/null 2>&1; then
  echo "Installing rustup (minimal + wasm32-unknown-unknown)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal \
                --target wasm32-unknown-unknown --no-modify-path
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add wasm32-unknown-unknown 2>/dev/null || true

npm run build
