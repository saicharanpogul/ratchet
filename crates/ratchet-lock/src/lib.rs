//! Lockfile format for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! `ratchet.lock` pins the discriminators, account sizes, PDA seed hashes, and
//! instruction set of a deployed program so CI can diff source against a
//! committed baseline without a live RPC call.
