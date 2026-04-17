//! R015 — account-removed.
//!
//! An entire `#[account]` struct disappeared between versions. Every
//! existing on-chain account of that type is now orphaned: the new
//! binary has no matching dispatch arm for its discriminator and
//! deserialization fails with `AccountDiscriminatorNotFound`.
//!
//! This is distinct from R003 (field removal inside a still-present
//! account): R003 corrupts data for existing holders; R015 leaves the
//! data readable by old clients but unreachable through the new
//! program.
//!
//! Breaking with an `allow-account-removal` escape hatch for the
//! case where the developer has confirmed no accounts of that type
//! exist on chain (e.g. a short-lived helper type that was never
//! initialised in production).

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R015";
pub const NAME: &str = "account-removed";
pub const DESCRIPTION: &str =
    "An account struct was removed from the program; existing accounts of that type are orphaned.";

pub struct AccountRemoved;

impl Rule for AccountRemoved {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn description(&self) -> &'static str {
        DESCRIPTION
    }

    fn check(
        &self,
        old: &ProgramSurface,
        new: &ProgramSurface,
        _ctx: &CheckContext,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();
        for name in old.accounts.keys() {
            if new.accounts.contains_key(name) {
                continue;
            }
            findings.push(
                self.finding(Severity::Breaking)
                    .at([format!("account:{name}")])
                    .message(format!(
                        "account struct `{name}` was removed; every existing on-chain account of this type is orphaned"
                    ))
                    .allow_flag("allow-account-removal")
                    .suggestion(
                        "Keep the account struct declared in the program even if no instruction \
                         creates new ones. Only remove once no existing account of this type \
                         has been held on mainnet.",
                    ),
            );
        }
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::AccountDef;

    fn acc(name: &str) -> AccountDef {
        AccountDef {
            name: name.into(),
            discriminator: [0; 8],
            fields: vec![],
            size: None,
        }
    }

    fn surface_with<I: IntoIterator<Item = AccountDef>>(accs: I) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        for a in accs {
            s.accounts.insert(a.name.clone(), a);
        }
        s
    }

    #[test]
    fn identical_account_set_no_finding() {
        let s = surface_with([acc("Vault"), acc("User")]);
        assert!(AccountRemoved.check(&s, &s, &CheckContext::new()).is_empty());
    }

    #[test]
    fn removed_account_is_breaking() {
        let old = surface_with([acc("Vault"), acc("User")]);
        let new = surface_with([acc("Vault")]);
        let findings = AccountRemoved.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.rule_id, ID);
        assert_eq!(f.severity, Severity::Breaking);
        assert_eq!(f.path, vec!["account:User"]);
        assert_eq!(f.allow_flag.as_deref(), Some("allow-account-removal"));
    }

    #[test]
    fn adding_accounts_is_not_removal() {
        let old = surface_with([acc("Vault")]);
        let new = surface_with([acc("Vault"), acc("User")]);
        assert!(AccountRemoved
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn multiple_removals_each_emit_a_finding() {
        let old = surface_with([acc("Vault"), acc("User"), acc("Meta")]);
        let new = surface_with([acc("Vault")]);
        let findings = AccountRemoved.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 2);
    }
}
