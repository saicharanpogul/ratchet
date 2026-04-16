//! Anchor IDL adapter for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! Loads Anchor IDLs from local build output and from on-chain IDL accounts,
//! and normalizes them into the `ratchet-core` intermediate representation.

pub mod decode;
pub mod idl;
pub mod load;
pub mod normalize;

pub use decode::{decode_idl_account, IDL_PREFIX_LEN};
pub use idl::AnchorIdl;
pub use load::{load_idl_from_file, load_idl_from_workspace};
pub use normalize::{default_account_discriminator, default_instruction_discriminator, normalize};
