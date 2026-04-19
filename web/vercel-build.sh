#!/usr/bin/env bash
# Vercel build wrapper. The web app's `prebuild` hook calls wasm-pack,
# which Vercel's Node-only build image doesn't ship. Bootstrap Rust and
# a prebuilt wasm-pack into $HOME/.cargo/bin so both are on PATH before
# `next build` runs.
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal \
                --target wasm32-unknown-unknown
fi
source "$HOME/.cargo/env"

if ! command -v wasm-pack >/dev/null 2>&1; then
  WP=v0.14.0
  TRIPLE=x86_64-unknown-linux-musl
  curl -fsSL "https://github.com/rustwasm/wasm-pack/releases/download/${WP}/wasm-pack-${WP}-${TRIPLE}.tar.gz" \
    | tar xz -C "$HOME/.cargo/bin" --strip-components=1 \
        "wasm-pack-${WP}-${TRIPLE}/wasm-pack"
fi

npm run build
