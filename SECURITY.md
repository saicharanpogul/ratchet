# Security policy

## Scope

`ratchet` is an upgrade-safety checker for Solana programs. A security
issue in ratchet is one where:

- A rule *misses* a class of breaking upgrade it claims to catch, and
  a user relying on that rule could ship a program upgrade that
  corrupts on-chain state or breaks existing clients.
- An input (IDL JSON, Squads proposal blob, on-chain account bytes,
  `ratchet.lock`) can cause denial of service, panic, or memory
  unsafety in the crates.
- The lockfile / report format is vulnerable to spoofing or replay
  in a way that would let a malicious upgrade pass CI.

Crashes from obviously malformed input that still error out cleanly
are not security issues — but a quiet mis-classification that lets
a Breaking change pass as Additive is.

## Reporting

Email **pogul1804@gmail.com** with:
- A minimal reproducer (inputs + expected vs. observed behavior).
- The ratchet git sha or published version.
- Your proposed severity.

Please give us a reasonable window before public disclosure
(typically 30 days for verified issues, longer for ones that
require upstream Anchor / Squads coordination).

## Supported versions

During the alpha period, only the latest `main` is supported. Once
tags begin, the most recent minor line plus the previous minor line
receive fixes.
