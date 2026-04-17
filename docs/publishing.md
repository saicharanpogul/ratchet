# Publishing ratchet crates

Maintainer runbook. Assumes you have `cargo login <token>` already set
up on this machine with a crates.io API token that owns (or is about
to own) the `solana-ratchet-*` namespace.

## Pre-flight

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p solana-ratchet-svm --features litesvm
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

All four must be green before publishing. (CI enforces this on every
push.)

## First-time publish

crates.io requires each crate to be published before any crate that
depends on it. Publish in dependency order:

```sh
cargo publish -p solana-ratchet-core
cargo publish -p solana-ratchet-anchor     # depends on core
cargo publish -p solana-ratchet-lock        # depends on core
cargo publish -p solana-ratchet-source      # depends on core
cargo publish -p solana-ratchet-svm         # depends on core + anchor
cargo publish -p solana-ratchet-squads      # depends on anchor
cargo publish -p solana-ratchet-quasar      # depends on core
cargo publish -p solana-ratchet-cli         # depends on everything above
```

crates.io takes a few seconds between accepting a publish and making
it available for downstream resolution. If a `cargo publish` on a
dependent crate fails with "no matching package named …", wait 30
seconds and retry.

Tag after all eight succeed:

```sh
git tag -a v0.1.0 -m "ratchet 0.1.0"
git push --tags
```

Update `action.yml` once tags exist so the GitHub Action pins to
`@v0.1.0` by default instead of rolling `@main`.

## Subsequent releases

1. Bump `[workspace.package].version` in the root `Cargo.toml`.
2. Bump every `version = "x.y.z"` on the path-dep entries inside each
   crate's `[dependencies]` block (one grep + replace per crate).
3. Update `CHANGELOG.md`.
4. Run the pre-flight checks above.
5. `cargo publish` each crate in the same order.
6. Tag and push.

## Yanking

If a broken release ships, yank it instead of deleting. Deletion on
crates.io is irreversible and does not free the version number for
reuse.

```sh
cargo yank -p solana-ratchet-core --version 0.1.0
```

Yanked versions can't be newly-depended-on but existing Cargo.lock
files that pin to them keep resolving.
