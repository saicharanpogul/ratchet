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

Pre-alpha. Phases 0–3 shipped: IR, 13 rules across accounts / instructions / enums / PDAs, Anchor IDL adapter, `ratchet.lock`, CLI with JSON output, CI-friendly exit codes.

Coming next: automatic IDL-account derivation from `--program`, Squads proposal diff view, LiteSVM/Surfpool runtime replay, Quasar compiler-pass mode, GitHub Action.

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
ratchet check-upgrade \
  --idl-account <IDL_ACCOUNT_PUBKEY> \
  --cluster mainnet \
  --new target/idl/vault.json
```

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
├── crates/
│   ├── ratchet-core/     # framework-agnostic IR and rule engine
│   ├── ratchet-anchor/   # Anchor IDL loader, decoder, normalizer, RPC fetch
│   ├── ratchet-lock/     # ratchet.lock format
│   └── ratchet-cli/      # the `ratchet` binary
└── ...
```

## License

Apache-2.0. See [LICENSE](LICENSE).
