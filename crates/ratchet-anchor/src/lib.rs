//! Anchor IDL adapter for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! Loads Anchor IDLs from local build output and from on-chain IDL accounts,
//! and normalizes them into the `ratchet-core` intermediate representation.

pub mod decode;
#[cfg(feature = "rpc")]
pub mod fetch;
pub mod idl;
pub mod load;
pub mod normalize;
pub mod pda;

pub use decode::{decode_idl_account, IDL_PREFIX_LEN};
#[cfg(feature = "rpc")]
pub use fetch::{fetch_account_data, fetch_idl_account, fetch_idl_for_program, Cluster};
pub use idl::AnchorIdl;
pub use load::{load_idl_from_file, load_idl_from_workspace};
pub use normalize::{
    default_account_discriminator, default_event_discriminator, default_instruction_discriminator,
    normalize,
};
#[cfg(feature = "rpc")]
pub use pda::{anchor_idl_address, find_program_address, is_on_curve};
pub use pda::{create_with_seed, decode_pubkey, encode_pubkey, ANCHOR_IDL_SEED};
