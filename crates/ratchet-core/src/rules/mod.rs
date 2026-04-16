//! The built-in rule set.
//!
//! Each rule lives in its own module, is exposed as a zero-sized struct,
//! and is wired up through [`all`] which the engine uses as its default
//! registration.

use crate::rule::Rule;

pub mod r001_account_field_reorder;

pub use r001_account_field_reorder::AccountFieldReorder;

/// Every rule that ships with ratchet. Order matches the `RXXX` ids.
pub fn all() -> Vec<Box<dyn Rule>> {
    vec![Box::new(AccountFieldReorder)]
}
