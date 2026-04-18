---
name: ratchet
description: Check Solana program upgrades for breaking schema changes before they ship. Use when a user is upgrading an Anchor or native Solana program, preparing to sign a Squads multisig upgrade proposal, or reviewing a PR that touches `programs/` or `target/idl/` on a project that has live on-chain deployments. Catches silent data-corrupting changes — renamed struct discriminators, reordered fields, changed PDA seeds, and fourteen other rule categories — that `solana program upgrade` would happily land.
homepage: https://github.com/saicharanpogul/ratchet
---

# ratchet — agent skill

ratchet is an upgrade-safety checker for Solana programs. It diffs a candidate program (local Anchor IDL) against a baseline (deployed on-chain IDL, committed lockfile, or a second IDL file) and classifies every change as **Additive** (safe), **Unsafe** (needs a declared migration), or **Breaking** (will corrupt on-chain state or break existing clients).

This skill tells an AI agent when to reach for ratchet, how to invoke it, how to read its output, and how to decide whether an upgrade is safe to ship.

## When to invoke

Trigger ratchet when any of these hold:

- User says "is it safe to upgrade", "will this break my program", "can I ship this", "review this upgrade"
- User mentions `anchor upgrade`, `anchor idl upgrade`, `solana program deploy`, `solana program write-buffer`, upgrade authority, Squads proposal
- User is editing `#[account]`, `#[derive(Accounts)]`, `#[event]` structs in a live Anchor program
- User added, removed, renamed, or reordered fields in any Anchor struct
- User changed `#[account(seeds = [...])]` on an account that might already have live PDAs
- User is reviewing a pull request that modifies `programs/**/src/*.rs` or `target/idl/*.json`
- User is about to approve a Squads V4 `VaultTransaction` that contains a BPF-loader-upgradeable `Upgrade` instruction

Do NOT use ratchet when:

- The program has never been deployed anywhere (nothing to compare against)
- The program is intentionally immutable (`solana program --final` was run and nobody will upgrade it)
- The work is purely off-chain (wallet, frontend, indexer, RPC client)

## Installation

One-time, machine-wide:

```sh
cargo install solana-ratchet-cli
```

Or with the optional in-process LiteSVM deploy smoke test:

```sh
cargo install solana-ratchet-cli --features litesvm-deploy
```

The binary is called `ratchet`. Sub-crates publish under `solana-ratchet-*` on crates.io if a project wants to depend on the library directly.

## Commands

All commands support `--json` for machine-parseable output. Exit codes: `0` safe, `1` breaking, `2` unsafe, `3` CLI error.

### `ratchet check-upgrade` — the main verb

Diff a candidate against a baseline and report findings.

```sh
# Baseline = IDL file on disk
ratchet check-upgrade --old path/to/old.json --new target/idl/my_program.json

# Baseline = committed ratchet.lock (most common in CI)
ratchet check-upgrade --lock ratchet.lock --new target/idl/my_program.json

# Baseline = on-chain IDL (auto-derives the IDL account address from program id)
ratchet check-upgrade --program <PROGRAM_ID> --cluster mainnet \
  --new target/idl/my_program.json

# Baseline = explicit IDL account pubkey (when the IDL was moved off the canonical slot)
ratchet check-upgrade --idl-account <PUBKEY> --cluster mainnet \
  --new target/idl/my_program.json
```

Optional augmentation flags:

- `--new-source <DIR>` / `--old-source <DIR>` — parse Anchor source to fill in PDA seed detail the IDL flattened away. Auto-populates `--realloc-account` when it sees `#[account(mut, realloc = ...)]`.
- `--unsafe <FLAG>` — acknowledge a specific allow-flag on a finding (repeatable).
- `--migrated-account <NAME>` — declare that the account has a `Migration<From, To>` wrapper or a custom migration instruction. Demotes R003/R004/R005 for that account.
- `--realloc-account <NAME>` — declare that an Anchor `realloc = ...` constraint exists. Demotes R005.

### `ratchet lock` — write a baseline snapshot

```sh
# From local IDL
ratchet lock --from-idl target/idl/my_program.json --out ratchet.lock

# From on-chain (requires program id or idl account)
ratchet lock --program <PROGRAM_ID> --cluster mainnet --out ratchet.lock
```

Commit the resulting `ratchet.lock` to the repo. Every subsequent PR runs `ratchet check-upgrade --lock ratchet.lock --new ...` and never needs RPC.

### `ratchet replay` — runtime verification

```sh
ratchet replay --program <PROGRAM_ID> --new target/idl/my_program.json --limit 200

# With .so binary check + optional in-process LiteSVM deploy
ratchet replay --program <PROGRAM_ID> --new target/idl/my_program.json \
  --so target/deploy/my_program.so --deploy
```

