//! Anchor IDL adapter for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! Loads Anchor IDLs from local build output and from on-chain IDL accounts,
//! and normalizes them into the `ratchet-core` intermediate representation.

pub mod idl;
pub mod load;

pub use idl::AnchorIdl;
pub use load::{load_idl_from_file, load_idl_from_workspace};
