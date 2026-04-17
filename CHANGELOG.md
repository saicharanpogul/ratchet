# Changelog

All notable changes to ratchet are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and ratchet adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- 14 upgrade-safety rules across accounts, instructions, enums, and PDAs
  (R001–R014).
- `ratchet-anchor` adapter: IDL file loader, on-chain IDL account decoder,
  `fetch_idl_for_program` auto-deriving the IDL account via
  `find_program_address` + `create_with_seed` (curve25519-dalek, no
  solana-sdk dep).
- `ratchet-lock` committable baseline format with envelope-level
  `program_id` and `program_name`, plus `ensure_matches` tamper check.
- `ratchet-source` syn-based parser extracting PDA seed expressions
  from `#[derive(Accounts)]` + `#[account(seeds = […])]`.
- `ratchet-svm` ELF header verifier and optional `litesvm` feature
  that deploys the candidate `.so` into an in-process VM.
- `ratchet-squads` V4 `VaultTransaction` decoder with full `SmallVec`
  handling, extracting concrete program id + buffer on upgrade
  proposals.
- `ratchet-quasar` scaffolding: `SurfaceBuilder`, `check_pair`,
  `QuasarSchema` forward-compat envelope.
- GitHub Action (`action.yml`) with composite steps to install
  ratchet and run `check-upgrade` in CI.
- `ratchet` CLI with `check-upgrade`, `lock`, `replay`, `squads`,
  `list-rules`; human and `--json` output.
- GitHub Actions CI running `cargo fmt`, `clippy`, `test`, and `doc`
  (`-D warnings` throughout).

### Changed
- `TypeRef::Unrecognized { raw }` distinguishes unknown Anchor types
  from genuine user-defined types, so `coption<u64> → coption<u32>`
  is caught as a retype.
- R010 distinguishes signer/writable *tightening* (Breaking) from
  *relaxation* (Additive).
- R003 / R004 honor `--migrated-account` and added explicit allow
  flags (`allow-field-removed`, `allow-field-insert`).
- R013 detects PDA shape transitions (None ↔ Some).
- ELF parser no longer accepts the fabricated `EM_SBPF = 0x0107`;
  surfaces `e_flags` and `sbpf_version_hint` instead.
- Squads `VaultTransactionMessage` now parses with correct `SmallVec<u8, T>`
  prefixes; previous Borsh `Vec<T>` layout would silently fall back to
  the heuristic decoder on real on-chain data.

### Fixed
- `normalize_pda` preserves `AnchorIdlPda.program` so PDAs derived
  off other programs (Token Metadata, etc.) are diff-sensitive.
- Lockfile identity check prevents cross-program diffs from silently
  producing wrong results.

## [0.0.1] - Unreleased

Initial workspace skeleton and first-pass rule engine.
