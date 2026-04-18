# ratchet web

Next.js 15 frontend for ratchet. Three surfaces:

- **Landing** — what ratchet is, three-step explainer, highlighted rules, install snippet.
- **/diff** — drop two Anchor IDL JSONs, see every finding with the exact old/new values and allow-flag suggestion.
- **/rules** — complete 16-rule catalog with severity and allow-flag columns.
- **/skill.md** — serves the repo-root `SKILL.md` so `domain.com/skill.md` is canonical for agent discovery.

## Dev

```sh
cd web
pnpm install   # or npm install / yarn install
pnpm dev
```

Visit http://localhost:3000.

The diff page POSTs to `/api/check`, which shells out to the locally-installed `ratchet` CLI. Install it first:

```sh
cargo install --path ../crates/ratchet-cli    # from a checkout
# or once published:
cargo install solana-ratchet-cli
```

The API route honours the `RATCHET_BIN` env var if the binary isn't on `PATH`:

```sh
RATCHET_BIN=/path/to/ratchet pnpm dev
```

## Deployment

Any Node-host that can install a Rust binary works — Fly.io, Railway, a Dockerfile on any platform, etc. Vercel's serverless functions don't include Rust binaries, so production deployment on Vercel needs either:

1. A custom build step that pulls a precompiled `ratchet` binary into the bundle, or
2. A companion API hosted elsewhere (Fly.io, a small VPS), or
3. (Future) a WASM-compiled version of `ratchet-core` + `ratchet-anchor` loaded in the browser, removing the server round-trip entirely.

The third path is the eventual destination.

## Design

- Dark first, Solana purple (`#9945ff`) + green (`#14f195`) as the primary accents — terminal-adjacent aesthetic that matches the CLI output.
- Monospace for everything that maps to on-chain bytes (pubkeys, discriminators, rule ids, diff lines).
- Severity palette: breaking `#ff3d57`, unsafe `#f5a524`, additive reuses the Solana green.
- One single-page feel: navigation is flat, no deep menus.

All colors declared as CSS custom properties in `app/globals.css`.
