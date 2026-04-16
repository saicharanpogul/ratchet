//! The built-in rule set.
//!
//! Each rule lives in its own module, is exposed as a zero-sized struct,
//! and is wired up through [`all`] which the engine uses as its default
//! registration.

use crate::rule::Rule;

pub mod r001_account_field_reorder;
pub mod r002_account_field_retype;
pub mod r003_account_field_removed;
pub mod r004_account_field_insert_middle;
pub mod r005_account_field_append;
pub mod r006_account_discriminator_change;
pub mod r007_instruction_removed;
pub mod r008_instruction_arg_change;
pub mod r009_instruction_account_list_change;
pub mod r010_instruction_signer_writable_flip;
pub mod r011_enum_variant_removed_or_inserted;

pub use r001_account_field_reorder::AccountFieldReorder;
pub use r002_account_field_retype::AccountFieldRetype;
pub use r003_account_field_removed::AccountFieldRemoved;
pub use r004_account_field_insert_middle::AccountFieldInsertMiddle;
pub use r005_account_field_append::AccountFieldAppend;
pub use r006_account_discriminator_change::AccountDiscriminatorChange;
pub use r007_instruction_removed::InstructionRemoved;
pub use r008_instruction_arg_change::InstructionArgChange;
pub use r009_instruction_account_list_change::InstructionAccountListChange;
pub use r010_instruction_signer_writable_flip::InstructionSignerWritableFlip;
pub use r011_enum_variant_removed_or_inserted::EnumVariantRemovedOrInserted;

/// Every rule that ships with ratchet. Order matches the `RXXX` ids.
pub fn all() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(AccountFieldReorder),
        Box::new(AccountFieldRetype),
        Box::new(AccountFieldRemoved),
        Box::new(AccountFieldInsertMiddle),
        Box::new(AccountFieldAppend),
        Box::new(AccountDiscriminatorChange),
        Box::new(InstructionRemoved),
        Box::new(InstructionArgChange),
        Box::new(InstructionAccountListChange),
        Box::new(InstructionSignerWritableFlip),
        Box::new(EnumVariantRemovedOrInserted),
    ]
}
