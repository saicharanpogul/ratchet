# ratchet ↔ Quasar — integration roadmap

[Quasar](https://github.com/blueshift-gg/quasar) is a compile-time
Solana program framework currently in active beta development at
Blueshift. This doc describes how ratchet integrates with Quasar
projects today and how that integration is expected to evolve as
Quasar's own surfaces mature.

The roadmap describes **ratchet's plan** for matching Quasar's
evolution. It is not a proposal for what Quasar should ship, and it
makes no commitments on Quasar's behalf.

## Today — IDL JSON parser + normalizer

`quasar build` emits an IDL JSON at `target/idl/<program>.json`. The
shape is structurally distinct from Anchor's (variable-length
discriminators, untagged `IdlType` union, struct-only typedefs, no
`program` field on PDAs), so ratchet ships a dedicated parser and
normalizer in `crates/ratchet-quasar/`. Both Quasar and Anchor lower
into the same framework-agnostic `ProgramSurface` IR, so every R-rule
and P-rule applies identically once normalised.

Workflow:

```sh
quasar build
ratchet readiness     --new target/idl/<program>.json   # auto-detects Quasar.toml
ratchet check-upgrade --old <baseline>.json --new target/idl/<program>.json
```

Or via the CI helper at [`action-quasar.yml`](../action-quasar.yml).

Two semantics carry over from Quasar's shape:

- Discriminators are padded from `Vec<u8>` (typically 1 byte) to
  ratchet's 8-byte slot with trailing zeros. R006 still detects byte
  changes; the padding choice keeps Quasar's leading-position dispatch
  bytes in the same position ratchet's diff inspects.
- P003 / P004 (default-discriminator-pin) stay silent on Quasar
  surfaces because Quasar devs always assign discriminators
  explicitly. The padded bytes never match `sha256("account:<Name>")
  [..8]`, so the rule has no signal to fire on — which is the right
  semantics, not a gap.

## Soon — `__QUASAR_SCHEMA` binary canonical reader

Open Quasar PR
[#177](https://github.com/blueshift-gg/quasar/pull/177) (`feat(schema-canonical):
canonical binary encoding of Idl`) proposes a deterministic binary
encoding for Quasar IDLs, embedded in the program binary as
`pub const __QUASAR_SCHEMA: &[u8]` and storable on-chain at a
per-program PDA via a reserved upgrade discriminator (`0xFE`). The
schema spec lives at `schema-canonical/SPEC.md` in that PR's
contributor fork.

When the canonical schema lands in upstream Quasar, ratchet will:

1. Add `ratchet_quasar::load_quasar_schema_binary(&[u8])` that
   verifies the magic bytes, version, and SHA-256 digest, then
   deserialises the body.
2. Extend the `--quasar` loader path to detect binary payloads
   alongside today's JSON (peek-byte on the magic prefix).
3. Add a `ratchet check-upgrade --schema-pda <pubkey>` flow that
   fetches the canonical schema directly from the on-chain PDA the
   PR-1b instruction writes — closes the loop on "no IDL JSON
   committed in the repo, but the program publishes its schema
   on-chain."

This is a strict superset of today's behaviour; the JSON path stays
supported for Quasar versions that emit it.

## Eventual — compiler-pass integration

The ideal integration is ratchet running as a Quasar compiler pass:
an unsafe upgrade refuses to compile, full stop. Quasar's
`CONTRIBUTING.md` documents that the project is in beta and not
accepting external PRs at this time, so this integration is on a
slower track than the runtime ones above.

The shape would look like:

```rust
// inside Quasar's `cli/src/build.rs`, after parse_program() emits
// the IDL but before cargo build-sbf runs:
let new = quasar_to_surface(&parsed_program);
let old = ratchet_lock::Lockfile::read(".ratchet.lock")?;
let report = ratchet_quasar::check_pair(&old, &new, &CheckContext::new());
if report.has_breaking() {
    bail!("ratchet: breaking IDL change vs ratchet.lock — refusing to build");
}
```

What ratchet provides today to make this drop-in:

- `ratchet_quasar::SurfaceBuilder` — fluent builder for assembling a
  `ProgramSurface` from an AST without going through JSON. Useful when
  the compiler already has the parsed shape in memory.
- `ratchet_quasar::check_pair` and `check_pair_readiness` — one-call
  wrappers around the diff and preflight rule sets.
- `ratchet_quasar::QuasarSchema` envelope — forward-compatible spec
  versioning so a future native format can land without breaking
  callers.

What's needed from Quasar (when its API stabilises and PRs are
re-opened):

- A documented compiler-pass / plugin / `post-emit` hook surface, or
  any extension point that runs after `parse_program()` and before
  `cargo build-sbf`.
- A lockfile-or-manifest convention for "what is the current on-chain
  baseline" — probably aligned with whatever `quasar schema upload`
  ultimately uses for the on-chain schema PDA.

Until then, the wrapper pattern (`quasar build && ratchet ...`) plus
[`action-quasar.yml`](../action-quasar.yml) is the idiomatic
"compile-time linter" experience Quasar projects can use today.

## What ratchet does *not* do for Quasar (yet)

- **Does not parse Quasar source code directly.** Anchor has a
  `ratchet-source` crate that walks `#[account(seeds = ...)]`
  attributes from `.rs` files. Quasar's macro layer is different
  enough that we'd need a separate parser; deferred until the JSON
  path proves insufficient for real users.
- **Does not differentiate Quasar enum-typedefs.** Quasar's
  `IdlTypeDefKind` enum currently has a single `Struct` variant, so
  there's nothing to differentiate. When upstream adds enum support,
  the normalizer will need an `IdlTypeDefKind::Enum → TypeDef::Enum`
  arm.
- **Does not lint Quasar's own L001–L009 in-tree linter rules** (no
  signer on mut, has-one consistency, etc.). Those overlap conceptually
  with ratchet's rules but live inside `quasar build`. Running both
  layers gives strictly more coverage.

## Tracking

When upstream signals shift, the changes here trigger:

- PR [#177](https://github.com/blueshift-gg/quasar/pull/177) merges →
  add the binary schema reader in `ratchet_quasar`.
- New `IdlTypeDefKind` variant ships → extend the normalizer.
- A plugin / `post-emit` API appears in `Quasar.toml` or
  `cli/src/build.rs` → add a worked compiler-pass example here +
  upstream a wrapper (subject to the project's policy at the time).
