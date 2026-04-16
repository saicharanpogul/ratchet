# ratchet

Upgrade-safety checker for Solana programs.

`ratchet` compares a new program source or IDL against the deployed program on-chain and flags changes that would silently corrupt data, break clients, or orphan PDAs — before the upgrade transaction lands.

## Why

Solana program upgrades have no equivalent of `buf breaking` or `@openzeppelin/hardhat-upgrades`. Today a developer can rename an `#[account]` struct, silently change the discriminator, and orphan every account the program owns — `solana program upgrade` will happily land it.

`ratchet` classifies every diff as `ADDITIVE` (safe), `BREAKING` (will corrupt data or break clients), or `UNSAFE` (requires a declared migration). Bad upgrades fail the check and CI blocks the deploy.

## Status

Pre-alpha. Phase 0 scaffolding. Not yet usable.

## Planned checks

- Account struct field reorder, type change, removal, mid-insertion
- Account struct field append with size growth (requires migration)
- Discriminator change (struct rename)
- Instruction removal, argument reordering, account-list changes, signer/writable flips
- Enum variant insertion (non-tail) or removal
- PDA seed expression change
- Program data account growth without extend

## Install

```sh
cargo install ratchet-cli   # once published
```

## Usage

```sh
ratchet check-upgrade --program <PROGRAM_ID> --cluster mainnet
```

## License

Apache-2.0. See [LICENSE](LICENSE).
