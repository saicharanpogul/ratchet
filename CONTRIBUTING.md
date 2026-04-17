# Contributing to ratchet

Thanks for considering a contribution. The codebase is small and the
invariants are load-bearing — upgrade-safety tools *must* be correct
and conservative — so the contribution bar is accuracy first,
polish second.

## Development

```sh
cargo test --workspace          # 190+ tests; every rule has fire + non-fire cases
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ratchet-svm --features litesvm  # exercises the feature-gated deploy path
```

CI runs all four on every PR.

## Adding a rule

Rules live under `crates/ratchet-core/src/rules/`, one file per rule,
named `rXXX_<kebab_identifier>.rs`. A rule must:

- Have a stable `ID`, `NAME`, and `DESCRIPTION` (constants in the
  module). Never change `ID` after it's been published.
- Emit `Breaking` when the diff will corrupt on-chain state or
  break existing clients. `Unsafe` when a declared migration or an
  explicit `--unsafe` acknowledgement is the fix. `Additive` for
  visibility-only findings that never fail CI.
- Provide an `allow_flag` on non-fundamental breaks so authors can
  demote them via `--unsafe <flag>`. Changes with no safe
  acknowledgement (e.g. reorder of shared fields) should omit the
  flag entirely.
- Ship with at least two tests: one that fires on the target pattern
  and one that *doesn't* fire on an adjacent pattern (avoids
  over-firing regressions).
- Register itself in `crates/ratchet-core/src/rules/mod.rs::all()`.

## Adding a dependency

Keep the dep graph tight. Any new dep on a new crate needs a note in
the PR description explaining why an existing crate couldn't do the
job. `ratchet-core`, `ratchet-anchor`, `ratchet-lock`, and
`ratchet-source` should *not* pull in `solana-sdk`. `ratchet-svm`
may, but only behind the `litesvm` feature flag.

## Commit style

Conventional-style prefixes: `feat(…)`, `fix(…)`, `chore(…)`,
`docs(…)`, `test(…)`, `style(…)`, `refactor(…)`. Messages explain
the *why*, not just the *what*. One logical change per commit.

## Reporting bugs

Open a GitHub issue with:
- A reduced IDL pair (or `ratchet.lock` + candidate) demonstrating
  the misfire or miss.
- The `ratchet --json` output.
- What you expected.

Security-sensitive reports go to the address in [SECURITY.md](SECURITY.md).
