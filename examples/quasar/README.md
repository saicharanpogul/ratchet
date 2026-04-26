# Quasar example IDLs

Two committed IDL JSON fixtures shaped exactly like
[`quasar build`](https://github.com/blueshift-gg/quasar) emits at
`target/idl/<program>.json`. Used for:

- The repo's `crates/ratchet-quasar/tests/` integration suite —
  exercises the full parse → normalize → run-rules pipeline against
  representative Quasar shapes.
- A worked README example (`Using ratchet with Quasar` section in the
  repo root) so a Quasar dev can copy + run the commands without
  installing Quasar's toolchain first.

## Files

- **`escrow.json`** — a 3-instruction (`make`, `take`, `refund`)
  escrow program. One account type (`Escrow`), two events,
  `version`-field free / `_reserved`-padding free — i.e. a typical
  hackathon shape that *should* trigger the readiness lint.
- **`escrow.v2.json`** — a deliberately-broken upgrade of `escrow.json`
  that hits multiple R-rules at once for `check-upgrade` demos:
  - removes the `refund` instruction (R007 `instruction-removed`)
  - retypes `make.receive` from `u64` → `u32` (R008
    `instruction-arg-type-change`)
  - reorders the `Escrow` account fields (R001
    `account-field-reorder`)
  - changes the `Escrow` discriminator from `[42]` → `[99]` (R006
    `account-discriminator-change`)

## Try it

```sh
ratchet readiness --new examples/quasar/escrow.json --quasar
ratchet check-upgrade \
  --old examples/quasar/escrow.json \
  --new examples/quasar/escrow.v2.json \
  --quasar
```

Expect the readiness run to flag missing version field + reserved
padding (UNSAFE), and the check-upgrade run to fire R001 / R006 /
R007 / R008 (BREAKING).

## Why these are committed (not generated in CI)

Building Quasar programs requires Quasar's toolchain (a Rust
proc-macro framework + `quasar build` CLI), which we'd rather not
add to ratchet's CI. The IDL files here are manually shaped to match
the JSON Quasar's compiler emits — verified against
[`blueshift-gg/quasar`](https://github.com/blueshift-gg/quasar)'s
`schema/src/lib.rs`. When upstream's schema evolves, regenerate by
running `quasar build` against any of their `examples/` programs
and copy the resulting `target/idl/*.json` here.
