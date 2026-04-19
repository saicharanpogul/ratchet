//! Single-surface readiness rules. Keyed `PNNN` to stay clearly
//! separate from the `RNNN` diff rules. Each rule implements
//! [`crate::preflight::PreflightRule`] and runs against one
//! [`ProgramSurface`](crate::surface::ProgramSurface).

use crate::preflight::PreflightRule;

pub mod p001_account_missing_version_field;
pub mod p002_account_missing_reserved_padding;
pub mod p003_account_missing_discriminator_pin;
pub mod p004_event_missing_discriminator_pin;
pub mod p005_account_name_collision;
pub mod p006_instruction_missing_signer;

pub use p001_account_missing_version_field::AccountMissingVersionField;
pub use p002_account_missing_reserved_padding::AccountMissingReservedPadding;
pub use p003_account_missing_discriminator_pin::AccountMissingDiscriminatorPin;
pub use p004_event_missing_discriminator_pin::EventMissingDiscriminatorPin;
pub use p005_account_name_collision::AccountNameCollision;
pub use p006_instruction_missing_signer::InstructionMissingSigner;

pub fn all() -> Vec<Box<dyn PreflightRule>> {
    vec![
        Box::new(AccountMissingVersionField),
        Box::new(AccountMissingReservedPadding),
        Box::new(AccountMissingDiscriminatorPin),
        Box::new(EventMissingDiscriminatorPin),
        Box::new(AccountNameCollision),
        Box::new(InstructionMissingSigner),
    ]
}
