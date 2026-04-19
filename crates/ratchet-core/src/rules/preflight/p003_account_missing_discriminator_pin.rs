//! P003 — account-missing-discriminator-pin.
//!
//! When an `#[account]` relies on the default `sha256("account:<Name>")[..8]`
//! discriminator, any future rename of the struct (even a cosmetic
//! one) changes the discriminator and R006 fires as a Breaking upgrade.
//! Pinning the discriminator explicitly with
//! `#[account(discriminator = &[...])]` (Anchor 0.31+) lets the
//! developer rename freely — the discriminator stays stable.
//!
//! Emitted as `Additive` (informational) — it's a code-hygiene
//! recommendation, not a correctness bug. Default discriminators
//! work today; the rule exists so agents can suggest the pinning
//! pattern when reviewing a pre-mainnet design.

use ratchet_anchor_default::default_account_discriminator;

use crate::diagnostics::{Finding, Severity};
use crate::preflight::PreflightRule;
use crate::rule::CheckContext;
use crate::surface::ProgramSurface;

pub const ID: &str = "P003";
pub const NAME: &str = "account-missing-discriminator-pin";
pub const DESCRIPTION: &str =
    "Account uses Anchor's default discriminator; pinning it explicitly survives struct renames without firing R006.";

pub struct AccountMissingDiscriminatorPin;

impl PreflightRule for AccountMissingDiscriminatorPin {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn description(&self) -> &'static str {
        DESCRIPTION
    }

    fn check(&self, surface: &ProgramSurface, _ctx: &CheckContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        for (name, account) in &surface.accounts {
            let default = default_account_discriminator(name);
            if account.discriminator != default {
                continue;
            }
            findings.push(
                self.finding(Severity::Additive)
                    .at([format!("account:{name}"), "discriminator".into()])
                    .message(format!(
                        "account `{name}` uses the default discriminator; a future rename would fire R006",
                    ))
                    .suggestion(
                        "Pin the discriminator with `#[account(discriminator = &[..])]` (Anchor 0.31+) so the 8-byte selector stays stable across struct renames.",
                    ),
            );
        }
        findings
    }
}

// Inline the Anchor-discriminator helper here rather than depending
// on ratchet-anchor. The helper is tiny (one sha2 call) and ratchet-core
// must stay Anchor-framework-agnostic per the workspace design.
mod ratchet_anchor_default {
    use sha2::{Digest, Sha256};

    pub fn default_account_discriminator(name: &str) -> [u8; 8] {
        let digest = Sha256::digest(format!("account:{name}").as_bytes());
        let mut out = [0u8; 8];
        out.copy_from_slice(&digest[..8]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountDef, ProgramSurface};

    fn surface_with(name: &str, disc: [u8; 8]) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.accounts.insert(
            name.into(),
            AccountDef {
                name: name.into(),
                discriminator: disc,
                fields: vec![],
                size: None,
            },
        );
        s
    }

    #[test]
    fn default_discriminator_is_flagged_as_additive() {
        let s = surface_with(
            "Vault",
            ratchet_anchor_default::default_account_discriminator("Vault"),
        );
        let findings = AccountMissingDiscriminatorPin.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
    }

    #[test]
    fn pinned_discriminator_is_not_flagged() {
        let s = surface_with("Vault", [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0]);
        assert!(AccountMissingDiscriminatorPin
            .check(&s, &CheckContext::new())
            .is_empty());
    }
}
