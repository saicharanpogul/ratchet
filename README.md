# ratchet

Upgrade-safety checker for Solana programs.

`ratchet` compares a new program surface against the deployed program on-chain (or a committed `ratchet.lock` baseline) and flags changes that would silently corrupt data, break clients, or orphan PDAs — before the upgrade transaction lands.

## Why

Solana program upgrades have no equivalent of `buf breaking` or `@openzeppelin/hardhat-upgrades`. Today a developer can rename an `#[account]` struct, silently change the discriminator, and orphan every account the program owns — `solana program upgrade` will happily land it. `ratchet` closes that gap.

Every diff is classified as:

| Verdict | Exit | Meaning |
|---|---|---|
| `ADDITIVE` | `0` | Backward-compatible. Existing accounts and clients keep working. |
| `UNSAFE` | `2` | Needs a declared migration or `--unsafe-*` acknowledgement. |
| `BREAKING` | `1` | Will corrupt on-chain state, break existing clients, or orphan existing PDAs. |

## Status

Alpha. Phases 0–4 shipped:

- framework-agnostic IR and rule engine
- 13 rules across accounts / instructions / enums / PDAs
- Anchor IDL adapter — file loader, RPC fetcher with auto-derived IDL account address from `--program`, on-chain account decoder
- `ratchet.lock` format for committable baselines
- `syn`-based Anchor source parser that fills in PDA seeds the IDL lost
- `ratchet replay` — samples live program accounts via RPC and flags ones that don't match the new IDL's minimum layout
- GitHub Action
- human + JSON output, CI-friendly exit codes (0 safe, 1 breaking, 2 unsafe)

Coming next: Squads proposal diff view, literal LiteSVM program deploy, Quasar compiler-pass mode.

## Install

```sh
cargo install --path crates/ratchet-cli
```

(published crate coming once the rule list stabilises)

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
# Rebuild with the feature to enable it
cargo install --path crates/ratchet-cli --features litesvm-deploy

ratchet replay --program <PID> --new target/idl/vault.json \
  --so target/deploy/vault.so --deploy
```

`--deploy` loads the `.so` into an in-process LiteSVM to confirm the runtime accepts the bytecode. The feature is opt-in because LiteSVM pulls in the Solana runtime crates; default builds stay lightweight and use the ELF-header check.

### Acknowledge an intentional change

```sh
# Demote the R006 finding for a deliberate struct rename
ratchet check-upgrade --lock ratchet.lock --new new.json \
  --unsafe allow-rename

# Declare a Migration<From, To> for the Vault account
ratchet check-upgrade --lock ratchet.lock --new new.json \
  --migrated-account Vault
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
| R003 | account-field-removed | BREAKING | — |
| R004 | account-field-insert-middle | BREAKING | — |
| R005 | account-field-append | UNSAFE | `allow-field-append` or `Migration<_,_>` |
| R006 | account-discriminator-change | BREAKING | `allow-rename` |
| R007 | instruction-removed | BREAKING | `allow-ix-removal` |
| R008 | instruction-arg-change | BREAKING | `allow-ix-arg-change` |
| R009 | instruction-account-list-change | BREAKING | `allow-ix-account-change` |
| R010 | instruction-signer-writable-flip | BREAKING | `allow-signer-mut-flip` |
| R011 | enum-variant-removed-or-inserted | BREAKING | — |
| R012 | enum-variant-append | ADDITIVE | — (informational) |
| R013 | pda-seed-change | BREAKING | — |

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
├── crates/
│   ├── ratchet-core/               # framework-agnostic IR and rule engine
│   ├── ratchet-anchor/             # Anchor IDL loader, decoder, normalizer, RPC fetch, PDA derivation
│   ├── ratchet-lock/               # ratchet.lock format
│   ├── ratchet-source/             # syn-based source parser for PDA seeds
│   ├── ratchet-svm/                # sample-account runtime verification + ELF header check
│   ├── ratchet-squads/             # Squads V4 vault-transaction decoder
│   ├── ratchet-quasar/             # compiler-pass entry points and SurfaceBuilder
│   └── ratchet-cli/                # the `ratchet` binary
├── examples/
│   └── github-workflow.yml
├── docs/
│   └── quasar-integration.md
└── ...
```

## License

Apache-2.0. See [LICENSE](LICENSE).
