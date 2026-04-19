# Changelog

All notable changes to ratchet are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and ratchet adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] — 2026-04-19

### Added
- **R015 account-removed** — flags `#[account]` structs that disappeared
  entirely between versions; every existing on-chain account of that
  type is orphaned. Allow flag: `allow-account-removal`.
- **R016 event-discriminator-change** — parallel to R006 for events.
  Catches renamed `#[event]` logs that would silently desync off-chain
  indexers filtering by the old 8-byte selector.
- **Events in the IR** — `ProgramSurface.events` with `EventDef`
  (name + 8-byte discriminator). `ratchet-anchor` normalizes them
  from `AnchorIdlEventHeader`, defaulting missing discriminators to
  `sha256("event:<Name>")[..8]`.
- **Realloc-aware R005** — `CheckContext::realloc_accounts` and the
  `--realloc-account <NAME>` CLI flag demote field-append to Additive
  with a realloc-specific message. `ratchet-source` auto-detects
  `#[account(mut, realloc = ...)]` on `Account/AccountLoader/
  InterfaceAccount` fields (including `Box<>` wrappers) and populates
  the context when `--new-source` is provided.
- **SKILL.md** — agent-discoverable skill definition at the repo root.
  Decision tree for BREAKING/UNSAFE findings, flag reference, canonical
  install + command surface. Served at `/skill.md` on the website.
- **Next.js web frontend** (`web/`) — landing page, client diff tool,
  complete rule catalog, and a `/skill.md` route that reads the repo
  file directly. Dark Solana theme (purple `#9945ff` + green `#14f195`),
  JetBrains Mono for every on-chain byte, severity palette that matches
  the CLI output.
- **Crate rename** — all eight crates publish under the
  `solana-ratchet-*` namespace on crates.io. The binary stays `ratchet`;
  Rust imports stay `ratchet_core`/`ratchet_anchor`/etc. via package
  aliases so source code is unchanged.
- **GitHub Actions CI** — `fmt`, `clippy`, `test` (incl. `litesvm`
  feature), and `doc` jobs running with `-D warnings` on every push
  and pull request.
- **docs/publishing.md** — runbook for crate publishing order,
  rate-limit handling, tagging, and yanking.

### Changed
- R010 distinguishes signer/writable *tightening* (Breaking) from
  *relaxation* (Additive).
- R003 and R004 now honour `--migrated-account` and gained explicit
  allow flags (`allow-field-removed`, `allow-field-insert`).
- R013 detects PDA shape transitions (None ↔ Some) and diffs the
  target program id (via `PdaSpec.program_id`).
- `parse_primitive` / `parse_complex` return `TypeRef::Unrecognized`
  for anything the normalizer can't classify, preserving inner
  JSON shape so `coption<u64> → coption<u32>` is correctly flagged as
  a retype.
- ELF parser surfaces `e_flags` + `sbpf_version_hint` instead of a
  fabricated `EM_SBPF` constant; real Solana binaries all ship with
  `e_machine = EM_BPF`.
- Squads `VaultTransactionMessage` parses with correct `SmallVec<u8, T>`
  prefixes. Previous Borsh `Vec<T>` layout would silently fall back to
  the heuristic decoder on real on-chain data.
- Lockfile envelope surfaces `program_id` and `program_name`;
  `Lockfile::ensure_matches` rejects a mismatched candidate.
- `SourcePatch::apply_to` per-seed merge — source's richer
  `Account { field: Some(_) }` wins over the IDL's coarser
  `Account { field: None }` at the same seed position.

### Fixed
- `normalize_pda` preserves `AnchorIdlPda.program` so PDAs derived off
  other programs (Token Metadata, ATA, etc.) remain diff-sensitive.
- Rustdoc `-D warnings` build satisfied (bare URL in ratchet-quasar,
  cross-crate `[ProgramSurface]` intra-doc links in
  ratchet-source/ratchet-svm).
- `augment_from_source` suppresses its parse-summary stderr banner
  when `--json` is set, so machine consumers get a clean stream.

## [0.1.0] — 2026-04-18

Initial crates.io release: `ratchet-core/anchor/lock/source/quasar` at
v0.1.0 (`solana-ratchet-svm/squads/cli` rate-limited — first appear at
v0.2.0).

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

