//! The [`Rule`] trait and [`CheckContext`] that every check implements.

use std::collections::HashSet;

use crate::diagnostics::{Finding, Severity};
use crate::surface::ProgramSurface;

/// A single upgrade-safety check. Rules are stateless; all per-run state
/// lives in [`CheckContext`].
pub trait Rule: Send + Sync {
    /// Stable identifier, e.g. `"R006"`. Never change once published.
    fn id(&self) -> &'static str;
    /// Kebab-case rule name, e.g. `"account-discriminator-change"`.
    fn name(&self) -> &'static str;
    /// One-line description used in `--list-rules` output.
    fn description(&self) -> &'static str;
    /// Compute findings by comparing `old` to `new`.
    fn check(
        &self,
        old: &ProgramSurface,
        new: &ProgramSurface,
        ctx: &CheckContext,
    ) -> Vec<Finding>;

    /// Convenience helper that seeds a [`Finding`] with this rule's id and
    /// name. Rule implementations typically call `self.finding(Severity::...)`
    /// and chain the builder setters.
    fn finding(&self, severity: Severity) -> Finding {
        Finding::new(severity, self.id(), self.name())
    }
}

/// Per-run context shared across all rules.
#[derive(Debug, Default, Clone)]
pub struct CheckContext {
    /// `--unsafe-*` flags enabled on this run. Flag names are the suffix
    /// after `--`, e.g. `"unsafe-allow-rename"`.
    pub allowed_unsafes: HashSet<String>,
    /// Accounts whose layout change is covered by a declared migration
    /// (e.g. `Migration<From, To>` in Anchor 1.0+). Keyed by account name.
    pub migrated_accounts: HashSet<String>,
}

impl CheckContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_allow(mut self, flag: impl Into<String>) -> Self {
        self.allowed_unsafes.insert(flag.into());
        self
    }

    pub fn with_migration(mut self, account: impl Into<String>) -> Self {
        self.migrated_accounts.insert(account.into());
        self
    }

    pub fn is_allowed(&self, flag: &str) -> bool {
        self.allowed_unsafes.contains(flag)
    }

    pub fn has_migration(&self, account: &str) -> bool {
        self.migrated_accounts.contains(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_allow_and_migration() {
        let ctx = CheckContext::new()
            .with_allow("unsafe-allow-rename")
            .with_migration("Vault");
        assert!(ctx.is_allowed("unsafe-allow-rename"));
        assert!(!ctx.is_allowed("unsafe-allow-type-change"));
        assert!(ctx.has_migration("Vault"));
        assert!(!ctx.has_migration("User"));
    }

    struct StubRule;
    impl Rule for StubRule {
        fn id(&self) -> &'static str {
            "R999"
        }
        fn name(&self) -> &'static str {
            "stub"
        }
        fn description(&self) -> &'static str {
            "test rule"
        }
        fn check(&self, _: &ProgramSurface, _: &ProgramSurface, _: &CheckContext) -> Vec<Finding> {
            vec![self.finding(Severity::Additive).message("stub")]
        }
    }

    #[test]
    fn finding_helper_stamps_rule_identity() {
        let f = StubRule.finding(Severity::Breaking);
        assert_eq!(f.rule_id, "R999");
        assert_eq!(f.rule_name, "stub");
        assert_eq!(f.severity, Severity::Breaking);
    }
}
