# Integrating ratchet with Quasar

This document describes the full design for running `ratchet` as a
compile-time upgrade-safety check inside the [Quasar](https://quasar-lang.com)
compiler, and what is required from Quasar upstream to light up each
piece.

## Status today

Quasar programs already work with `ratchet` in two modes:

1. **Via the Anchor IDL they emit.** Quasar's build pipeline produces
   an Anchor-compatible IDL in `target/idl/<program>.json`. Point
   `ratchet check-upgrade` at that file and everything from Phase 0–4
   applies transparently — including the 13-rule engine, `ratchet.lock`
   baselines, and `ratchet replay`.
2. **Via the `ratchet-quasar` crate's `check_pair` helper.** A Quasar
   compiler pass that has access to the old and new schema ASTs can
   invoke `check_pair(old, new, ctx)` directly and emit compile errors
   when the report contains Breaking findings.

What is *not* yet implemented is a source-level Quasar parser in
`ratchet-source`. That step is blocked by Quasar itself, not by
`ratchet`, and is described below.

## Three integration surfaces

### A. Quasar compiler plugin (ideal)

The strongest guarantee: Quasar refuses to compile a program whose
upgrade would be breaking, unless the breaking change is acknowledged
in source (a `#[migration]` attribute, or similar).

What Quasar upstream needs to do:
- Expose a stable plugin/hook API that runs after semantic analysis
  and before codegen, with a visitor over the fully-resolved schema.
- Provide access to the previous release's schema — either by checking
  it into source (`quasar.lock.json`), by hitting a registry, or via
  `git` on a local branch.

What `ratchet-quasar` exposes today for that plugin to consume:
- `ProgramSurface` (the IR) and `SurfaceBuilder`.
- `check_pair(old, new, ctx) -> Report`.
- `default_rules()` re-exported for filterable subsets.

### B. `quasar check-upgrade` subcommand (medium effort)

Drop-in CLI invocation from inside a Quasar project:

```sh
quasar check-upgrade --new target/idl/vault.json --lock ratchet.lock
```

This is essentially `ratchet check-upgrade` vendored as a Quasar
subcommand so Quasar developers don't need a separate binary. Requires
nothing from ratchet beyond the existing `ratchet-cli` crate; Quasar
just invokes it as a subprocess.

### C. Native Quasar schema loader (deferred)

Analogous to `ratchet-source` for Anchor. Would walk a Quasar project
directory, find schema/program declaration nodes, and extract PDA seed
expressions and account layouts directly from Quasar syntax instead of
from the emitted Anchor IDL.

Blocked on Quasar shipping:
- A stable syntax for account declarations.
- A stable representation for PDA seed expressions comparable to
  Anchor's `#[account(seeds = [...])]`.
- A public grammar or `syn`-compatible parser crate.

Until then, relying on Quasar's Anchor-IDL output (surface A) covers
the same ground — the IDL already contains everything ratchet needs.

## Forward-compatible types

`ratchet_quasar::QuasarSchema` is a forward-declared, serde-visible
struct that Quasar's compiler can target as its exported schema format
once the schema stabilises. Today it is effectively a thin wrapper
around `ProgramSurface` — when Quasar lands with native features the
IDL doesn't express, this struct gains fields and a normalizer is
added.

## Recommended adoption path

1. **Today.** Drop `ratchet check-upgrade` into Quasar projects via
   the existing CLI and the `ratchet` GitHub Action. No Quasar-side
   changes needed — it reads the Anchor IDL Quasar already emits.
2. **v0.2.** Add a `quasar check-upgrade` thin wrapper in the Quasar
   CLI that calls `ratchet` internally.
3. **v0.3.** When Quasar's compiler API stabilises, ship the plugin
   described in (A) so breaking changes become compile errors. Use
   `ratchet_quasar::check_pair` and emit findings through Quasar's
   diagnostic machinery.
4. **v0.4.** Add `ratchet-source`-equivalent Quasar source parser if
   Quasar's schema diverges from Anchor's IDL in ways the IDL can't
   capture.