Samples live program-owned accounts via `getProgramAccounts`, classifies them by discriminator, and flags any whose data is shorter than the new IDL's minimum layout — catches "old-layout accounts never migrated" that static rules can't see.

### `ratchet squads` — decode a Squads V4 proposal

```sh
# Quick classification
ratchet squads --proposal <VAULT_TX_PUBKEY> --cluster mainnet

# Full signer experience: decode + fetch current IDL + run check-upgrade
ratchet squads --proposal <VAULT_TX_PUBKEY> \
  --auto-diff --new target/idl/my_program.json
```

`--auto-diff` extracts the proposal's target program id, fetches the current on-chain IDL, and chains into `check-upgrade` against your candidate. Exit code reflects the overall verdict.

### `ratchet list-rules` — show the rule catalog

Prints all 16 rules with one-line descriptions. Useful for writing release notes or understanding what ratchet actually checks.

## Rule catalog

| ID | Name | Severity | Allow flag |
|---|---|---|---|
| R001 | account-field-reorder | BREAKING | — (no safe override) |
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

## Interpreting output

Each finding has:

- `severity` — `additive`, `unsafe`, or `breaking`
- `rule_id` / `rule_name` — e.g. `R013 / pda-seed-change`
- `path` — where in the surface the problem is, e.g. `account:Vault/field:balance`
- `message` — human description of the change
- `old` / `new` — rendered old and new values when applicable
- `suggestion` — concrete next step
- `allow_flag` — if set, the finding can be demoted to additive via `--unsafe <flag>`

Parse `--json` output with jq:

```sh
ratchet --json check-upgrade --lock ratchet.lock --new new.json \
  | jq '.findings[] | select(.severity != "additive")'
```

## Decision tree for findings

**On BREAKING findings with no allow flag** (R001 reorder, R015 removal without flag, etc.): the upgrade will corrupt data. Two real options:

1. Revert the change. (Often the right answer — "we didn't need to reorder those fields.")
2. Deploy as a new program with a different program id, and write a migration instruction on the old program that reads each account and mints the v2 equivalent.

Do not `--unsafe` your way past these without reading the allow-flag note. Some have none deliberately.

**On BREAKING findings with an allow flag** (R006 rename, R009 account reshuffle, R014 ix rename, etc.): ask the user *whether the change is intentional and whether existing clients/data are coordinated*.

- If the rename is an oversight → revert.
- If it's deliberate AND there are no existing accounts / no existing callers (pre-launch, testnet, throwaway) → `--unsafe <flag>` to acknowledge and ship.
- If it's deliberate AND existing state matters → treat as "no allow flag" above: either revert or do a full migration.

**On UNSAFE R005 (field append) findings**: the account grew; existing accounts lack the new bytes.

- If the account has a `realloc = ...` constraint in source → `--realloc-account <Name>` (auto-populated when `--new-source` is set). Every instruction call auto-resizes, so the append is safe.
- If there's a `Migration<From, To>` wrapper (Anchor 1.0+) or a custom migration ix that every caller runs first → `--migrated-account <Name>`.
- If neither exists, the user has to write one before the upgrade is safe. `--unsafe allow-field-append` is for the rare case where the developer has confirmed no live accounts exist.

**On ADDITIVE findings** (R012 enum tail-append, R010 relaxation, etc.): nothing to do, upgrade is safe.

## Common workflows

### Set up a project for the first time

```sh
# Once you have a deployed program
ratchet lock --program <PID> --cluster mainnet --out ratchet.lock
git add ratchet.lock && git commit -m "snapshot pre-upgrade baseline"
```

Add a GitHub workflow using `saicharanpogul/ratchet@main` (see `examples/github-workflow.yml`).

### Daily PR check

The action runs automatically. Reviewing failures: the PR comment will link to the specific findings.

### Before signing a Squads proposal

```sh
ratchet squads --proposal <VAULT_TX_PUBKEY> \
  --auto-diff --new target/idl/my_program.json
```

Exit 0 → safe to sign. Exit 1 → the proposal would break things; share the report with co-signers before anyone clicks approve.

### When ratchet flags something you believe is safe

1. Re-read the finding's `suggestion` — usually points at the real fix.
2. If a `Migration<From, To>` truly exists, pass `--migrated-account <Name>`.
3. If not, write one. Don't `--unsafe` through something you haven't verified.

## Don't bypass without understanding

ratchet's default severity is conservative on purpose. Every time an AI agent acknowledges a finding with `--unsafe` on behalf of a user without confirming the user understands the on-chain consequences, real value can be at risk. When in doubt, surface the findings back to the user with the table above and ask which path they want to take.

## Links

- Repo: https://github.com/saicharanpogul/ratchet
- crates.io: https://crates.io/crates/solana-ratchet-cli
- Rule-by-rule docs: see each `crates/ratchet-core/src/rules/r*.rs` module-level comment
