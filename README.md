# ratchet

[![CI](https://github.com/saicharanpogul/ratchet/actions/workflows/ci.yml/badge.svg)](https://github.com/saicharanpogul/ratchet/actions/workflows/ci.yml)
[![solana-ratchet-cli on crates.io](https://img.shields.io/crates/v/solana-ratchet-cli.svg?label=solana-ratchet-cli)](https://crates.io/crates/solana-ratchet-cli)
[![solana-ratchet-core on crates.io](https://img.shields.io/crates/v/solana-ratchet-core.svg?label=solana-ratchet-core)](https://crates.io/crates/solana-ratchet-core)
[![docs.rs](https://img.shields.io/docsrs/solana-ratchet-core?label=docs.rs)](https://docs.rs/solana-ratchet-core)
[![License: Apache-2.0](https://img.shields.io/crates/l/solana-ratchet-cli.svg)](LICENSE)

Upgrade-safety checker for Solana programs.

`ratchet` compares a new program surface against the deployed program on-chain (or a committed `ratchet.lock` baseline) and flags changes that would silently corrupt data, break clients, or orphan PDAs — before the upgrade transaction lands.

## Agent integration

`ratchet` ships with a [`SKILL.md`](SKILL.md) at the repo root — a
frontmattered skill definition any Claude-style agent can load to know
when to invoke ratchet, which flags to use, and how to interpret
findings. The same file is served at `/skill.md` on the website when
deployed.

## Why

Solana program upgrades have no equivalent of `buf breaking` or `@openzeppelin/hardhat-upgrades`. Today a developer can rename an `#[account]` struct, silently change the discriminator, and orphan every account the program owns — `solana program upgrade` will happily land it. `ratchet` closes that gap.

Every diff is classified as:

| Verdict | Exit | Meaning |
|---|---|---|
| `ADDITIVE` | `0` | Backward-compatible. Existing accounts and clients keep working. |
| `UNSAFE` | `2` | Needs a declared migration or `--unsafe-*` acknowledgement. |
| `BREAKING` | `1` | Will corrupt on-chain state, break existing clients, or orphan existing PDAs. |

## Status

Alpha — production-shaped, but pre-1.0. What ships today:

- 22 rules: 16 R-rules (diff-time) + 6 P-rules (preflight readiness).
- **Anchor** IDL adapter — file loader, RPC fetcher with auto-derived
  IDL account address from `--program`, on-chain account decoder.
- **Quasar** IDL adapter — parses + normalises Quasar's
  variable-discriminator JSON shape so every rule applies identically.
  Auto-detects `Quasar.toml`; explicit `--quasar` flag for non-cwd cases.
- `ratchet.lock` committable baselines; `syn`-based Anchor source parser
  that fills in PDA seeds the IDL lost.
- `ratchet replay` — samples live program accounts via RPC and flags
  ones that don't match the new IDL's minimum layout. Optional LiteSVM
  deploy smoke test under the `litesvm-deploy` feature.
- `ratchet squads` — decodes Squads V4 vault-transaction proposals and
  optionally runs `check-upgrade` against the proposal's target program.
- GitHub Actions: `action.yml` (Anchor) + `action-quasar.yml` (Quasar).
- Human + JSON output everywhere, CI-friendly exit codes (0 safe,
  1 breaking, 2 unsafe).

Coming next: stable Quasar `__QUASAR_SCHEMA` binary reader once
[upstream PR](https://github.com/blueshift-gg/quasar/pull/177) lands;
deeper compiler-pass integration when Quasar exposes a plugin surface
(see [`docs/quasar-integration.md`](docs/quasar-integration.md) for our
roadmap).

## Install

```sh
cargo install solana-ratchet-cli
```

The binary is called `ratchet`. Local install from a checkout:

```sh
cargo install --path crates/ratchet-cli
```

Library crates publish under the `solana-ratchet-*` prefix
(`solana-ratchet-core`, `solana-ratchet-anchor`, `solana-ratchet-lock`,
`solana-ratchet-source`, `solana-ratchet-svm`, `solana-ratchet-squads`,
`solana-ratchet-quasar`).

## Usage

### Diff two IDL files

```sh
ratchet check-upgrade \
  --old target/idl/vault.json \
  --new target/idl/vault.json.new
```

### Diff against a committed baseline

```sh
# Snapshot the current surface into ratchet.lock (run once per release)
ratchet lock --from-idl target/idl/vault.json --out ratchet.lock

# In CI, on every PR:
ratchet check-upgrade --lock ratchet.lock --new target/idl/vault.json
```

### Diff against the on-chain IDL

```sh
# Auto-derive the Anchor IDL account address from the program id
ratchet check-upgrade \
  --program <PROGRAM_ID> \
  --cluster mainnet \
  --new target/idl/vault.json

# Or point at an explicit IDL account (e.g. for programs with custom layouts)
ratchet check-upgrade \
  --idl-account <IDL_ACCOUNT_PUBKEY> \
  --cluster mainnet \
  --new target/idl/vault.json
```

### Augment from source for richer PDA checks

Anchor 0.30+ IDLs capture PDA seeds but sometimes flatten account-field references. Point `ratchet` at your program source to parse `#[account(seeds = [...])]` directly:

```sh
ratchet check-upgrade --lock ratchet.lock --new target/idl/vault.json \
  --new-source programs/vault/src
```

### Using ratchet with [Quasar](https://github.com/blueshift-gg/quasar)

Quasar emits an IDL JSON at `target/idl/<program>.json` after `quasar build`. Same workflow as Anchor — point `ratchet readiness` / `check-upgrade` at the file. Inside a Quasar workspace, ratchet detects `Quasar.toml` and switches loaders automatically:

```sh
# From a Quasar repo root — autodetect picks the Quasar parser.
quasar build
ratchet readiness --new target/idl/escrow.json
ratchet check-upgrade \
  --old examples/quasar/escrow.json \
  --new target/idl/escrow.json
```

Outside a Quasar workspace, force the Quasar parser explicitly:

```sh
ratchet readiness --new path/to/quasar.json --quasar
ratchet check-upgrade --old old.json --new new.json --quasar
```

Try the committed examples without installing Quasar:

```sh
ratchet readiness  --new examples/quasar/escrow.json --quasar
ratchet check-upgrade \
  --old examples/quasar/escrow.json \
  --new examples/quasar/escrow.v2.json \
  --quasar
```

The same R-rule + P-rule catalog applies; only the loader differs. Two semantics notes:

- **Quasar uses 1-byte discriminators.** Ratchet pads to its 8-byte
  internal slot with trailing zeros. R006 (account-discriminator-change)
  fires on any byte change, same as Anchor.
- **P003 / P004 (default-discriminator-pin) stay silent on Quasar.**
  Quasar devs always assign discriminators explicitly
  (`#[instruction(discriminator = N)]`), so the "is this the Anchor
  sha256 default?" check is a category error there.

A drop-in CI workflow lives at [`action-quasar.yml`](action-quasar.yml). The roadmap for matching Quasar's evolution (binary `__QUASAR_SCHEMA`, eventual plugin API) is in [`docs/quasar-integration.md`](docs/quasar-integration.md).

### Sample live accounts and verify they still match

```sh
ratchet replay --program <PROGRAM_ID> \
  --new target/idl/vault.json \
  --limit 500 \
  --so target/deploy/vault.so
```

Pulls up to 500 program-owned accounts via `getProgramAccounts`, classifies each by the Anchor discriminator, and flags any whose data is shorter than the new IDL's minimum layout. Optional `--so` verifies the candidate binary's ELF header (magic, 64-bit, little-endian, SBF/SBPF shared object) before sampling — catches pushes of the wrong target build.

### Summarise a Squads V4 upgrade proposal (and auto-diff)

```sh
# Basic classification
ratchet squads --proposal <VAULT_TX_PUBKEY> --cluster mainnet

# Full signer experience: decode + fetch current IDL + run check-upgrade
ratchet squads --proposal <VAULT_TX_PUBKEY> \
  --auto-diff --new target/idl/vault.json
```

Full Borsh decode pulls the concrete `program_id` and `buffer` pubkeys straight off the `CompiledInstruction`. With `--auto-diff`, `ratchet` fetches the current on-chain IDL for the proposal's target program and runs `check-upgrade` against the candidate IDL you provide — the signer sees the exact schema diff before clicking approve.

### LiteSVM deploy smoke test (optional)

```sh
cargo install solana-ratchet-cli --features litesvm-deploy

ratchet replay --program <PID> --new target/idl/vault.json \
  --so target/deploy/vault.so --deploy
```

`--deploy` loads the `.so` into an in-process LiteSVM to confirm the runtime accepts the bytecode. The feature is opt-in because LiteSVM pulls in the Solana runtime crates; default builds stay lightweight and use the ELF-header check.

### Acknowledge an intentional change

```sh
# Demote the R006 finding for a deliberate struct rename
ratchet check-upgrade --lock ratchet.lock --new new.json \
  --unsafe allow-rename

# Declare a Migration<From, To> for the Vault account — demotes
# R003 (removed), R004 (mid-insert), R005 (append) for that account.
ratchet check-upgrade --lock ratchet.lock --new new.json \
  --migrated-account Vault

# Declare an Anchor realloc constraint for the Vault account — demotes
# R005 (append) for that account. Auto-detected when --new-source is
# provided and the field carries #[account(mut, realloc = ...)].
ratchet check-upgrade --lock ratchet.lock --new new.json \
  --realloc-account Vault
```

### JSON output for CI

```sh
ratchet --json check-upgrade --lock ratchet.lock --new new.json \
  | jq '.findings[] | select(.severity != "additive")'
```

### List every rule

```sh
ratchet list-rules
```

### GitHub Action

A composite action is shipped from the repo root (`action.yml`). On every PR, diff the candidate IDL against a committed `ratchet.lock`:

```yaml
- uses: saicharanpogul/ratchet@main
  with:
    new: target/idl/my_program.json
    lock: ratchet.lock
```

See [`examples/github-workflow.yml`](examples/github-workflow.yml) for a complete example including Rust toolchain setup and caching. Action outputs `verdict` (safe / breaking / unsafe) and `exit-code` for downstream steps to react to.

## Rules

| ID | Name | Severity | Allow flag |
|---|---|---|---|
| R001 | account-field-reorder | BREAKING | — |
| R002 | account-field-retype | BREAKING | `allow-type-change` |
| R003 | account-field-removed | BREAKING | `allow-field-removed` or `--migrated-account` |
| R004 | account-field-insert-middle | BREAKING | `allow-field-insert` or `--migrated-account` |
| R005 | account-field-append | UNSAFE | `allow-field-append`, `--realloc-account`, or `--migrated-account` |
| R006 | account-discriminator-change | BREAKING | `allow-rename` |
| R007 | instruction-removed | BREAKING | `allow-ix-removal` |
| R008 | instruction-arg-change | BREAKING | `allow-ix-arg-change` |
| R009 | instruction-account-list-change | BREAKING | `allow-ix-account-change` |
| R010 | instruction-signer-writable-flip | BREAKING (tightening) / ADDITIVE (relaxation) | `allow-signer-mut-flip` |
| R011 | enum-variant-removed-or-inserted | BREAKING | — |
| R012 | enum-variant-append | ADDITIVE | — (informational) |
| R013 | pda-seed-change | BREAKING | `allow-pda-shape-change` (presence flip only) |
| R014 | instruction-discriminator-change | BREAKING | `allow-ix-rename` |
| R015 | account-removed | BREAKING | `allow-account-removal` |
| R016 | event-discriminator-change | BREAKING | `allow-event-rename` |

Pass an allow flag with `--unsafe <flag>` (e.g. `--unsafe allow-rename`). Declare migration coverage with `--migrated-account <Name>` or `--realloc-account <Name>` — both demote R005 appends to Additive, and `--migrated-account` also demotes R003/R004 since a declared migration can rewrite accounts to any layout.

## Example output

A vault program in which `Vault` has had its fields reordered, a new `bump` field appended, its discriminator changed, the `withdraw` instruction removed, the `deposit` argument type changed from `u64` to `u32`, and a new enum variant inserted in the middle:

```
BREAKING  R001  account-field-reorder  account:Vault
          fields reordered in account `Vault`: [owner, balance] → [balance, owner]
UNSAFE    R005  account-field-append  account:Vault/field:bump
          field `Vault.bump` (u8) appended; existing accounts lack these bytes...
          (acknowledge with --unsafe allow-field-append)
BREAKING  R006  account-discriminator-change  account:Vault/discriminator
          discriminator of account `Vault` changed: 0xd308e82b02987577 → 0x6363636363636363
BREAKING  R007  instruction-removed  ix:withdraw
          instruction `withdraw` was removed...
BREAKING  R008  instruction-arg-change  ix:deposit/args
          argument signature of `deposit` changed: (amount: u64) → (amount: u32)
BREAKING  R011  enum-variant-removed-or-inserted  type:Side/variant:Cross
          enum variant `Side::Cross` inserted before existing variants...

verdict: BREAKING — upgrade will corrupt data or break clients
```

Exit code `1`.

## Layout

```
ratchet/
├── action.yml                      # GitHub Action (composite)
├── SKILL.md                        # agent-discoverable skill definition
├── crates/                         # Rust workspace (all 8 crates publish under solana-ratchet-*)
│   ├── ratchet-core/               # framework-agnostic IR and rule engine
│   ├── ratchet-anchor/             # Anchor IDL loader, decoder, normalizer, RPC fetch, PDA derivation
│   ├── ratchet-lock/               # ratchet.lock format
│   ├── ratchet-source/             # syn-based source parser for PDA seeds + realloc constraints
│   ├── ratchet-svm/                # sample-account runtime verification + ELF header check
│   ├── ratchet-squads/             # Squads V4 vault-transaction decoder
│   ├── ratchet-quasar/             # compiler-pass entry points and SurfaceBuilder
│   └── ratchet-cli/                # the `ratchet` binary
├── web/                            # Next.js 15 frontend (landing, /diff, /rules, /skill.md)
├── examples/
│   └── github-workflow.yml
├── docs/
│   ├── publishing.md
│   └── quasar-integration.md
└── ...
```

## License

Apache-2.0. See [LICENSE](LICENSE).
